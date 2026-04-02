use ccode_bootstrap::exports::{
    CronJob, CronJobId, CronRepository, ProviderPort, next_run_ms, parse_natural_schedule,
};
use ccode_bootstrap::wire_from_config;
use clap::Subcommand;
use std::sync::Arc;

#[derive(Subcommand)]
pub enum Action {
    /// List all scheduled jobs
    List,
    /// Add a new scheduled job
    Add {
        /// Human-readable name
        #[arg(short, long)]
        name: String,
        /// Natural language schedule, e.g. "每天早上 9 點" or "every monday at 3pm"
        #[arg(short, long)]
        when: String,
        /// Message sent to the agent when the job fires
        #[arg(short, long)]
        message: String,
    },
    /// Remove a scheduled job
    Remove {
        /// Job ID
        id: String,
    },
    /// Enable a job
    Enable {
        /// Job ID
        id: String,
    },
    /// Disable a job (without deleting it)
    Disable {
        /// Job ID
        id: String,
    },
}

pub async fn run(action: Action) -> anyhow::Result<()> {
    let state = wire_from_config().map_err(|e| anyhow::anyhow!("bootstrap error: {e}"))?;

    match action {
        Action::List => list(&state.cron_repo).await,
        Action::Add {
            name,
            when,
            message,
        } => {
            let provider = state
                .provider
                .ok_or_else(|| anyhow::anyhow!("no LLM provider configured — set an API key"))?;
            add(&state.cron_repo, provider, name, when, message).await
        }
        Action::Remove { id } => remove(&state.cron_repo, id).await,
        Action::Enable { id } => set_enabled(&state.cron_repo, id, true).await,
        Action::Disable { id } => set_enabled(&state.cron_repo, id, false).await,
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

async fn add(
    repo: &dyn CronRepository,
    provider: Arc<dyn ProviderPort>,
    name: String,
    when: String,
    message: String,
) -> anyhow::Result<()> {
    eprint!("Parsing schedule...");
    let schedule = parse_natural_schedule(&*provider, &when)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    eprintln!(" → {schedule}");

    let now = now_ms();
    let job_id = format!("cron-{now}");
    let mut job = CronJob::new(job_id, name, when, schedule, message, now);
    job.next_run_at = next_run_ms(&job.schedule);

    repo.save(&job).await?;
    println!("Created job: {}", job.id);
    if let Some(t) = job.next_run_at {
        println!("Next run: {}", ms_to_rfc3339(t));
    }
    Ok(())
}

async fn remove(repo: &dyn CronRepository, id: String) -> anyhow::Result<()> {
    repo.delete(&CronJobId(id.clone())).await?;
    println!("Deleted job: {id}");
    Ok(())
}

async fn set_enabled(repo: &dyn CronRepository, id: String, enabled: bool) -> anyhow::Result<()> {
    let job_id = CronJobId(id.clone());
    let mut job = repo
        .find_by_id(&job_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("job not found: {id}"))?;
    job.enabled = enabled;
    if enabled {
        job.next_run_at = next_run_ms(&job.schedule);
    }
    repo.save(&job).await?;
    let state = if enabled { "enabled" } else { "disabled" };
    println!("Job {id} {state}");
    Ok(())
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
