use async_trait::async_trait;
use serde_json::{json, Value};
use ccode_ports::{
    PortError,
    tool::{FsPolicy, ToolContext, ToolPort},
};

pub struct FsReadTool;

#[async_trait]
impl ToolPort for FsReadTool {
    fn name(&self) -> &str {
        "fs_read"
    }

    fn description(&self) -> &str {
        "Read a file's content with optional line range and comment filtering. Returns line-numbered content."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to read" },
                "offset": { "type": "integer", "description": "Starting line number (1-based, default 1)" },
                "limit": { "type": "integer", "description": "Max lines to return (default 200, max 500)" },
                "filter_level": {
                    "type": "string",
                    "enum": ["none", "minimal", "aggressive"],
                    "description": "Comment filtering level (default none)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, PortError> {
        let path_str = args["path"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing path".into()))?;

        let offset = args["offset"].as_u64().unwrap_or(1).max(1) as usize;
        let limit = args["limit"].as_u64().unwrap_or(200).min(500) as usize;
        let filter_level = args["filter_level"].as_str().unwrap_or("none");

        let path = std::path::Path::new(path_str);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            ctx.cwd.join(path)
        };

        // Permission check
        match &ctx.permission.fs_read {
            FsPolicy::None => {
                return Err(PortError::PermissionDenied("fs_read is disabled".into()));
            }
            FsPolicy::Cwd => {
                if !resolved.starts_with(&ctx.cwd) {
                    return Err(PortError::PermissionDenied(format!(
                        "path {} is outside cwd {}",
                        resolved.display(),
                        ctx.cwd.display()
                    )));
                }
            }
            FsPolicy::Paths(allowed) => {
                if !allowed.iter().any(|p| resolved.starts_with(p)) {
                    return Err(PortError::PermissionDenied(format!(
                        "path {} is not in allowed paths",
                        resolved.display()
                    )));
                }
            }
            FsPolicy::Any => {}
        }

        let content = tokio::fs::read_to_string(&resolved)
            .await
            .map_err(|e| PortError::Tool(format!("read error: {e}")))?;

        let ext = resolved
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let lines: Vec<String> = if filter_level == "none" {
            content.lines().map(|l| l.to_string()).collect()
        } else {
            let comment_prefix = match ext {
                "rs" | "go" | "java" | "c" | "h" | "cpp" | "js" | "ts" => Some("//"),
                "py" | "rb" | "sh" => Some("#"),
                _ => None,
            };

            content
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    if let Some(prefix) = comment_prefix {
                        if trimmed.starts_with(prefix) {
                            return false;
                        }
                    }
                    if filter_level == "aggressive" && trimmed.is_empty() {
                        return false;
                    }
                    true
                })
                .map(|l| l.to_string())
                .collect()
        };

        let total_lines = lines.len() as u64;
        let from = offset as u64;
        let start = offset.saturating_sub(1);
        let end = (start + limit).min(lines.len());
        let to = end as u64;
        let truncated = end < lines.len();

        let width = to.to_string().len();
        let mut out = String::new();
        for (i, line) in lines[start..end].iter().enumerate() {
            let line_no = start + i + 1;
            out.push_str(&format!("{:>width$} \u{2502} {}\n", line_no, line));
        }

        let result = json!({
            "path": resolved.to_string_lossy(),
            "total_lines": total_lines,
            "from": from,
            "to": to,
            "truncated": truncated,
            "content": out
        });

        Ok(result.to_string())
    }
}
