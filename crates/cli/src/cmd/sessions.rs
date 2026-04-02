use ccode_application::queries::sessions_list::SessionsListQuery;
use ccode_bootstrap::wire_dev;
use clap::Subcommand;
use std::sync::Arc;

#[derive(Subcommand)]
pub enum Action {
    /// List recent sessions
    List {
        /// Maximum number of sessions to show
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

pub async fn run(action: Action) -> anyhow::Result<()> {
    match action {
        Action::List { limit } => list(limit).await,
    }
}

async fn list(limit: usize) -> anyhow::Result<()> {
    let state = wire_dev();
    let sessions = SessionsListQuery::new(Arc::clone(&state.session_repo))
        .execute(limit)
        .await?;

    if sessions.is_empty() {
        println!("No sessions.");
        return Ok(());
    }

    println!("{:<36}  {:>4}  updated_at", "id", "msgs");
    println!("{}", "─".repeat(58));
    for s in &sessions {
        println!("{:<36}  {:>4}  {}", s.id, s.message_count, s.updated_at);
    }
    Ok(())
}
