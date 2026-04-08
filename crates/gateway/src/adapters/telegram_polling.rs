use std::sync::Arc;
use std::time::Duration;

use ccode_bootstrap::AppState;
use ccode_config::schema::{ImageConfig, TelegramConfig};
use ccode_domain::llm::ImageMediaType;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::watch;

use crate::adapters::telegram_image;

const POLL_TIMEOUT_SECS: u64 = 30;
const ERROR_RETRY_SECS: u64 = 5;

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    pub text: Option<String>,
    pub caption: Option<String>,
    pub photo: Option<Vec<TelegramPhotoSize>>,
    pub document: Option<TelegramDocument>,
    pub chat: TelegramChat,
}

#[derive(Debug, Deserialize)]
struct TelegramPhotoSize {
    pub file_id: String,
    pub width: u32,
    pub height: u32,
    pub file_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TelegramDocument {
    pub file_id: String,
    pub mime_type: Option<String>,
    pub file_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    pub id: i64,
}

impl TelegramMessage {
    fn effective_text(&self) -> String {
        self.text
            .as_deref()
            .or(self.caption.as_deref())
            .unwrap_or_default()
            .to_string()
    }

    fn primary_image_candidate(&self) -> Option<(&str, Option<ImageMediaType>)> {
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

pub async fn run(
    cfg: TelegramConfig,
    image_cfg: ImageConfig,
    state: Arc<AppState>,
    http_client: reqwest::Client,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut offset: i64 = 0;
    let bot_token = cfg.bot_token.clone();
    let base_url = format!("https://api.telegram.org/bot{}", cfg.bot_token);

    tracing::info!("telegram long polling started");

    loop {
        if *shutdown.borrow() {
            tracing::info!("telegram long polling stopped");
            return;
        }

        let updates = tokio::select! {
            result = fetch_updates(&http_client, &base_url, offset) => result,
            _ = shutdown.changed() => {
                tracing::info!("telegram long polling stopped");
                return;
            }
        };

        match updates {
            Ok(updates) => {
                for update in updates {
                    offset = update.update_id + 1;
                    if let Some(msg) = update.message {
                        process_message(
                            &state,
                            &http_client,
                            &bot_token,
                            &base_url,
                            &image_cfg,
                            msg,
                        )
                        .await;
                    }
                }
            }
            Err(err) => {
                tracing::error!(error = ?err, "telegram getUpdates failed, retrying in {ERROR_RETRY_SECS}s");
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(ERROR_RETRY_SECS)) => {}
                    _ = shutdown.changed() => return,
                }
            }
        }
    }
}

async fn fetch_updates(
    http_client: &reqwest::Client,
    base_url: &str,
    offset: i64,
) -> anyhow::Result<Vec<TelegramUpdate>> {
    let bytes = http_client
        .get(format!("{base_url}/getUpdates"))
        .query(&[
            ("timeout", POLL_TIMEOUT_SECS.to_string()),
            ("offset", offset.to_string()),
        ])
        .timeout(Duration::from_secs(POLL_TIMEOUT_SECS + 10))
        .send()
        .await?
        .bytes()
        .await?;

    #[derive(serde::Deserialize)]
    struct TelegramResponse {
        ok: bool,
        description: Option<String>,
        #[serde(default)]
        result: Vec<TelegramUpdate>,
    }

    let response: TelegramResponse = serde_json::from_slice(&bytes)?;

    if !response.ok {
        let desc = response
            .description
            .unwrap_or_else(|| "no description".to_string());
        return Err(anyhow::anyhow!("telegram API error: {desc}"));
    }

    Ok(response.result)
}

async fn process_message(
    state: &Arc<AppState>,
    http_client: &reqwest::Client,
    bot_token: &str,
    base_url: &str,
    image_cfg: &ImageConfig,
    message: TelegramMessage,
) {
    let chat_id = message.chat.id;
    let text = message.effective_text();
    let images = if let Some((file_id, media_type)) = message.primary_image_candidate() {
        match telegram_image::download_and_process_image(
            http_client,
            bot_token,
            file_id,
            media_type,
            image_cfg,
        )
        .await
        {
            Ok(image) => vec![image],
            Err(err) => {
                tracing::error!(error = ?err, chat_id, "telegram image fetch/process failed");
                return;
            }
        }
    } else {
        Vec::new()
    };

    if text.is_empty() && images.is_empty() {
        return;
    }

    // Resolve /skill-name activations: if text starts with '/', look up skill body.
    // Unknown skill names are silently dropped (not forwarded to the agent).
    let text = if let Some(skill_token) = text.strip_prefix('/') {
        let skill_name = skill_token.trim();
        match ccode_bootstrap::skill::load_skill_body(skill_name, &state.skills) {
            Some(body) => body,
            None => {
                tracing::debug!(skill_name, "skill not found, dropping message");
                return;
            }
        }
    } else {
        text
    };

    let has_images = !images.is_empty();
    let reply = match crate::agent_bridge::run_agent(state, text, images, None).await {
        Ok(r) => r,
        Err(err) => {
            if has_images && crate::agent_bridge::is_vision_not_supported(&err) {
                send_message(http_client, base_url, chat_id, "此 provider 不支援圖片輸入").await;
                return;
            }
            tracing::error!(error = ?err, chat_id, "run_agent failed");
            return;
        }
    };

    send_message(http_client, base_url, chat_id, &reply).await;
}

async fn send_message(http_client: &reqwest::Client, base_url: &str, chat_id: i64, text: &str) {
    let payload = json!({ "chat_id": chat_id, "text": text });
    if let Err(err) = http_client
        .post(format!("{base_url}/sendMessage"))
        .json(&payload)
        .send()
        .await
    {
        tracing::error!(error = ?err, chat_id, "telegram sendMessage failed");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ccode_bootstrap::wire_dev;
    use ccode_config::schema::{ImageConfig, TelegramConfig};
    use ccode_domain::llm::ImageMediaType;
    use tokio::sync::watch;

    use super::{TelegramDocument, TelegramMessage, TelegramPhotoSize};

    #[tokio::test]
    async fn polling_loop_stops_on_shutdown_signal() {
        let (tx, rx) = watch::channel(true); // already shutdown
        drop(tx);

        let cfg = TelegramConfig {
            bot_token: "fake".to_string(),
            mode: Some("long_polling".to_string()),
            webhook_secret: None,
        };
        let state = Arc::new(wire_dev());
        let client = reqwest::Client::new();

        // Should return immediately because shutdown = true
        super::run(cfg, ImageConfig::default(), state, client, rx).await;
    }

    #[test]
    fn primary_image_candidate_prefers_largest_photo() {
        let message = TelegramMessage {
            text: Some("hello".to_string()),
            caption: None,
            photo: Some(vec![
                TelegramPhotoSize {
                    file_id: "small".to_string(),
                    width: 100,
                    height: 100,
                    file_size: Some(10),
                },
                TelegramPhotoSize {
                    file_id: "large".to_string(),
                    width: 500,
                    height: 300,
                    file_size: Some(40),
                },
            ]),
            document: None,
            chat: super::TelegramChat { id: 7 },
        };

        let (file_id, media_type) = message.primary_image_candidate().expect("has image");
        assert_eq!(file_id, "large");
        assert_eq!(media_type, None);
    }

    #[test]
    fn primary_image_candidate_uses_only_image_document() {
        let image_doc_message = TelegramMessage {
            text: None,
            caption: Some("inspect".to_string()),
            photo: None,
            document: Some(TelegramDocument {
                file_id: "image-doc".to_string(),
                mime_type: Some("image/webp".to_string()),
                file_name: Some("image.webp".to_string()),
            }),
            chat: super::TelegramChat { id: 7 },
        };
        let (file_id, media_type) = image_doc_message
            .primary_image_candidate()
            .expect("has image");
        assert_eq!(file_id, "image-doc");
        assert_eq!(media_type, Some(ImageMediaType::Webp));

        let non_image_doc_message = TelegramMessage {
            text: Some("hello".to_string()),
            caption: None,
            photo: None,
            document: Some(TelegramDocument {
                file_id: "pdf-doc".to_string(),
                mime_type: Some("application/pdf".to_string()),
                file_name: Some("report.pdf".to_string()),
            }),
            chat: super::TelegramChat { id: 7 },
        };
        assert!(non_image_doc_message.primary_image_candidate().is_none());
    }

    #[test]
    fn effective_text_uses_text_then_caption_then_empty() {
        let from_text = TelegramMessage {
            text: Some("t".to_string()),
            caption: Some("c".to_string()),
            photo: None,
            document: None,
            chat: super::TelegramChat { id: 1 },
        };
        assert_eq!(from_text.effective_text(), "t");

        let from_caption = TelegramMessage {
            text: None,
            caption: Some("c".to_string()),
            photo: None,
            document: None,
            chat: super::TelegramChat { id: 1 },
        };
        assert_eq!(from_caption.effective_text(), "c");

        let empty = TelegramMessage {
            text: None,
            caption: None,
            photo: None,
            document: None,
            chat: super::TelegramChat { id: 1 },
        };
        assert_eq!(empty.effective_text(), "");
    }
}
