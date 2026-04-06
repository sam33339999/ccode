use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

mod adapters;
mod agent_bridge;

#[derive(Debug, Parser)]
#[command(name = "ccode-gateway", version, about = "ccode gateway daemon")]
struct Cli {
    #[arg(long)]
    port: Option<u16>,
    #[arg(long)]
    workdir: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    let cfg = ccode_config::load()?;

    let port = cli
        .port
        .or_else(|| cfg.gateway.as_ref().and_then(|gateway| gateway.port))
        .unwrap_or(7001);

    let workdir_override = cli
        .workdir
        .or_else(|| {
            cfg.gateway
                .as_ref()
                .and_then(|gateway| gateway.workdir.clone())
        })
        .map(PathBuf::from);

    let state = ccode_bootstrap::wire_from_config_with_cwd(workdir_override)?;
    server::start(state, port, cfg.gateway).await
}

mod server {
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

    pub async fn start(
        state: AppState,
        port: u16,
        gateway_cfg: Option<GatewayConfig>,
    ) -> Result<()> {
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

        let app = Router::new()
            .route("/healthz", get(healthz))
            .route("/webhook/telegram", post(adapters::telegram::handle))
            .route("/webhook/discord", post(adapters::discord::handle))
            .with_state(shared_state);

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!("gateway listening on :{}", port);

        axum::serve(listener, app).await?;
        Ok(())
    }

    async fn healthz() -> &'static str {
        "ok"
    }
}
