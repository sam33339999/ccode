pub mod agent;
pub mod cron;
pub mod health;
pub mod output;
pub mod repl;
pub mod sessions;
pub mod tui;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum Commands {
    /// Check service health
    Health,
    /// Manage sessions
    Sessions {
        #[command(subcommand)]
        action: sessions::Action,
    },
    /// Run a single agent turn
    Agent(agent::AgentArgs),
    /// Start an interactive chat session (REPL)
    Repl(repl::ReplArgs),
    /// Start the full-screen terminal UI (TUI)
    Tui(tui::TuiArgs),
    /// Manage scheduled agent jobs
    Cron {
        #[command(subcommand)]
        action: cron::Action,
    },
}

pub async fn run(cmd: Commands) -> anyhow::Result<()> {
    match cmd {
        Commands::Health => health::run().await,
        Commands::Sessions { action } => sessions::run(action).await,
        Commands::Agent(args) => agent::run(args).await,
        Commands::Repl(args) => repl::run(args).await,
        Commands::Tui(args) => tui::run(args).await,
        Commands::Cron { action } => cron::run(action).await,
    }
}
