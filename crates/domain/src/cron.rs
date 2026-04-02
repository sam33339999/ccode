use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CronJobId(pub String);

impl std::fmt::Display for CronJobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A scheduled agent task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: CronJobId,
    pub name: String,
    /// Original human-readable schedule description, e.g. "每天早上 9 點".
    pub description: String,
    /// Standard 5-field cron expression, e.g. "0 9 * * *" (min hour dom month dow).
    /// Derived from `description` via LLM parsing.
    pub schedule: String,
    /// Message sent to the agent when this job fires.
    pub message: String,
    pub enabled: bool,
    /// Unix timestamp ms
    pub created_at: u64,
    /// Unix timestamp ms of last execution
    pub last_run_at: Option<u64>,
    /// Unix timestamp ms of next scheduled execution (None = compute on first tick)
    pub next_run_at: Option<u64>,
}

impl CronJob {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        schedule: impl Into<String>,
        message: impl Into<String>,
        now: u64,
    ) -> Self {
        Self {
            id: CronJobId(id.into()),
            name: name.into(),
            description: description.into(),
            schedule: schedule.into(),
            message: message.into(),
            enabled: true,
            created_at: now,
            last_run_at: None,
            next_run_at: None,
        }
    }
}
