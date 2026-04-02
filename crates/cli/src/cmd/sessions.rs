use ccode_application::commands::{
    sessions_clear::SessionsClearCommand, sessions_delete::SessionsDeleteCommand,
};
use ccode_application::queries::sessions_list::SessionsListQuery;
use ccode_application::queries::sessions_show::{
    SessionMessageView, SessionView, SessionsShowQuery,
};
use ccode_bootstrap::wire_from_config_with_cwd;
use clap::Subcommand;
use std::io::{self, Write};

#[derive(Subcommand)]
pub enum Action {
    /// List recent sessions
    List {
        /// Maximum number of sessions to show
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show full content of a session
    Show {
        /// Session ID
        id: String,
    },
    /// Delete a session by ID
    Delete {
        /// Session ID
        id: String,
    },
    /// Delete all sessions (with confirmation prompt)
    Clear,
}

pub async fn run(action: Action) -> anyhow::Result<()> {
    let state = wire_from_config_with_cwd(std::env::current_dir().ok())
        .map_err(|e| anyhow::anyhow!("bootstrap error: {e}"))?;

    match action {
        Action::List { limit } => list(&state, limit).await,
        Action::Show { id } => show(&state, id).await,
        Action::Delete { id } => delete(&state, id).await,
        Action::Clear => clear(&state).await,
    }
}

async fn list(state: &ccode_bootstrap::AppState, limit: usize) -> anyhow::Result<()> {
    let sessions = SessionsListQuery::new(state.session_repo.clone())
        .execute(limit)
        .await?;

    if sessions.is_empty() {
        println!("No sessions.");
        return Ok(());
    }

    println!("{:<36}  {:>4}  timestamp", "id", "msgs");
    println!("{}", "─".repeat(66));
    for s in &sessions {
        println!(
            "{}",
            render_session_summary_line(&s.id.0, s.message_count, s.updated_at)
        );
    }
    Ok(())
}

async fn show(state: &ccode_bootstrap::AppState, id: String) -> anyhow::Result<()> {
    let session = SessionsShowQuery::new(state.session_repo.clone())
        .execute(id.clone())
        .await?
        .ok_or_else(|| anyhow::anyhow!("session not found: {id}"))?;
    println!("{}", render_session_detail(&session));
    Ok(())
}

async fn delete(state: &ccode_bootstrap::AppState, id: String) -> anyhow::Result<()> {
    let deleted = SessionsDeleteCommand::new(state.session_repo.clone())
        .execute(id.clone())
        .await?;
    if !deleted {
        return Err(anyhow::anyhow!("session not found: {id}"));
    }
    println!("Deleted session: {id}");
    Ok(())
}

async fn clear(state: &ccode_bootstrap::AppState) -> anyhow::Result<()> {
    print!("Delete ALL sessions? Type 'yes' to continue: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    if !is_clear_confirmed(&input) {
        println!("Aborted.");
        return Ok(());
    }

    let deleted = SessionsClearCommand::new(state.session_repo.clone())
        .execute()
        .await?;
    println!("Deleted {deleted} session(s).");
    Ok(())
}

fn render_session_summary_line(id: &str, message_count: usize, updated_at: u64) -> String {
    format!(
        "{:<36}  {:>4}  {}",
        id,
        message_count,
        format_timestamp(updated_at)
    )
}

fn render_session_detail(session: &SessionView) -> String {
    let mut out = String::new();
    out.push_str(&format!("id: {}\n", session.id));
    out.push_str(&format!(
        "created_at: {}\n",
        format_timestamp(session.created_at)
    ));
    out.push_str(&format!(
        "updated_at: {}\n",
        format_timestamp(session.updated_at)
    ));
    out.push_str(&format!("messages: {}\n", session.message_count));
    out.push('\n');

    for m in &session.messages {
        out.push_str(&render_message(m));
        out.push('\n');
    }
    out
}

fn render_message(message: &SessionMessageView) -> String {
    format!(
        "[{}] {} ({})\n{}\n",
        message.id,
        message.role,
        format_timestamp(message.created_at),
        message.content
    )
}

fn format_timestamp(ms: u64) -> String {
    let seconds = (ms / 1000) as i64;
    let nanos = ((ms % 1000) * 1_000_000) as u32;
    match chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, nanos) {
        Some(ts) => ts.to_rfc3339(),
        None => ms.to_string(),
    }
}

fn is_clear_confirmed(input: &str) -> bool {
    input.trim() == "yes"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_confirmation_requires_exact_yes() {
        assert!(is_clear_confirmed("yes\n"));
        assert!(!is_clear_confirmed("y\n"));
        assert!(!is_clear_confirmed("YES\n"));
    }

    #[test]
    fn summary_line_contains_id_message_count_and_rfc3339_timestamp() {
        let line = render_session_summary_line("s-1", 3, 1_700_000_000_000);
        assert!(line.contains("s-1"));
        assert!(line.contains("3"));
        assert!(line.contains("2023-11-14T22:13:20+00:00"));
    }

    #[test]
    fn session_detail_contains_all_messages() {
        let session = SessionView {
            id: "s-1".to_string(),
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_002_000,
            message_count: 2,
            messages: vec![
                SessionMessageView {
                    id: "m1".to_string(),
                    role: "user".to_string(),
                    content: "hello".to_string(),
                    created_at: 1_700_000_001_000,
                },
                SessionMessageView {
                    id: "m2".to_string(),
                    role: "assistant".to_string(),
                    content: "hi there".to_string(),
                    created_at: 1_700_000_002_000,
                },
            ],
        };

        let out = render_session_detail(&session);
        assert!(out.contains("id: s-1"));
        assert!(out.contains("messages: 2"));
        assert!(out.contains("[m1] user"));
        assert!(out.contains("hello"));
        assert!(out.contains("[m2] assistant"));
        assert!(out.contains("hi there"));
    }
}
