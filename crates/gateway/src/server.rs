use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::routing::post;
use axum::{Router, routing::get};
use ccode_bootstrap::AppState;
use ccode_config::schema::{DiscordConfig, GatewayConfig, TelegramConfig};

use crate::adapters;

#[derive(Clone)]
pub struct GatewayState {
    pub app_state: Arc<AppState>,
    pub telegram: Option<TelegramConfig>,
    pub discord: Option<DiscordConfig>,
    pub http_client: reqwest::Client,
}

pub async fn start(state: AppState, port: u16, gateway_cfg: Option<GatewayConfig>) -> Result<()> {
    let (telegram_cfg, discord_cfg) = match gateway_cfg {
        Some(cfg) => (cfg.telegram, cfg.discord),
        None => (None, None),
    };

    let shared_state = GatewayState {
        app_state: Arc::new(state),
        telegram: telegram_cfg,
        discord: discord_cfg,
        http_client: reqwest::Client::new(),
    };

    let app = build_router(shared_state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("gateway listening on :{}", port);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn build_router(state: GatewayState) -> Router {
    let telegram_enabled = state.telegram.is_some();
    let discord_enabled = state.discord.is_some();

    let mut app = Router::new().route("/health", get(health));
    if telegram_enabled {
        app = app.route("/webhook/telegram", post(adapters::telegram::handle));
    }
    if discord_enabled {
        app = app.route("/webhook/discord", post(adapters::discord::handle));
    }

    app.with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = ?err, "failed to install ctrl_c signal handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};

        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(err) => {
                tracing::warn!(error = ?err, "failed to install SIGTERM signal handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use ccode_bootstrap::wire_dev;
    use ccode_config::schema::{DiscordConfig, TelegramConfig};
    use tower::util::ServiceExt;

    use crate::server::{GatewayState, build_router};

    fn gateway_state(
        telegram: Option<TelegramConfig>,
        discord: Option<DiscordConfig>,
    ) -> GatewayState {
        GatewayState {
            app_state: Arc::new(wire_dev()),
            telegram,
            discord,
            http_client: reqwest::Client::new(),
        }
    }

    #[tokio::test]
    async fn health_endpoint_returns_ok_body() {
        let app = build_router(gateway_state(None, None));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .method("GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn telegram_endpoint_is_404_when_config_missing() {
        let app = build_router(gateway_state(None, None));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/webhook/telegram")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn discord_endpoint_is_404_when_config_missing() {
        let app = build_router(gateway_state(None, None));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/webhook/discord")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn telegram_endpoint_is_enabled_when_config_present() {
        let app = build_router(gateway_state(
            Some(TelegramConfig {
                bot_token: "bot-token".to_string(),
                webhook_secret: None,
            }),
            None,
        ));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/webhook/telegram")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn discord_endpoint_is_enabled_when_config_present() {
        let app = build_router(gateway_state(
            None,
            Some(DiscordConfig {
                application_public_key: "invalid-public-key".to_string(),
                bot_token: None,
            }),
        ));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/webhook/discord")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(response.status(), StatusCode::NOT_FOUND);
    }
}
