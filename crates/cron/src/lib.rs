use async_trait::async_trait;
use ccode_domain::{
    cron::{CronJob, CronJobId},
    message::{Message, Role},
};
use ccode_ports::{
    PortError,
    cron::CronRepository,
    provider::{CompletionRequest, ProviderPort},
};
use chrono::Utc;
use std::path::PathBuf;
use std::str::FromStr;

pub struct FileCronRepo {
    dir: PathBuf,
}

impl FileCronRepo {
    pub fn new(dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path_for(&self, id: &CronJobId) -> PathBuf {
        self.dir.join(format!("{}.json", id.0))
    }
}

#[async_trait]
impl CronRepository for FileCronRepo {
    async fn list(&self) -> Result<Vec<CronJob>, PortError> {
        let mut entries = tokio::fs::read_dir(&self.dir)
            .await
            .map_err(|e| PortError::Storage(e.to_string()))?;

        let mut jobs = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| PortError::Storage(e.to_string()))?
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let data = tokio::fs::read(&path)
                .await
                .map_err(|e| PortError::Storage(e.to_string()))?;
            match serde_json::from_slice::<CronJob>(&data) {
                Ok(job) => jobs.push(job),
                Err(e) => tracing::warn!("skipping corrupt cron file {:?}: {e}", path),
            }
        }
        jobs.sort_by_key(|j| j.created_at);
        Ok(jobs)
    }

    async fn find_by_id(&self, id: &CronJobId) -> Result<Option<CronJob>, PortError> {
        let path = self.path_for(id);
        match tokio::fs::read(&path).await {
            Ok(data) => {
                let job =
                    serde_json::from_slice(&data).map_err(|e| PortError::Storage(e.to_string()))?;
                Ok(Some(job))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(PortError::Storage(e.to_string())),
        }
    }

    async fn save(&self, job: &CronJob) -> Result<(), PortError> {
        let path = self.path_for(&job.id);
        let tmp = path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(job).map_err(|e| PortError::Storage(e.to_string()))?;
        tokio::fs::write(&tmp, &data)
            .await
            .map_err(|e| PortError::Storage(e.to_string()))?;
        tokio::fs::rename(&tmp, &path)
            .await
            .map_err(|e| PortError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn delete(&self, id: &CronJobId) -> Result<(), PortError> {
        let path = self.path_for(id);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(PortError::Storage(e.to_string())),
        }
    }
}

/// Ask the LLM to convert a natural-language schedule description into a
/// 5-field cron expression.  Returns the validated cron string.
pub async fn parse_natural_schedule(
    provider: &dyn ProviderPort,
    description: &str,
) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let prompt = format!(
        "Convert the following schedule description to a standard 5-field cron expression.\n\
         Format: MIN HOUR DOM MONTH DOW\n\
         Examples: \"每天早上9點\" → \"0 9 * * *\", \"每週一下午3點\" → \"0 15 * * 1\", \"每小時\" → \"0 * * * *\"\n\
         Rules:\n\
         - Reply with ONLY the 5-field cron expression, nothing else\n\
         - No explanation, no markdown, no quotes\n\n\
         Schedule: {description}"
    );

    let req = CompletionRequest {
        messages: vec![Message::new("q", Role::User, prompt, now)],
        model: None,
        max_tokens: Some(32),
        temperature: Some(0.0),
        tools: vec![],
    };

    let resp = provider
        .complete(req)
        .await
        .map_err(|e| format!("provider error: {e}"))?;

    let expr = resp.content.trim().to_string();
    validate(&expr).map_err(|e| format!("LLM returned invalid cron \"{expr}\": {e}"))?;
    Ok(expr)
}

/// Parse a 5-field unix cron expression and return the next run timestamp (ms).
/// Accepts standard "min hour dom month dow" format.
/// Returns None if the expression is invalid.
pub fn next_run_ms(schedule: &str) -> Option<u64> {
    let normalized = normalize(schedule);
    let sched = cron::Schedule::from_str(&normalized).ok()?;
    let next = sched.upcoming(Utc).next()?;
    u64::try_from(next.timestamp_millis()).ok()
}

/// Validate a cron expression. Returns Ok(()) or Err(description).
pub fn validate(schedule: &str) -> Result<(), String> {
    let normalized = normalize(schedule);
    cron::Schedule::from_str(&normalized)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Convert 5-field "min hour dom month dow" → 7-field "0 min hour dom month dow *"
/// required by the `cron` crate. 6 or 7 field strings are passed through.
fn normalize(s: &str) -> String {
    let fields: Vec<&str> = s.split_whitespace().collect();
    match fields.len() {
        5 => format!("0 {} *", fields.join(" ")),
        6 => format!("{} *", s),
        _ => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_5_field() {
        assert_eq!(normalize("0 9 * * *"), "0 0 9 * * * *");
    }

    #[test]
    fn validate_ok() {
        assert!(validate("0 9 * * *").is_ok());
        assert!(validate("*/5 * * * *").is_ok());
    }

    #[test]
    fn validate_err() {
        assert!(validate("not a cron").is_err());
    }

    #[test]
    fn next_run_is_future() {
        let ms = next_run_ms("* * * * *").unwrap();
        let now = chrono::Utc::now().timestamp_millis() as u64;
        assert!(ms > now);
    }
}
