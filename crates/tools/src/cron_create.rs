use std::sync::Arc;
use async_trait::async_trait;
use serde_json::{json, Value};
use ccode_domain::cron::CronJob;
use ccode_ports::{
    cron::CronRepository,
    provider::ProviderPort,
    PortError,
    tool::{ToolContext, ToolPort},
};

/// Tool that lets the agent schedule a future task by creating a cron job.
pub struct CronCreateTool {
    pub cron_repo: Arc<dyn CronRepository>,
    pub provider:  Arc<dyn ProviderPort>,
}

impl CronCreateTool {
    pub fn new(cron_repo: Arc<dyn CronRepository>, provider: Arc<dyn ProviderPort>) -> Self {
        Self { cron_repo, provider }
    }
}

#[async_trait]
impl ToolPort for CronCreateTool {
    fn name(&self) -> &str { "cron_create" }

    fn description(&self) -> &str {
        "Schedule a future agent task. Provide a natural-language 'when' description \
         (e.g. '每天早上9點', 'every monday at 3pm') and a 'message' the agent should \
         act on at that time. Optionally provide a human-readable 'name'."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "when":    { "type": "string", "description": "Natural-language schedule, e.g. '每天早上9點'" },
                "message": { "type": "string", "description": "Task for the agent to perform at that time" },
                "name":    { "type": "string", "description": "Optional human-readable label" }
            },
            "required": ["when", "message"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<String, PortError> {
        let when = args["when"].as_str()
            .ok_or_else(|| PortError::Tool("missing: when".into()))?
            .to_string();
        let message = args["message"].as_str()
            .ok_or_else(|| PortError::Tool("missing: message".into()))?
            .to_string();
        let name = args["name"].as_str()
            .unwrap_or("agent-scheduled")
            .to_string();

        let schedule = parse_natural_schedule(&*self.provider, &when)
            .await
            .map_err(PortError::Tool)?;

        let now = now_ms();
        let job_id = format!("cron-{now}");
        let mut job = CronJob::new(job_id.clone(), name, when.clone(), schedule.clone(), message, now);
        job.next_run_at = next_run_ms(&schedule);

        self.cron_repo.save(&job).await?;

        let next = job.next_run_at.map(ms_to_rfc3339).unwrap_or_else(|| "unknown".into());

        Ok(json!({
            "job_id": job_id,
            "schedule": schedule,
            "next_run": next,
        }).to_string())
    }
}

// ── Helpers (duplicated from ccode-cron to avoid circular dep) ───────────────

async fn parse_natural_schedule(provider: &dyn ProviderPort, description: &str) -> Result<String, String> {
    use ccode_domain::message::{Message, Role};
    use ccode_ports::provider::CompletionRequest;

    let prompt = format!(
        "Convert to a 5-field cron expression (MIN HOUR DOM MONTH DOW).\n\
         Reply with ONLY the expression, nothing else.\n\n\
         Schedule: {description}"
    );
    let req = CompletionRequest {
        messages: vec![Message::new("q", Role::User, prompt, now_ms())],
        model: None,
        max_tokens: Some(32),
        temperature: Some(0.0),
        tools: vec![],
    };
    let resp = provider.complete(req).await.map_err(|e| e.to_string())?;
    let expr = resp.content.trim().to_string();
    validate_cron(&expr).map_err(|e| format!("LLM returned invalid cron \"{expr}\": {e}"))?;
    Ok(expr)
}

fn validate_cron(s: &str) -> Result<(), String> {
    use std::str::FromStr;
    let fields: Vec<&str> = s.split_whitespace().collect();
    let normalized = match fields.len() {
        5 => format!("0 {} *", fields.join(" ")),
        6 => format!("{} *", s),
        _ => s.to_string(),
    };
    cron::Schedule::from_str(&normalized).map(|_| ()).map_err(|e| e.to_string())
}

fn next_run_ms(schedule: &str) -> Option<u64> {
    use std::str::FromStr;
    use chrono::Utc;
    let fields: Vec<&str> = schedule.split_whitespace().collect();
    let normalized = match fields.len() {
        5 => format!("0 {} *", fields.join(" ")),
        6 => format!("{} *", schedule),
        _ => schedule.to_string(),
    };
    let sched = cron::Schedule::from_str(&normalized).ok()?;
    u64::try_from(sched.upcoming(Utc).next()?.timestamp_millis()).ok()
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
