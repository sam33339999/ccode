use ccode_bootstrap::exports::{
    CompletionRequest, CronJob, CronJobId, CronRepository, Message, ProviderPort, Role,
    next_run_ms, parse_natural_schedule,
};
use ccode_bootstrap::wire_from_config;
use clap::Subcommand;
use std::sync::Arc;

#[derive(Subcommand)]
pub enum Action {
    /// List all scheduled jobs
    List,
    /// Create a new scheduled job
    #[command(visible_alias = "add")]
    Create {
        /// Cron expression or natural language schedule
        #[arg(long)]
        schedule: String,
        /// Message sent to the agent when the job fires
        #[arg(long)]
        message: String,
        /// Optional human-readable name
        #[arg(long, default_value = "agent-scheduled")]
        name: String,
    },
    /// Delete a scheduled job
    #[command(visible_alias = "remove")]
    Delete {
        /// Job ID
        id: String,
    },
    /// Manually trigger one execution of a scheduled job
    Run {
        /// Job ID
        id: String,
    },
}

pub async fn run(action: Action) -> anyhow::Result<()> {
    let state = wire_from_config().map_err(|e| anyhow::anyhow!("bootstrap error: {e}"))?;

    match action {
        Action::List => list(&state.cron_repo).await,
        Action::Create {
            schedule,
            name,
            message,
        } => create(&state.cron_repo, state.provider, schedule, message, name).await,
        Action::Delete { id } => delete(&state.cron_repo, id).await,
        Action::Run { id } => run_job(&state.cron_repo, state.provider, id).await,
    }
}

async fn list(repo: &dyn CronRepository) -> anyhow::Result<()> {
    let jobs = repo.list().await?;
    if jobs.is_empty() {
        println!("No scheduled jobs.");
        return Ok(());
    }
    println!("{:<24}  {:<16}  {:<20}  en  message", "id", "name", "when");
    println!("{}", "─".repeat(90));
    for job in &jobs {
        let enabled = if job.enabled { "✓" } else { "✗" };
        println!(
            "{:<24}  {:<16}  {:<20}  {}   {}",
            job.id,
            truncate(&job.name, 16),
            truncate(&job.description, 20),
            enabled,
            truncate(&job.message, 40),
        );
    }
    Ok(())
}

async fn create(
    repo: &dyn CronRepository,
    provider: Option<Arc<dyn ProviderPort>>,
    schedule_input: String,
    message: String,
    name: String,
) -> anyhow::Result<()> {
    let schedule = parse_schedule(provider.as_deref(), &schedule_input).await?;

    let now = now_ms();
    let job_id = format!("cron-{now}");
    let mut job = CronJob::new(job_id, name, schedule_input, schedule, message, now);
    job.next_run_at = next_run_ms(&job.schedule);

    repo.save(&job).await?;
    println!("Created job: {}", job.id);
    if let Some(t) = job.next_run_at {
        println!("Next run: {}", ms_to_rfc3339(t));
    }
    Ok(())
}

async fn delete(repo: &dyn CronRepository, id: String) -> anyhow::Result<()> {
    repo.delete(&CronJobId(id.clone())).await?;
    println!("Deleted job: {id}");
    Ok(())
}

async fn run_job(
    repo: &dyn CronRepository,
    provider: Option<Arc<dyn ProviderPort>>,
    id: String,
) -> anyhow::Result<()> {
    let provider =
        provider.ok_or_else(|| anyhow::anyhow!("no LLM provider configured — set an API key"))?;
    let job_id = CronJobId(id.clone());
    let mut job = repo
        .find_by_id(&job_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("job not found: {id}"))?;

    let req = CompletionRequest {
        messages: vec![Message::new(
            "cron-run",
            Role::User,
            job.message.clone(),
            now_ms(),
        )],
        model: None,
        max_tokens: None,
        temperature: None,
        tools: vec![],
    };
    let resp = provider.complete(req).await?;

    let now = now_ms();
    job.last_run_at = Some(now);
    job.next_run_at = next_run_ms(&job.schedule);
    repo.save(&job).await?;

    println!("Ran job: {id}");
    if !resp.content.trim().is_empty() {
        println!("{}", resp.content.trim());
    }
    Ok(())
}

async fn parse_schedule(
    provider: Option<&dyn ProviderPort>,
    schedule_input: &str,
) -> anyhow::Result<String> {
    if next_run_ms(schedule_input).is_some() {
        return Ok(schedule_input.to_string());
    }

    let provider =
        provider.ok_or_else(|| anyhow::anyhow!("no LLM provider configured — set an API key"))?;
    parse_natural_schedule(provider, schedule_input)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn ms_to_rfc3339(ms: u64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let t = UNIX_EPOCH + Duration::from_millis(ms);
    let dt: chrono::DateTime<chrono::Utc> = t.into();
    dt.to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::Action;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        action: Action,
    }

    #[test]
    fn parses_list_command() {
        let cli = Cli::try_parse_from(["ccode", "list"]).expect("list should parse");
        assert!(matches!(cli.action, Action::List));
    }

    #[test]
    fn parses_create_command() {
        let cli = Cli::try_parse_from([
            "ccode",
            "create",
            "--schedule",
            "0 9 * * *",
            "--message",
            "daily summary",
        ])
        .expect("create should parse");
        assert!(matches!(cli.action, Action::Create { .. }));
    }

    #[test]
    fn parses_delete_command() {
        let cli = Cli::try_parse_from(["ccode", "delete", "cron-1"]).expect("delete should parse");
        assert!(matches!(cli.action, Action::Delete { .. }));
    }

    #[test]
    fn parses_run_command() {
        let cli = Cli::try_parse_from(["ccode", "run", "cron-1"]).expect("run should parse");
        assert!(matches!(cli.action, Action::Run { .. }));
    }
}
