use std::sync::Arc;
use std::time::Duration;

use ccode_bootstrap::AppState;
use ccode_config::schema::TelegramConfig;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::watch;

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
    pub chat: TelegramChat,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    pub id: i64,
}

pub async fn run(
    cfg: TelegramConfig,
    state: Arc<AppState>,
    http_client: reqwest::Client,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut offset: i64 = 0;
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
                    if let Some(msg) = update.message
                        && let Some(text) = msg.text
                    {
                        process_message(&state, &http_client, &base_url, msg.chat.id, text).await;
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
    base_url: &str,
    chat_id: i64,
    text: String,
) {
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

    let reply = match crate::agent_bridge::run_agent(state, text, None).await {
        Ok(r) => r,
        Err(err) => {
            tracing::error!(error = ?err, chat_id, "run_agent failed");
            return;
        }
    };

    let payload = json!({ "chat_id": chat_id, "text": reply });
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
    use ccode_config::schema::TelegramConfig;
    use tokio::sync::watch;

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
        super::run(cfg, state, client, rx).await;
    }
}
