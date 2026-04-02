use super::types::*;
use async_stream::stream;
use ccode_domain::message::{Role, ToolCall};
use ccode_ports::provider::{
    LlmError, LlmRequest, LlmResponse, LlmStream, StreamEvent, TokenUsage,
};
use futures::StreamExt;
use std::collections::HashMap;
use std::time::Duration;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// HTTP client for the Anthropic Messages API.
///
/// Wire format differences from OpenAI-compat:
/// - Auth: `x-api-key` header (not Bearer)
/// - Required header: `anthropic-version`
/// - `system` prompt is a top-level field, not in the messages array
/// - `max_tokens` is required
/// - SSE ends with `event: message_stop`, no `[DONE]` sentinel
pub struct AnthropicCompatClient {
    client: reqwest::Client,
    pub api_key: String,
    pub base_url: String,
    pub default_model: String,
}

impl AnthropicCompatClient {
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            base_url: base_url.into(),
            default_model: default_model.into(),
        }
    }

    fn messages_endpoint(&self) -> String {
        format!("{}/messages", self.base_url)
    }

    fn models_endpoint(&self) -> String {
        format!("{}/models", self.base_url)
    }

    fn add_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
    }

    /// 將 LlmRequest 的 messages 轉換為 Anthropic 格式。
    ///
    /// 三種轉換規則：
    /// 1. System → 抽出，合併為頂層 `system` 欄位
    /// 2. Assistant + tool_calls → content 為 block 陣列（text block + tool_use blocks）
    /// 3. Tool（工具結果）→ role="user"，content 為 tool_result block 陣列；
    ///    **連續多個 Tool 訊息必須合併成同一個 user 訊息**（Anthropic 禁止連續同 role）
    fn split_system(&self, req: &LlmRequest) -> (Option<String>, Vec<AnthropicMessage>) {
        use serde_json::{Value, json};

        let mut system_parts: Vec<String> = Vec::new();
        let mut messages: Vec<AnthropicMessage> = Vec::new();

        // 先過濾 System，並把其餘訊息逐一轉換
        let non_system: Vec<_> = req
            .messages
            .iter()
            .filter(|m| {
                if m.role == Role::System {
                    system_parts.push(m.content.clone());
                    false
                } else {
                    true
                }
            })
            .collect();

        let mut i = 0;
        while i < non_system.len() {
            let m = non_system[i];

            match m.role {
                Role::Tool => {
                    // 收集所有連續的 Tool 訊息，合併成一個 user message
                    let mut tool_result_blocks: Vec<Value> = Vec::new();
                    while i < non_system.len() && non_system[i].role == Role::Tool {
                        let tm = non_system[i];
                        tool_result_blocks.push(json!({
                            "type": "tool_result",
                            "tool_use_id": tm.tool_call_id.as_deref().unwrap_or(""),
                            "content": tm.content,
                        }));
                        i += 1;
                    }
                    messages.push(AnthropicMessage {
                        role: "user".into(),
                        content: Value::Array(tool_result_blocks),
                    });
                }
                Role::Assistant => {
                    // 若 assistant 訊息有 tool_calls，建立 content block 陣列
                    if let Some(tool_calls) = &m.tool_calls {
                        let mut blocks: Vec<Value> = Vec::new();
                        // 文字內容（可能為空）
                        if !m.content.is_empty() {
                            blocks.push(json!({"type": "text", "text": m.content}));
                        }
                        // tool_use blocks
                        for tc in tool_calls {
                            let input: Value = serde_json::from_str(&tc.arguments)
                                .unwrap_or(Value::Object(Default::default()));
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.name,
                                "input": input,
                            }));
                        }
                        messages.push(AnthropicMessage {
                            role: "assistant".into(),
                            content: Value::Array(blocks),
                        });
                    } else {
                        // 純文字回應
                        messages.push(AnthropicMessage {
                            role: "assistant".into(),
                            content: Value::String(m.content.clone()),
                        });
                    }
                    i += 1;
                }
                Role::User => {
                    messages.push(AnthropicMessage {
                        role: "user".into(),
                        content: Value::String(m.content.clone()),
                    });
                    i += 1;
                }
                Role::System => unreachable!("already filtered above"),
            }
        }

        let system = if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n"))
        };

        (system, messages)
    }

    fn build_body(&self, req: &LlmRequest, stream: bool) -> AnthropicRequest {
        let model = req
            .model
            .clone()
            .unwrap_or_else(|| self.default_model.clone());
        let (system, messages) = self.split_system(req);
        let tools: Vec<AnthropicTool> = req
            .tools
            .iter()
            .map(|t| AnthropicTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
            })
            .collect();
        AnthropicRequest {
            model,
            max_tokens: req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            system,
            messages,
            stream,
            temperature: req.temperature,
            tools,
        }
    }

    pub async fn health_check(&self) -> Result<(), LlmError> {
        let resp = self
            .add_headers(self.client.get(self.models_endpoint()))
            .send()
            .await
            .map_err(map_reqwest_error)?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(LlmError::ProviderError {
                status: resp.status().as_u16(),
                message: "health check failed".into(),
            })
        }
    }

    pub async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        let body = self.build_body(&req, false);
        let resp = self
            .add_headers(self.client.post(self.messages_endpoint()).json(&body))
            .send()
            .await
            .map_err(map_reqwest_error)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(map_http_status_error(status.as_u16(), text));
        }

        let ar: AnthropicResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        let content = ar
            .content
            .into_iter()
            .filter(|c| c.content_type == "text")
            .filter_map(|c| c.text)
            .collect::<Vec<_>>()
            .join("");

        Ok(LlmResponse {
            content,
            model: ar.model,
            usage: Some(TokenUsage {
                prompt_tokens: ar.usage.input_tokens,
                completion_tokens: ar.usage.output_tokens,
                total_tokens: ar.usage.input_tokens + ar.usage.output_tokens,
            }),
        })
    }

    pub async fn stream(&self, req: LlmRequest) -> Result<LlmStream, LlmError> {
        let body = self.build_body(&req, true);
        let resp = self
            .add_headers(self.client.post(self.messages_endpoint()).json(&body))
            .send()
            .await
            .map_err(map_reqwest_error)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(map_http_status_error(status.as_u16(), text));
        }

        let bytes_stream = resp.bytes_stream();

        let s = stream! {
            tokio::pin!(bytes_stream);
            let mut buf = String::new();
            let mut input_tokens: u32 = 0;
            // Map from block index -> (id, name, accumulated_json)
            let mut tool_use_buf: HashMap<usize, (String, String, String)> = HashMap::new();
            // Track which block index is currently a tool_use block
            let mut tool_use_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();

            while let Some(chunk) = bytes_stream.next().await {
                match chunk {
                    Err(e) => {
                        yield Err(LlmError::StreamInterrupted(e.to_string()));
                        return;
                    }
                    Ok(bytes) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));

                        loop {
                            match buf.find('\n') {
                                None => break,
                                Some(pos) => {
                                    let line = buf[..pos].trim().to_string();
                                    buf = buf[pos + 1..].to_string();

                                    // Skip blank lines and "event:" lines —
                                    // all needed info is in the "data:" payload.
                                    if line.is_empty() || line.starts_with("event:") {
                                        continue;
                                    }
                                    if !line.starts_with("data:") {
                                        continue;
                                    }

                                    let data = line["data:".len()..].trim();

                                    match serde_json::from_str::<AnthropicEvent>(data) {
                                        Err(e) => {
                                            tracing::debug!("skip unparseable Anthropic SSE: {e}");
                                        }
                                        Ok(event) => match event.event_type.as_str() {
                                            "message_start" => {
                                                if let Some(msg) = event.message
                                                    && let Some(u) = msg.usage
                                                {
                                                    input_tokens = u.input_tokens;
                                                }
                                            }
                                            "content_block_start" => {
                                                if let (Some(block), Some(idx)) =
                                                    (event.content_block, event.index)
                                                    && block.block_type == "tool_use"
                                                {
                                                    tool_use_indices.insert(idx);
                                                    tool_use_buf.insert(
                                                        idx,
                                                        (
                                                            block.id.unwrap_or_default(),
                                                            block.name.unwrap_or_default(),
                                                            String::new(),
                                                        ),
                                                    );
                                                }
                                            }
                                            "content_block_delta" => {
                                                if let Some(delta) = event.delta {
                                                    if delta.delta_type == "text_delta" {
                                                        if let Some(text) = delta.text
                                                            && !text.is_empty()
                                                        {
                                                            yield Ok(StreamEvent::Delta {
                                                                content: text,
                                                            });
                                                        }
                                                    } else if delta.delta_type == "input_json_delta"
                                                        && let (Some(partial), Some(idx)) =
                                                            (delta.partial_json, event.index)
                                                        && let Some(entry) =
                                                            tool_use_buf.get_mut(&idx)
                                                    {
                                                        entry.2.push_str(&partial);
                                                    }
                                                }
                                            }
                                            "content_block_stop" => {
                                                // When a tool_use block stops, emit its tool call
                                                if let Some(idx) = event.index
                                                    && tool_use_indices.remove(&idx)
                                                    && let Some((id, name, args)) =
                                                        tool_use_buf.remove(&idx)
                                                {
                                                    let tool_calls = vec![ToolCall {
                                                        id,
                                                        name,
                                                        arguments: args,
                                                    }];
                                                    yield Ok(StreamEvent::ToolCallDone { tool_calls });
                                                }
                                            }
                                            "message_delta" => {
                                                let output_tokens = event
                                                    .usage
                                                    .map(|u| u.output_tokens)
                                                    .unwrap_or(0);
                                                let usage = TokenUsage {
                                                    prompt_tokens: input_tokens,
                                                    completion_tokens: output_tokens,
                                                    total_tokens: input_tokens + output_tokens,
                                                };
                                                yield Ok(StreamEvent::Done { usage: Some(usage) });
                                                return;
                                            }
                                            "message_stop" => {
                                                yield Ok(StreamEvent::Done { usage: None });
                                                return;
                                            }
                                            _ => {} // ping, etc.
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };

        Ok(Box::pin(s))
    }
}

fn map_reqwest_error(e: reqwest::Error) -> LlmError {
    if e.is_timeout() {
        return LlmError::Timeout(Duration::from_secs(30));
    }
    LlmError::Network(e.to_string())
}

fn map_http_status_error(status: u16, message: String) -> LlmError {
    match status {
        401 | 403 => LlmError::AuthError(message),
        413 => LlmError::RequestTooLarge(message),
        429 => LlmError::RateLimited {
            retry_after_ms: None,
        },
        404 => LlmError::ModelNotAvailable(message),
        _ => LlmError::ProviderError { status, message },
    }
}
