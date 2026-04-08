use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::adapters::telegram_image;
use crate::agent_bridge;
use crate::server::GatewayState;

/// Resolve a Telegram message: if it starts with `/skill-name`, load skill body;
/// otherwise return the original text. Returns `None` if skill not found.
fn resolve_skill_text(text: &str, state: &GatewayState) -> Option<String> {
    let Some(skill_token) = text.strip_prefix('/') else {
        return Some(text.to_string());
    };
    let skill_name = skill_token.trim();
    if skill_name.is_empty() {
        return Some(text.to_string());
    }
    ccode_bootstrap::skill::load_skill_body(skill_name, &state.app_state.skills)
}

const TELEGRAM_SECRET_HEADER: &str = "X-Telegram-Bot-Api-Secret-Token";

#[derive(Debug, Deserialize)]
pub struct TelegramUpdate {
    pub message: Option<TelegramMessage>,
    #[serde(flatten)]
    pub _extra: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramMessage {
    pub text: Option<String>,
    pub caption: Option<String>,
    pub photo: Option<Vec<TelegramPhotoSize>>,
    pub document: Option<TelegramDocument>,
    pub chat: TelegramChat,
    #[serde(flatten)]
    pub _extra: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramPhotoSize {
    pub file_id: String,
    pub width: u32,
    pub height: u32,
    pub file_size: Option<u64>,
    #[serde(flatten)]
    pub _extra: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramDocument {
    pub file_id: String,
    pub mime_type: Option<String>,
    pub file_name: Option<String>,
    #[serde(flatten)]
    pub _extra: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramChat {
    pub id: i64,
    #[serde(flatten)]
    pub _extra: Map<String, Value>,
}

impl TelegramMessage {
    fn effective_text(&self) -> String {
        self.text
            .as_deref()
            .or(self.caption.as_deref())
            .unwrap_or_default()
            .to_string()
    }

    fn primary_image_candidate(&self) -> Option<(&str, Option<ccode_domain::llm::ImageMediaType>)> {
        if let Some(largest) = self.photo.as_ref().and_then(|photos| {
            photos.iter().max_by_key(|photo| {
                (
                    u64::from(photo.width) * u64::from(photo.height),
                    photo.file_size.unwrap_or_default(),
                )
            })
        }) {
            return Some((largest.file_id.as_str(), None));
        }

        let document = self.document.as_ref()?;
        let media_type = telegram_image::media_type_from_mime_or_name(
            document.mime_type.as_deref(),
            document.file_name.as_deref(),
        )?;
        Some((document.file_id.as_str(), Some(media_type)))
    }
}

pub async fn handle(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(update): Json<TelegramUpdate>,
) -> impl IntoResponse {
    let Some(telegram_cfg) = state.telegram.as_ref() else {
        return StatusCode::NOT_FOUND;
    };

    if !is_webhook_secret_valid(&headers, telegram_cfg.webhook_secret.as_deref()) {
        return StatusCode::UNAUTHORIZED;
    }

    let Some(message) = update.message else {
        return StatusCode::OK;
    };

    let text = message.effective_text();
    let images = if let Some((file_id, media_type)) = message.primary_image_candidate() {
        match telegram_image::download_and_process_image(
            &state.http_client,
            telegram_cfg.bot_token.as_str(),
            file_id,
            media_type,
            &state.image,
        )
        .await
        {
            Ok(image) => vec![image],
            Err(err) => {
                tracing::error!(error = ?err, "telegram image fetch/process failed");
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
        }
    } else {
        Vec::new()
    };

    if text.is_empty() && images.is_empty() {
        return StatusCode::OK;
    }

    let chat_id = message.chat.id;

    let text = match resolve_skill_text(&text, &state) {
        Some(resolved) => resolved,
        None => {
            tracing::debug!(skill_name = %&text[1..], "skill not found, ignoring message");
            return StatusCode::OK;
        }
    };

    let has_images = !images.is_empty();
    let agent_reply = match agent_bridge::run_agent(&state.app_state, text, images, None).await {
        Ok(reply) => reply,
        Err(err) => {
            if has_images && agent_bridge::is_vision_not_supported(&err) {
                return send_telegram_text(
                    &state.http_client,
                    telegram_cfg.bot_token.as_str(),
                    chat_id,
                    "此 provider 不支援圖片輸入",
                )
                .await;
            }
            tracing::error!(error = ?err, "telegram run_agent failed");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    send_telegram_text(
        &state.http_client,
        telegram_cfg.bot_token.as_str(),
        chat_id,
        &agent_reply,
    )
    .await
}

async fn send_telegram_text(
    http_client: &reqwest::Client,
    bot_token: &str,
    chat_id: i64,
    text: &str,
) -> StatusCode {
    let send_message_url = format!("https://api.telegram.org/bot{bot_token}/sendMessage");
    let payload = json!({
        "chat_id": chat_id,
        "text": text,
    });

    match http_client
        .post(send_message_url)
        .json(&payload)
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => StatusCode::OK,
        Ok(response) => {
            tracing::error!(status = %response.status(), "telegram sendMessage failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
        Err(err) => {
            tracing::error!(error = ?err, "telegram sendMessage request failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

fn is_webhook_secret_valid(headers: &HeaderMap, expected_secret: Option<&str>) -> bool {
    let Some(expected_secret) = expected_secret else {
        return true;
    };

    let Some(actual) = headers.get(TELEGRAM_SECRET_HEADER) else {
        return false;
    };

    match actual.to_str() {
        Ok(actual) => actual == expected_secret,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};
    use serde_json::{Map, Value};

    use super::{
        TelegramChat, TelegramDocument, TelegramMessage, TelegramPhotoSize, is_webhook_secret_valid,
    };

    #[test]
    fn webhook_secret_not_required_when_unset() {
        let headers = HeaderMap::new();
        assert!(is_webhook_secret_valid(&headers, None));
    }

    #[test]
    fn webhook_secret_rejects_missing_header() {
        let headers = HeaderMap::new();
        assert!(!is_webhook_secret_valid(&headers, Some("secret")));
    }

    #[test]
    fn webhook_secret_accepts_exact_match() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Telegram-Bot-Api-Secret-Token",
            HeaderValue::from_static("secret"),
        );

        assert!(is_webhook_secret_valid(&headers, Some("secret")));
    }

    #[test]
    fn webhook_secret_rejects_non_matching_value() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Telegram-Bot-Api-Secret-Token",
            HeaderValue::from_static("wrong"),
        );

        assert!(!is_webhook_secret_valid(&headers, Some("secret")));
    }

    #[test]
    fn primary_image_file_id_prefers_largest_photo_size() {
        let message = TelegramMessage {
            text: Some("hello".to_string()),
            caption: None,
            photo: Some(vec![
                TelegramPhotoSize {
                    file_id: "small".to_string(),
                    width: 100,
                    height: 100,
                    file_size: Some(10),
                    _extra: Map::<String, Value>::new(),
                },
                TelegramPhotoSize {
                    file_id: "large".to_string(),
                    width: 600,
                    height: 400,
                    file_size: Some(50),
                    _extra: Map::<String, Value>::new(),
                },
            ]),
            document: None,
            chat: TelegramChat {
                id: 42,
                _extra: Map::<String, Value>::new(),
            },
            _extra: Map::<String, Value>::new(),
        };

        let (file_id, media_type) = message.primary_image_candidate().expect("has image");
        assert_eq!(file_id, "large");
        assert_eq!(media_type, None);
    }

    #[test]
    fn primary_image_file_id_uses_image_document_when_no_photo() {
        let message = TelegramMessage {
            text: None,
            caption: Some("describe this".to_string()),
            photo: None,
            document: Some(TelegramDocument {
                file_id: "doc-image".to_string(),
                mime_type: Some("image/png".to_string()),
                file_name: Some("input.png".to_string()),
                _extra: Map::<String, Value>::new(),
            }),
            chat: TelegramChat {
                id: 42,
                _extra: Map::<String, Value>::new(),
            },
            _extra: Map::<String, Value>::new(),
        };

        let (file_id, media_type) = message.primary_image_candidate().expect("has image");
        assert_eq!(file_id, "doc-image");
        assert_eq!(media_type, Some(ccode_domain::llm::ImageMediaType::Png));
        assert_eq!(message.effective_text(), "describe this");
    }

    #[test]
    fn non_image_document_is_ignored() {
        let message = TelegramMessage {
            text: Some("ping".to_string()),
            caption: None,
            photo: None,
            document: Some(TelegramDocument {
                file_id: "doc-pdf".to_string(),
                mime_type: Some("application/pdf".to_string()),
                file_name: Some("report.pdf".to_string()),
                _extra: Map::<String, Value>::new(),
            }),
            chat: TelegramChat {
                id: 42,
                _extra: Map::<String, Value>::new(),
            },
            _extra: Map::<String, Value>::new(),
        };

        assert_eq!(message.primary_image_candidate(), None);
        assert_eq!(message.effective_text(), "ping");
    }
}
