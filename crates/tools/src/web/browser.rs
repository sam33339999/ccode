use async_trait::async_trait;
use ccode_ports::{
    tool::{ToolContext, ToolPort},
    PortError,
};
use serde_json::{json, Value};

const BROWSER_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub struct BrowserTool {
    client: reqwest::Client,
}

impl BrowserTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent(BROWSER_UA)
                .build()
                .unwrap_or_default(),
        }
    }
}

impl Default for BrowserTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolPort for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Fetch a web page with a browser User-Agent, strip scripts/styles, and return readable content."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to browse" }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<String, PortError> {
        if !ctx.permission.browser {
            return Err(PortError::PermissionDenied("browser is disabled".into()));
        }

        let url = args["url"]
            .as_str()
            .ok_or_else(|| PortError::Tool("missing url".into()))?;

        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| PortError::Tool(format!("fetch error: {e}")))?;

        let final_url = resp.url().to_string();
        let body = resp
            .text()
            .await
            .map_err(|e| PortError::Tool(format!("body read error: {e}")))?;

        // Strip <script>...</script> and <style>...</style> blocks
        let script_re = regex::Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
        let style_re = regex::Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
        let body = script_re.replace_all(&body, "").into_owned();
        let body = style_re.replace_all(&body, "").into_owned();

        let truncated = if body.len() > 50000 {
            body[..50000].to_string()
        } else {
            body
        };

        let result = json!({ "url": final_url, "body": truncated });
        Ok(result.to_string())
    }
}
