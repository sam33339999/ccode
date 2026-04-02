use async_trait::async_trait;
use ccode_platform::security::{
    DefaultSecretScanner, PathValidationError, PolicyError, ScanThenWritePolicy,
};
use ccode_ports::{
    tool::{FsPolicy, ToolContext, ToolPort},
    PortError,
};
use serde_json::{json, Value};

pub struct FsWriteTool;

#[async_trait]
impl ToolPort for FsWriteTool {
    fn name(&self) -> &str {
        "fs_write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file and any parent directories if they don't exist."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to write" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, PortError> {
        let path_str = args["path"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing path".into()))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing content".into()))?;

        let path = std::path::Path::new(path_str);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            ctx.cwd.join(path)
        };
        let policy = ScanThenWritePolicy::new(DefaultSecretScanner::default());

        let resolved = match &ctx.permission.fs_write {
            FsPolicy::None => {
                return Err(PortError::PermissionDenied("fs_write is disabled".into()));
            }
            FsPolicy::Cwd => policy
                .validate_then_scan(&ctx.cwd, &resolved, content)
                .map_err(map_policy_error)?,
            FsPolicy::Paths(allowed) => {
                let mut last_error = None;
                let mut validated = None;
                for root in allowed {
                    match policy.validate_then_scan(root, &resolved, content) {
                        Ok(path) => {
                            validated = Some(path);
                            break;
                        }
                        Err(PolicyError::Path(PathValidationError::OutsideRoot)) => continue,
                        Err(err) => {
                            last_error = Some(err);
                            break;
                        }
                    }
                }
                if let Some(path) = validated {
                    path
                } else if let Some(err) = last_error {
                    return Err(map_policy_error(err));
                } else {
                    return Err(PortError::PermissionDenied(format!(
                        "path {} is not in allowed paths",
                        resolved.display()
                    )));
                }
            }
            FsPolicy::Any => policy
                .validate_then_scan(std::path::Path::new("/"), &resolved, content)
                .map_err(map_policy_error)?,
        };

        // Create parent directories if needed
        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| PortError::Tool(format!("mkdir error: {e}")))?;
        }

        // Atomic write via temp file
        let tmp_path = resolved.with_extension(format!(
            "{}.tmp",
            resolved
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("bak")
        ));

        tokio::fs::write(&tmp_path, content)
            .await
            .map_err(|e| PortError::Tool(format!("write error: {e}")))?;

        tokio::fs::rename(&tmp_path, &resolved)
            .await
            .map_err(|e| PortError::Tool(format!("rename error: {e}")))?;

        let written_bytes = content.len() as u64;
        let result = json!({ "written_bytes": written_bytes });
        Ok(result.to_string())
    }
}

fn map_policy_error(err: PolicyError) -> PortError {
    match err {
        PolicyError::Path(path_err) => PortError::PermissionDenied(path_err.to_string()),
        PolicyError::SecretDetected { findings } => {
            let summary = findings
                .iter()
                .map(|f| format!("{}:{}", f.rule_id, f.label))
                .collect::<Vec<_>>()
                .join(", ");
            PortError::Tool(format!("secret detected ({summary})"))
        }
    }
}
