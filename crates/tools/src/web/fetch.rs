use async_trait::async_trait;
use ccode_platform::security::{DefaultSecretScanner, PolicyError, ScanThenWritePolicy};
use ccode_ports::{
    tool::{ToolContext, ToolPort},
    PortError,
};
use serde_json::{json, Value};

pub struct WebFetchTool {
    client: reqwest::Client,
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolPort for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL and return its content. Supports GET and POST."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to fetch" },
                "method": { "type": "string", "description": "HTTP method (default GET)" },
                "body": { "type": "string", "description": "Request body for POST/PUT" }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, PortError> {
        if !ctx.permission.web_fetch {
            return Err(PortError::PermissionDenied("web_fetch is disabled".into()));
        }

        let url = args["url"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing url".into()))?;
        let method = args["method"].as_str().unwrap_or("GET").to_uppercase();
        let policy = ScanThenWritePolicy::new(DefaultSecretScanner::default());

        let mut req = match method.as_str() {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PUT" => self.client.put(url),
            "DELETE" => self.client.delete(url),
            other => return Err(PortError::Tool(format!("unsupported method: {other}"))),
        };

        if let Some(body) = args["body"].as_str() {
            if matches!(method.as_str(), "POST" | "PUT" | "DELETE") {
                policy.scan_remote_payload(body).map_err(map_policy_error)?;
            }
            req = req.body(body.to_string());
        }

        let resp = req
            .send()
            .await
            .map_err(|e| PortError::Tool(format!("fetch error: {e}")))?;

        let status = resp.status().as_u16();
        let body = resp
            .text()
            .await
            .map_err(|e| PortError::Tool(format!("body read error: {e}")))?;

        let truncated_body = if body.len() > 50000 {
            body[..50000].to_string()
        } else {
            body
        };

        let result = json!({ "status": status, "body": truncated_body });
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
