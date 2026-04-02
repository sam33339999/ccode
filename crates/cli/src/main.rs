mod cmd;

use clap::Parser;

#[derive(Parser)]
#[command(name = "ccode", version, about = "ccode — AI agent CLI")]
struct Cli {
    #[command(subcommand)]
    command: cmd::Commands,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let cli = Cli::parse();
    if let Err(e) = cmd::run(cli.command).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
