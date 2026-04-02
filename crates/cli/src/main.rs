mod cmd;

use clap::Parser;
use cmd::output::{classify_error, error_category_label};

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
        let message = e.to_string();
        let category = error_category_label(classify_error(&message));
        eprintln!("[error:{category}] {message}");
        std::process::exit(1);
    }
}
