use async_trait::async_trait;
use serde_json::{json, Value};
use ccode_ports::{
    PortError,
    tool::{ShellPolicy, ToolContext, ToolPort},
};

pub struct ShellTool;

#[async_trait]
impl ToolPort for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Execute a shell command. Returns exit code, stdout, and stderr."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "cwd": { "type": "string", "description": "Working directory (default: tool context cwd)" },
                "timeout_secs": { "type": "integer", "description": "Timeout in seconds (default 30)" }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, PortError> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing command".into()))?;
        let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(30);

        // Permission check
        match &ctx.permission.shell {
            ShellPolicy::None => {
                return Err(PortError::PermissionDenied("shell is disabled".into()));
            }
            ShellPolicy::Allowlist(allowed) => {
                let first_token = command.split_whitespace().next().unwrap_or("");
                if !allowed.iter().any(|a| a == first_token) {
                    return Err(PortError::PermissionDenied(format!(
                        "command '{}' not in allowlist",
                        first_token
                    )));
                }
            }
            ShellPolicy::Any => {}
        }

        let work_dir = if let Some(cwd_str) = args["cwd"].as_str() {
            std::path::PathBuf::from(cwd_str)
        } else {
            ctx.cwd.clone()
        };

        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command).current_dir(&work_dir);

        let timeout = tokio::time::Duration::from_secs(timeout_secs);
        let output = tokio::time::timeout(timeout, cmd.output())
            .await
            .map_err(|_| PortError::Tool(format!("command timed out after {}s", timeout_secs)))?
            .map_err(|e| PortError::Tool(format!("spawn error: {e}")))?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        let result = json!({
            "exit_code": exit_code,
            "stdout": stdout,
            "stderr": stderr
        });
        Ok(result.to_string())
    }
}
