use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Auth,
    Network,
    RateLimit,
    Other,
}

pub fn error_category_label(category: ErrorCategory) -> &'static str {
    match category {
        ErrorCategory::Auth => "auth",
        ErrorCategory::Network => "network",
        ErrorCategory::RateLimit => "rate_limit",
        ErrorCategory::Other => "other",
    }
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
        out.push_str(&format!(
            "[tool:start] {}({})\n",
            tool_name,
            summarize_tool_args(args)
        ));
        self.at_line_start = true;
        out
    }

    pub fn tool_result_line(&mut self, tool_name: &str, result: &Result<String, String>) -> String {
        let status = if result.is_ok() { "success" } else { "failure" };
        self.at_line_start = true;
        format!("[tool:done] {} status={}\n", tool_name, status)
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

pub fn classify_error(message: &str) -> ErrorCategory {
    let m = message.to_ascii_lowercase();
    if m.contains("401")
        || m.contains("403")
        || m.contains("unauthorized")
        || m.contains("forbidden")
        || m.contains("api key")
        || m.contains("authentication")
    {
        return ErrorCategory::Auth;
    }
    if m.contains("429")
        || m.contains("rate limit")
        || m.contains("too many requests")
        || m.contains("quota")
    {
        return ErrorCategory::RateLimit;
    }
    if m.contains("timed out")
        || m.contains("timeout")
        || m.contains("connection")
        || m.contains("dns")
        || m.contains("network")
        || m.contains("unreachable")
        || m.contains("temporarily unavailable")
    {
        return ErrorCategory::Network;
    }
    ErrorCategory::Other
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

        assert_eq!(line, "[tool:start] fs_read(offset=12, path=/tmp/a.txt)\n");
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
    fn tool_events_start_on_clean_line_mid_stream() {
        let mut formatter = StreamFormatter::new();
        let mut rendered = String::new();

        rendered.push_str(&formatter.push_delta("Partial text"));
        rendered.push_str(&formatter.tool_start_line("shell", &json!({"cmd": "echo hi"})));
        rendered.push_str(&formatter.tool_result_line("shell", &Ok("hi".to_string())));
        rendered.push_str(&formatter.push_delta("resumed"));

        assert_eq!(
            rendered,
            "Partial text\n[tool:start] shell(cmd=echo hi)\n[tool:done] shell status=success\nresumed"
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
            ErrorCategory::Network
        );
        assert_eq!(
            classify_error("HTTP 429 too many requests"),
            ErrorCategory::RateLimit
        );
        assert_eq!(classify_error("some other failure"), ErrorCategory::Other);
    }

    #[test]
    fn error_category_labels_are_stable() {
        assert_eq!(error_category_label(ErrorCategory::Auth), "auth");
        assert_eq!(error_category_label(ErrorCategory::Network), "network");
        assert_eq!(error_category_label(ErrorCategory::RateLimit), "rate_limit");
        assert_eq!(error_category_label(ErrorCategory::Other), "other");
    }
}
