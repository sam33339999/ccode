mod cmd;

use clap::Parser;
use cmd::output::{ErrorContext, render_error};

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
        eprintln!("{}", render_error(&e, &ErrorContext::unknown()));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn parses_tui_subcommand() {
        assert!(
            Cli::try_parse_from(["ccode", "tui"]).is_ok(),
            "`ccode tui` should parse"
        );
    }
}
