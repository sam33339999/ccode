use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::agent_bridge;
use crate::server::GatewayState;

const DISCORD_SIGNATURE_HEADER: &str = "X-Signature-Ed25519";
const DISCORD_TIMESTAMP_HEADER: &str = "X-Signature-Timestamp";

#[derive(Debug, Deserialize)]
pub struct DiscordInteraction {
    #[serde(rename = "type")]
    pub r#type: u8,
    pub data: Option<DiscordInteractionData>,
    pub token: String,
    pub channel_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DiscordInteractionData {
    pub name: Option<String>,
    pub options: Option<Vec<DiscordInteractionOption>>,
}

#[derive(Debug, Deserialize)]
pub struct DiscordInteractionOption {
    pub value: Option<Value>,
}

pub async fn handle(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(discord_cfg) = state.discord.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };

    if !verify_signature(&headers, &body, &discord_cfg.application_public_key) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let interaction: DiscordInteraction = match serde_json::from_slice(&body) {
        Ok(interaction) => interaction,
        Err(err) => {
            tracing::warn!(error = ?err, "invalid discord interaction payload");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    match interaction.r#type {
        1 => Json(json!({ "type": 1 })).into_response(),
        2 => {
            let Some(text) = interaction_command_text(&interaction) else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            let session_id = interaction
                .channel_id
                .clone()
                .or(Some(interaction.token.clone()));

            let agent_reply =
                match agent_bridge::run_agent(&state.app_state, text, Vec::new(), session_id).await
                {
                    Ok(reply) => reply,
                    Err(err) => {
                        tracing::error!(error = ?err, "discord run_agent failed");
                        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                };

            Json(json!({
                "type": 4,
                "data": {
                    "content": agent_reply
                }
            }))
            .into_response()
        }
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

fn verify_signature(headers: &HeaderMap, body: &[u8], application_public_key: &str) -> bool {
    let Some(signature_hex) = header_value(headers, DISCORD_SIGNATURE_HEADER) else {
        return false;
    };
    let Some(timestamp) = header_value(headers, DISCORD_TIMESTAMP_HEADER) else {
        return false;
    };

    let signature_bytes = match hex::decode(signature_hex) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    let signature = match Signature::from_slice(&signature_bytes) {
        Ok(signature) => signature,
        Err(_) => return false,
    };

    let public_key_bytes = match hex::decode(application_public_key) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    let public_key_array: [u8; 32] = match public_key_bytes.try_into() {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    let verifying_key = match VerifyingKey::from_bytes(&public_key_array) {
        Ok(key) => key,
        Err(_) => return false,
    };

    let mut message = timestamp.as_bytes().to_vec();
    message.extend_from_slice(body);

    verifying_key.verify(&message, &signature).is_ok()
}

fn header_value<'a>(headers: &'a HeaderMap, key: &str) -> Option<&'a str> {
    headers.get(key)?.to_str().ok()
}

fn interaction_command_text(interaction: &DiscordInteraction) -> Option<String> {
    let data = interaction.data.as_ref()?;

    if let Some(options) = data.options.as_ref() {
        for option in options {
            if let Some(value) = option.value.as_ref().and_then(Value::as_str) {
                return Some(value.to_string());
            }
        }
    }

    data.name.clone()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::{Bytes, to_bytes};
    use axum::extract::State;
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use axum::response::IntoResponse;
    use ccode_bootstrap::wire_dev;
    use ccode_config::schema::{DiscordConfig, ImageConfig};
    use ed25519_dalek::{Signer, SigningKey};

    use crate::server::GatewayState;

    use super::{DISCORD_SIGNATURE_HEADER, DISCORD_TIMESTAMP_HEADER, handle, verify_signature};

    #[test]
    fn verify_signature_accepts_valid_signature() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());

        let timestamp = "1712428800";
        let body = br#"{"type":1,"token":"abc"}"#;
        let mut message = timestamp.as_bytes().to_vec();
        message.extend_from_slice(body);

        let signature = signing_key.sign(&message);

        let mut headers = HeaderMap::new();
        headers.insert(
            DISCORD_SIGNATURE_HEADER,
            HeaderValue::from_str(&hex::encode(signature.to_bytes())).unwrap(),
        );
        headers.insert(
            DISCORD_TIMESTAMP_HEADER,
            HeaderValue::from_str(timestamp).unwrap(),
        );

        assert!(verify_signature(&headers, body, &public_key_hex));
    }

    #[test]
    fn verify_signature_rejects_invalid_signature() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());

        let timestamp = "1712428800";
        let body = br#"{"type":1,"token":"abc"}"#;

        let mut headers = HeaderMap::new();
        headers.insert(
            DISCORD_SIGNATURE_HEADER,
            HeaderValue::from_static(
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            ),
        );
        headers.insert(
            DISCORD_TIMESTAMP_HEADER,
            HeaderValue::from_str(timestamp).unwrap(),
        );

        assert!(!verify_signature(&headers, body, &public_key_hex));
    }

    #[tokio::test]
    async fn handle_ping_returns_pong() {
        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());

        let body = br#"{"type":1,"token":"abc","channel_id":"42"}"#;
        let timestamp = "1712428800";
        let mut message = timestamp.as_bytes().to_vec();
        message.extend_from_slice(body);
        let signature = signing_key.sign(&message);

        let mut headers = HeaderMap::new();
        headers.insert(
            DISCORD_SIGNATURE_HEADER,
            HeaderValue::from_str(&hex::encode(signature.to_bytes())).unwrap(),
        );
        headers.insert(
            DISCORD_TIMESTAMP_HEADER,
            HeaderValue::from_str(timestamp).unwrap(),
        );

        let state = GatewayState {
            app_state: Arc::new(wire_dev()),
            telegram: None,
            discord: Some(DiscordConfig {
                application_public_key: public_key_hex,
                bot_token: None,
            }),
            image: ImageConfig::default(),
            http_client: reqwest::Client::new(),
        };

        let response = handle(State(state), headers, Bytes::from_static(body))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(parsed, serde_json::json!({"type": 1}));
    }
}
