use async_trait::async_trait;
use ccode_ports::{
    tool::{FsPolicy, ToolContext, ToolPort},
    PortError,
};
use serde_json::{json, Value};

pub struct FsEditTool;

#[async_trait]
impl ToolPort for FsEditTool {
    fn name(&self) -> &str {
        "fs_edit"
    }

    fn description(&self) -> &str {
        "Edit a file. Mode A (string match): provide old_string + new_string — replaces unique occurrence. Mode B (line range): provide from_line + to_line + new_string — replaces exact line range."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to edit" },
                "old_string": { "type": "string", "description": "String to find and replace (Mode A)" },
                "new_string": { "type": "string", "description": "Replacement string" },
                "from_line": { "type": "integer", "description": "Start line for range replacement (Mode B, 1-based)" },
                "to_line": { "type": "integer", "description": "End line for range replacement (Mode B, 1-based, inclusive)" }
            },
            "required": ["path", "new_string"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, PortError> {
        let path_str = args["path"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing path".into()))?;
        let new_string = args["new_string"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing new_string".into()))?;

        let path = std::path::Path::new(path_str);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            ctx.cwd.join(path)
        };

        // Permission check
        match &ctx.permission.fs_write {
            FsPolicy::None => {
                return Err(PortError::PermissionDenied("fs_write is disabled".into()));
            }
            FsPolicy::Cwd => {
                if !resolved.starts_with(&ctx.cwd) {
                    return Err(PortError::PermissionDenied(format!(
                        "path {} is outside cwd",
                        resolved.display()
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

        let original = tokio::fs::read_to_string(&resolved)
            .await
            .map_err(|e| PortError::Tool(format!("read error: {e}")))?;

        let size_before = original.len() as u64;

        let (new_content, lines_replaced) = if let Some(old_string) = args["old_string"].as_str() {
            // Mode A: string replacement
            let count = original.matches(old_string).count();
            if count == 0 {
                return Err(PortError::Tool("old_string not found".into()));
            }
            if count > 1 {
                return Err(PortError::Tool(format!(
                    "old_string matches {} locations, add more context",
                    count
                )));
            }
            let replaced = original.replacen(old_string, new_string, 1);
            let old_lines = old_string.lines().count().max(1) as u64;
            (replaced, old_lines)
        } else if let Some(from_line) = args["from_line"].as_u64() {
            // Mode B: line range replacement
            let to_line = args["to_line"]
                .as_u64()
                .ok_or_else(|| PortError::Tool("from_line requires to_line".into()))?;

            let lines: Vec<&str> = original.lines().collect();
            let total = lines.len() as u64;

            if from_line < 1 || from_line > total {
                return Err(PortError::Tool(format!(
                    "from_line {} out of range (total {})",
                    from_line, total
                )));
            }
            if to_line < from_line || to_line > total {
                return Err(PortError::Tool(format!(
                    "to_line {} out of range (from_line {}, total {})",
                    to_line, from_line, total
                )));
            }

            let from_idx = (from_line - 1) as usize;
            let to_idx = to_line as usize; // exclusive

            let replacement_lines: Vec<&str> = new_string.lines().collect();
            let replaced_count = to_line - from_line + 1;

            let mut new_lines: Vec<&str> = Vec::new();
            new_lines.extend_from_slice(&lines[..from_idx]);
            new_lines.extend_from_slice(&replacement_lines);
            new_lines.extend_from_slice(&lines[to_idx..]);

            // Preserve trailing newline if original had one
            let mut joined = new_lines.join("\n");
            if original.ends_with('\n') {
                joined.push('\n');
            }
            (joined, replaced_count)
        } else {
            return Err(PortError::Tool(
                "provide either old_string or from_line+to_line".into(),
            ));
        };

        let size_after = new_content.len() as u64;

        // Atomic write
        let tmp_path = resolved.with_extension(format!(
            "{}.tmp",
            resolved
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("bak")
        ));
        tokio::fs::write(&tmp_path, &new_content)
            .await
            .map_err(|e| PortError::Tool(format!("write error: {e}")))?;
        tokio::fs::rename(&tmp_path, &resolved)
            .await
            .map_err(|e| PortError::Tool(format!("rename error: {e}")))?;

        let result = json!({
            "path": resolved.to_string_lossy(),
            "lines_replaced": lines_replaced,
            "size_before": size_before,
            "size_after": size_after
        });
        Ok(result.to_string())
    }
}
