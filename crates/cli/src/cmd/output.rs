use anyhow::Error as AnyError;
use std::io::Write;

// ANSI color codes for terminal output
pub const GRAY: &str = "\x1b[90m";
/// Warm amber — used for AI response text
pub const AI_RESPONSE: &str = "\x1b[38;5;222m";
pub const RESET: &str = "\x1b[0m";
use ccode_application::commands::agent_run::estimate_tokens_from_chars;
use ccode_application::error::AppError;
use ccode_bootstrap::WireError;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Policy,
    Auth,
    Transport,
    State,
    Validation,
}

pub fn error_category_label(category: ErrorCategory) -> &'static str {
    match category {
        ErrorCategory::Policy => "policy",
        ErrorCategory::Auth => "auth",
        ErrorCategory::Transport => "transport",
        ErrorCategory::State => "state",
        ErrorCategory::Validation => "validation",
    }
}

pub fn worker_status_label(status: &str) -> Option<&'static str> {
    match status {
        "Running" => Some("Running"),
        "Completed" => Some("Completed"),
        "Failed" => Some("Failed"),
        "Cancelled" => Some("Cancelled"),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct ErrorContext {
    pub session_id: String,
    pub provider_name: String,
}

impl ErrorContext {
    pub fn unknown() -> Self {
        Self {
            session_id: "unknown".to_string(),
            provider_name: "unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ErrorEnvelope {
    pub category: ErrorCategory,
    pub message: String,
    pub suggestion: Option<String>,
    pub correlation_id: String,
    pub session_id: String,
    pub provider_name: String,
}

impl ErrorEnvelope {
    fn new(
        category: ErrorCategory,
        message: impl Into<String>,
        suggestion: Option<String>,
        ctx: &ErrorContext,
    ) -> Self {
        Self {
            category,
            message: message.into(),
            suggestion,
            correlation_id: next_correlation_id(),
            session_id: ctx.session_id.clone(),
            provider_name: ctx.provider_name.clone(),
        }
    }

    pub fn render(&self) -> String {
        let category = error_category_label(self.category);
        let mut out = format!("[error:{category}] {}", self.message);
        if let Some(suggestion) = &self.suggestion {
            out.push_str(&format!(" Hint: {suggestion}"));
        }
        out.push_str(&format!(
            " [correlation_id:{} session_id:{} provider:{}]",
            self.correlation_id, self.session_id, self.provider_name
        ));
        out
    }
}

pub fn render_error(error: &AnyError, ctx: &ErrorContext) -> String {
    let resolved_ctx = resolve_context(error, ctx);
    classify_anyhow_error(error, &resolved_ctx).render()
}

pub fn render_error_message(message: &str, ctx: &ErrorContext) -> String {
    classify_message_into_envelope(message, ctx).render()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRiskLevel {
    Low,
    Medium,
    High,
}

impl ToolRiskLevel {
    pub fn label(self) -> &'static str {
        match self {
            ToolRiskLevel::Low => "low",
            ToolRiskLevel::Medium => "medium",
            ToolRiskLevel::High => "high",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolConfirmationDecision {
    AllowOnce,
    AllowAlways,
    Deny,
}

pub struct StreamFormatter {
    at_line_start: bool,
}

impl StreamFormatter {
    pub fn new() -> Self {
        Self {
            at_line_start: true,
        }
    }

    pub fn push_delta(&mut self, content: &str) -> String {
        if !content.is_empty() {
            self.at_line_start = content.ends_with('\n');
        }
        content.to_string()
    }

    pub fn tool_start_line(&mut self, tool_name: &str, args: &Value) -> String {
        let mut out = String::new();
        if !self.at_line_start {
            out.push('\n');
        }
        let risk = classify_tool_risk(tool_name).label();
        out.push_str(&format!(
            "[tool:start] {} risk={}({})\n",
            tool_name,
            risk,
            summarize_tool_args(args)
        ));
        self.at_line_start = true;
        out
    }

    pub fn tool_result_line(&mut self, tool_name: &str, result: &Result<String, String>) -> String {
        let status = if result.is_ok() { "success" } else { "failure" };
        self.at_line_start = true;
        let mut rendered = format!("[tool:done] {} status={}\n", tool_name, status);
        if let Ok(payload) = result
            && let Some(worker_line) = render_worker_status_line(payload)
        {
            rendered.push_str(worker_line.as_str());
        }
        rendered
    }
}

/// Animated thinking indicator shown while waiting for the first response token.
/// Renders a braille spinner with elapsed time on stderr, erases itself when stopped.
pub struct ThinkingSpinner {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ThinkingSpinner {
    pub fn start_with(stop: Arc<AtomicBool>) -> Self {
        let stop_clone = stop.clone();
        let thread = std::thread::spawn(move || {
            const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut i = 0usize;
            let start = Instant::now();
            while !stop_clone.load(Ordering::Relaxed) {
                let elapsed = start.elapsed().as_secs_f64();
                let (mins, secs) = (elapsed as u64 / 60, elapsed % 60.0);
                let time_str = if mins > 0 {
                    format!("{mins}m {secs:.0}s")
                } else {
                    format!("{secs:.1}s")
                };
                eprint!(
                    "\r\x1b[90m{} thinking… ({time_str})\x1b[0m",
                    FRAMES[i % FRAMES.len()]
                );
                let _ = std::io::stderr().flush();
                i += 1;
                std::thread::sleep(Duration::from_millis(80));
            }
            eprint!("\r\x1b[2K");
            let _ = std::io::stderr().flush();
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }

    /// Clear the spinner line immediately and signal the thread to stop.
    /// Safe to call from any thread; idempotent.
    pub fn clear_and_stop(stop: &Arc<AtomicBool>) {
        if !stop.swap(true, Ordering::Relaxed) {
            // We were the first to stop — clear the line right now so output
            // that follows isn't written on top of the spinner text.
            eprint!("\r\x1b[2K");
            let _ = std::io::stderr().flush();
        }
    }

}

impl Drop for ThinkingSpinner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub struct StreamProgress {
    started_at: Instant,
    last_report_at: Instant,
    output_chars: usize,
}

impl StreamProgress {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            started_at: now,
            last_report_at: now,
            output_chars: 0,
        }
    }

    pub fn on_delta(&mut self, content: &str) -> Option<String> {
        self.output_chars += content.chars().count();
        let now = Instant::now();
        if now.duration_since(self.last_report_at) < Duration::from_secs(1) {
            return None;
        }
        self.last_report_at = now;
        Some(self.render_line(estimate_tokens_from_chars(self.output_chars)))
    }

    pub fn render_line(&self, output_tokens: usize) -> String {
        let elapsed = self.started_at.elapsed().as_secs_f64().max(0.001);
        let tps = output_tokens as f64 / elapsed;
        format!("[stream] out={output_tokens} tok, {tps:.1} tok/s")
    }
}

pub fn summarize_tool_args(args: &Value) -> String {
    match args {
        Value::Object(map) if !map.is_empty() => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            entries
                .into_iter()
                .map(|(k, v)| format!("{k}={}", summarize_arg_value(v)))
                .collect::<Vec<_>>()
                .join(", ")
        }
        Value::Object(_) => String::from("no-args"),
        _ => summarize_arg_value(args),
    }
}

pub fn classify_tool_risk(tool_name: &str) -> ToolRiskLevel {
    match tool_name {
        // Can execute arbitrary commands or mutate local state heavily.
        "shell" | "fs_write" | "fs_edit" | "spawn_agent" | "agent" | "task_stop"
        | "cron_create" => ToolRiskLevel::High,
        // External network requests with lower local impact.
        "browser" | "web_fetch" => ToolRiskLevel::Medium,
        _ => ToolRiskLevel::Low,
    }
}

pub fn confirmation_prompt(tool_name: &str, args: &Value) -> String {
    format!(
        "[tool:confirm] name={} params={} risk={}\nAllow? [y/n/a]: ",
        tool_name,
        summarize_tool_args(args),
        classify_tool_risk(tool_name).label()
    )
}

pub fn parse_confirmation_input(input: &str) -> ToolConfirmationDecision {
    match input.trim().to_ascii_lowercase().as_str() {
        "n" | "no" => ToolConfirmationDecision::Deny,
        "a" | "always" => ToolConfirmationDecision::AllowAlways,
        _ => ToolConfirmationDecision::AllowOnce,
    }
}

pub fn classify_error(message: &str) -> ErrorCategory {
    let m = message.to_ascii_lowercase();
    if m.contains("timed out")
        || m.contains("timeout")
        || m.contains("connection")
        || m.contains("dns")
        || m.contains("network")
        || m.contains("unreachable")
        || m.contains("temporarily unavailable")
    {
        return ErrorCategory::Transport;
    }
    if m.contains("401")
        || m.contains("403")
        || m.contains("unauthorized")
        || m.contains("forbidden")
        || m.contains("api key")
        || m.contains("authentication")
    {
        return ErrorCategory::Auth;
    }
    if m.contains("permission denied")
        || m.contains("policy")
        || m.contains("denied")
        || m.contains("forbidden by")
    {
        return ErrorCategory::Policy;
    }
    if m.contains("invalid")
        || m.contains("parse error")
        || m.contains("missing required")
        || m.contains("bad request")
    {
        return ErrorCategory::Validation;
    }
    if m.contains("not found")
        || m.contains("invalid state")
        || m.contains("already")
        || m.contains("no llm provider configured")
        || m.contains("not configured")
    {
        return ErrorCategory::State;
    }
    ErrorCategory::State
}

fn summarize_arg_value(v: &Value) -> String {
    const MAX_LEN: usize = 32;
    let raw = match v {
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    };
    if raw.chars().count() > MAX_LEN {
        let mut shortened = raw.chars().take(MAX_LEN).collect::<String>();
        shortened.push_str("...");
        shortened
    } else {
        raw
    }
}

fn render_worker_status_line(payload: &str) -> Option<String> {
    let value: Value = serde_json::from_str(payload).ok()?;
    let task_id = value.get("task_id")?.as_str()?;
    let status = value.get("status")?.as_str()?;
    let status = worker_status_label(status)?;
    Some(format!("[worker] task_id={task_id} status={status}\n"))
}

fn classify_anyhow_error(error: &AnyError, ctx: &ErrorContext) -> ErrorEnvelope {
    if let Some(wire) = find_in_chain::<WireError>(error) {
        return classify_wire_error(wire, ctx);
    }
    if let Some(app) = find_in_chain::<AppError>(error) {
        return classify_app_error(app, ctx);
    }
    classify_message_into_envelope(&error.to_string(), ctx)
}

fn resolve_context(error: &AnyError, fallback: &ErrorContext) -> ErrorContext {
    for cause in error.chain() {
        if let Some(text) = parse_error_context(cause.to_string().as_str()) {
            return text;
        }
    }
    fallback.clone()
}

fn parse_error_context(text: &str) -> Option<ErrorContext> {
    let prefix = "error_context ";
    if !text.starts_with(prefix) {
        return None;
    }
    let mut session_id = None;
    let mut provider_name = None;
    for part in text[prefix.len()..].split_whitespace() {
        if let Some((k, v)) = part.split_once('=') {
            match k {
                "session" => session_id = Some(v.to_string()),
                "provider" => provider_name = Some(v.to_string()),
                _ => {}
            }
        }
    }
    Some(ErrorContext {
        session_id: session_id.unwrap_or_else(|| "unknown".to_string()),
        provider_name: provider_name.unwrap_or_else(|| "unknown".to_string()),
    })
}

fn classify_wire_error(error: &WireError, ctx: &ErrorContext) -> ErrorEnvelope {
    match error {
        WireError::Config(config_error) => {
            classify_message_into_envelope(&config_error.to_string(), ctx)
        }
        WireError::Provider(msg) => classify_provider_error_message(msg, ctx),
        WireError::Storage(_) => ErrorEnvelope::new(
            ErrorCategory::State,
            "Local storage is unavailable.",
            Some("Check filesystem permissions and local disk state.".to_string()),
            ctx,
        ),
        WireError::McpRuntime(msg) => ErrorEnvelope::new(
            ErrorCategory::State,
            "MCP runtime is unavailable.",
            Some(format!(
                "Check MCP server configuration and startup command. ({msg})"
            )),
            ctx,
        ),
    }
}

fn classify_app_error(error: &AppError, ctx: &ErrorContext) -> ErrorEnvelope {
    match error {
        AppError::Port(port_error) => {
            let text = port_error.to_string();
            if let Some(provider_message) = text.strip_prefix("provider error: ") {
                return classify_provider_error_message(provider_message, ctx);
            }
            classify_message_into_envelope(&text, ctx)
        }
        AppError::Llm(llm_error) => classify_message_into_envelope(&llm_error.to_string(), ctx),
        AppError::Domain(domain_error) => {
            classify_message_into_envelope(&domain_error.to_string(), ctx)
        }
    }
}

fn classify_message_into_envelope(message: &str, ctx: &ErrorContext) -> ErrorEnvelope {
    let lower = message.to_ascii_lowercase();
    if lower.contains("no llm provider configured")
        || lower.contains("not configured")
        || lower.contains("missing api_key")
        || lower.contains("missing required field")
    {
        return ErrorEnvelope::new(
            ErrorCategory::State,
            "No provider configuration was found.",
            Some(
                "Copy settings from ./config.example.toml and set your API key (for example OPENROUTER_API_KEY).".to_string(),
            ),
            ctx,
        );
    }

    match classify_error(message) {
        ErrorCategory::Auth => ErrorEnvelope::new(
            ErrorCategory::Auth,
            "Authentication failed.",
            Some(
                "Check your API key configuration and environment variables (see ./config.example.toml)."
                    .to_string(),
            ),
            ctx,
        ),
        ErrorCategory::Transport => ErrorEnvelope::new(
            ErrorCategory::Transport,
            "Network request failed.",
            Some("Check your internet connectivity and retry.".to_string()),
            ctx,
        ),
        ErrorCategory::Policy => ErrorEnvelope::new(
            ErrorCategory::Policy,
            "Operation blocked by policy.",
            None,
            ctx,
        ),
        ErrorCategory::Validation => ErrorEnvelope::new(
            ErrorCategory::Validation,
            "Input or configuration is invalid.",
            Some("Review command arguments and configuration values.".to_string()),
            ctx,
        ),
        ErrorCategory::State => {
            if lower.contains("not found") {
                ErrorEnvelope::new(
                    ErrorCategory::State,
                    "Requested resource was not found.",
                    None,
                    ctx,
                )
            } else if lower.contains("invalid state") {
                ErrorEnvelope::new(
                    ErrorCategory::State,
                    "Operation is not allowed in the current state.",
                    None,
                    ctx,
                )
            } else {
                ErrorEnvelope::new(
                    ErrorCategory::State,
                    &format!("Unexpected internal error occurred. Detail: {message}"),
                    Some("Retry the command. If it persists, report the correlation ID.".to_string()),
                    ctx,
                )
            }
        }
    }
}

fn classify_provider_error_message(message: &str, ctx: &ErrorContext) -> ErrorEnvelope {
    // Never surface raw remote endpoint details in user-facing output.
    match classify_error(message) {
        ErrorCategory::Auth => ErrorEnvelope::new(
            ErrorCategory::Auth,
            "Provider authentication failed.",
            Some(
                "Check your API key configuration and provider credentials (see ./config.example.toml)."
                    .to_string(),
            ),
            ctx,
        ),
        ErrorCategory::Transport => ErrorEnvelope::new(
            ErrorCategory::Transport,
            "Provider request failed due to network transport.",
            Some("Check connectivity and retry.".to_string()),
            ctx,
        ),
        ErrorCategory::Validation => ErrorEnvelope::new(
            ErrorCategory::Validation,
            "Provider rejected the request payload.",
            Some("Review request inputs and model settings.".to_string()),
            ctx,
        ),
        _ => ErrorEnvelope::new(
            ErrorCategory::State,
            "Provider request failed.",
            Some("Retry later or switch provider configuration.".to_string()),
            ctx,
        ),
    }
}

fn find_in_chain<T>(error: &AnyError) -> Option<&T>
where
    T: std::error::Error + 'static,
{
    error.chain().find_map(|cause| cause.downcast_ref::<T>())
}

fn next_correlation_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("cid-{ts_ms:016x}-{seq:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn delta_passthrough_has_no_extra_newlines() {
        let mut formatter = StreamFormatter::new();

        let a = formatter.push_delta("Hello");
        let b = formatter.push_delta(", world");
        let c = formatter.push_delta("!\n");

        assert_eq!(format!("{a}{b}{c}"), "Hello, world!\n");
    }

    #[test]
    fn tool_start_shows_name_and_parameter_summary() {
        let mut formatter = StreamFormatter::new();
        let line =
            formatter.tool_start_line("fs_read", &json!({"path": "/tmp/a.txt", "offset": 12}));

        assert_eq!(
            line,
            "[tool:start] fs_read risk=low(offset=12, path=/tmp/a.txt)\n"
        );
    }

    #[test]
    fn tool_result_shows_success_and_failure() {
        let mut formatter = StreamFormatter::new();

        let ok_line = formatter.tool_result_line("fs_read", &Ok("done".to_string()));
        let err_line = formatter.tool_result_line("fs_read", &Err("boom".to_string()));

        assert_eq!(ok_line, "[tool:done] fs_read status=success\n");
        assert_eq!(err_line, "[tool:done] fs_read status=failure\n");
    }

    #[test]
    fn tool_result_includes_worker_status_when_present() {
        let mut formatter = StreamFormatter::new();
        let line = formatter.tool_result_line(
            "agent",
            &Ok(json!({"task_id":"w-1","status":"Running"}).to_string()),
        );

        assert_eq!(
            line,
            "[tool:done] agent status=success\n[worker] task_id=w-1 status=Running\n"
        );
    }

    #[test]
    fn worker_status_labels_are_stable() {
        assert_eq!(worker_status_label("Running"), Some("Running"));
        assert_eq!(worker_status_label("Completed"), Some("Completed"));
        assert_eq!(worker_status_label("Failed"), Some("Failed"));
        assert_eq!(worker_status_label("Cancelled"), Some("Cancelled"));
        assert_eq!(worker_status_label("unknown"), None);
    }

    #[test]
    fn tool_events_start_on_clean_line_mid_stream() {
        let mut formatter = StreamFormatter::new();
        let mut rendered = String::new();

        rendered.push_str(&formatter.push_delta("Partial text"));
        rendered.push_str(&formatter.tool_start_line("shell", &json!({"cmd": "echo hi"})));
        rendered.push_str(&formatter.tool_result_line("shell", &Ok("hi".to_string())));
        rendered.push_str(&formatter.push_delta("resumed"));

        assert_eq!(
            rendered,
            "Partial text\n[tool:start] shell risk=high(cmd=echo hi)\n[tool:done] shell status=success\nresumed"
        );
    }

    #[test]
    fn markdown_code_fence_content_is_preserved() {
        let mut formatter = StreamFormatter::new();
        let mut rendered = String::new();

        rendered.push_str(&formatter.push_delta("```rust\n"));
        rendered.push_str(&formatter.push_delta("fn main() {\n"));
        rendered.push_str(&formatter.push_delta("    println!(\"hi\");\n"));
        rendered.push_str(&formatter.push_delta("}\n"));
        rendered.push_str(&formatter.push_delta("```\n"));

        assert_eq!(
            rendered,
            "```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n"
        );
    }

    #[test]
    fn tool_arg_summary_truncates_long_values() {
        let long = "x".repeat(80);
        let summary = summarize_tool_args(&json!({"path": long, "lines": 20}));

        assert_eq!(
            summary,
            "lines=20, path=xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx..."
        );
    }

    #[test]
    fn classify_error_supports_required_categories() {
        assert_eq!(classify_error("HTTP 401 unauthorized"), ErrorCategory::Auth);
        assert_eq!(
            classify_error("connection timed out while sending request"),
            ErrorCategory::Transport
        );
        assert_eq!(
            classify_error("permission denied by policy"),
            ErrorCategory::Policy
        );
        assert_eq!(
            classify_error("parse error: invalid config"),
            ErrorCategory::Validation
        );
        assert_eq!(classify_error("session not found"), ErrorCategory::State);
    }

    #[test]
    fn error_category_labels_are_stable() {
        assert_eq!(error_category_label(ErrorCategory::Policy), "policy");
        assert_eq!(error_category_label(ErrorCategory::Auth), "auth");
        assert_eq!(error_category_label(ErrorCategory::Transport), "transport");
        assert_eq!(error_category_label(ErrorCategory::State), "state");
        assert_eq!(
            error_category_label(ErrorCategory::Validation),
            "validation"
        );
    }

    #[test]
    fn risk_classification_is_stable() {
        assert_eq!(classify_tool_risk("fs_read"), ToolRiskLevel::Low);
        assert_eq!(classify_tool_risk("web_fetch"), ToolRiskLevel::Medium);
        assert_eq!(classify_tool_risk("shell"), ToolRiskLevel::High);
    }

    #[test]
    fn confirmation_prompt_contains_required_fields_and_shortcuts() {
        let prompt = confirmation_prompt("fs_edit", &json!({"path": "src/main.rs"}));
        assert_eq!(
            prompt,
            "[tool:confirm] name=fs_edit params=path=src/main.rs risk=high\nAllow? [y/n/a]: "
        );
    }

    #[test]
    fn parse_confirmation_input_supports_y_n_a() {
        assert_eq!(
            parse_confirmation_input("y"),
            ToolConfirmationDecision::AllowOnce
        );
        assert_eq!(
            parse_confirmation_input("n"),
            ToolConfirmationDecision::Deny
        );
        assert_eq!(
            parse_confirmation_input("a"),
            ToolConfirmationDecision::AllowAlways
        );
    }

    #[test]
    fn rendered_envelope_contains_correlation_and_context() {
        let rendered = render_error_message(
            "connection timeout",
            &ErrorContext {
                session_id: "s-123".to_string(),
                provider_name: "openrouter".to_string(),
            },
        );
        assert!(rendered.contains("[error:transport]"));
        assert!(rendered.contains("correlation_id:"));
        assert!(rendered.contains("session_id:s-123"));
        assert!(rendered.contains("provider:openrouter"));
    }

    #[test]
    fn provider_error_is_sanitized_and_no_remote_details_are_leaked() {
        let rendered = render_error_message(
            "provider error: HTTP 500: backend trace=abc123, db password=secret",
            &ErrorContext {
                session_id: "unknown".to_string(),
                provider_name: "router".to_string(),
            },
        );

        assert!(rendered.contains("[error:state]"));
        assert!(!rendered.contains("trace=abc123"));
        assert!(!rendered.contains("password=secret"));
    }

    #[test]
    fn missing_config_points_to_example_file() {
        let rendered = render_error_message("no LLM provider configured", &ErrorContext::unknown());
        assert!(rendered.contains("[error:state]"));
        assert!(rendered.contains("config.example.toml"));
    }

    #[test]
    fn render_error_reads_context_from_error_chain() {
        let err = anyhow::anyhow!("root failure")
            .context("error_context provider=openrouter session=s-77");
        let rendered = render_error(&err, &ErrorContext::unknown());
        assert!(rendered.contains("provider:openrouter"));
        assert!(rendered.contains("session_id:s-77"));
    }

    #[test]
    fn stream_progress_renders_token_stats() {
        let mut progress = StreamProgress::new();
        let _ = progress.on_delta("1234");
        let line = progress.render_line(estimate_tokens_from_chars(8));
        assert!(line.contains("out=2 tok"));
        assert!(line.contains("tok/s"));
    }
}
