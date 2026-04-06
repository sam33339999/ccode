use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

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
    use anyhow::Result;
    use ccode_bootstrap::AppState;
    use ccode_config::schema::GatewayConfig;

    pub async fn start(
        _state: AppState,
        _port: u16,
        _gateway_cfg: Option<GatewayConfig>,
    ) -> Result<()> {
        Ok(())
    }
}
