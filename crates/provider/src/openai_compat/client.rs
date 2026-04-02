use super::types::*;
use async_stream::stream;
use ccode_domain::message::{Role, ToolCall};
use ccode_ports::{
    PortError,
    provider::{CompletionRequest, CompletionResponse, ProviderStream, StreamEvent, TokenUsage},
};
use futures::StreamExt;
use std::collections::HashMap;

/// Shared HTTP client for OpenAI-compatible APIs.
///
/// Pass `extra_headers` to inject provider-specific headers (e.g. `X-Title`).
pub struct OpenAiCompatClient {
    client: reqwest::Client,
    pub api_key: String,
    pub base_url: String,
    pub default_model: String,
    extra_headers: Vec<(String, String)>,
}

impl OpenAiCompatClient {
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        extra_headers: Vec<(String, String)>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            base_url: base_url.into(),
            default_model: default_model.into(),
            extra_headers,
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn build_body(&self, req: &CompletionRequest, stream: bool) -> ChatRequest {
        let model = req
            .model
            .clone()
            .unwrap_or_else(|| self.default_model.clone());
        let messages = req
            .messages
            .iter()
            .map(|m| {
                // tool role：帶 tool_call_id，content 為結果
                if m.role == Role::Tool {
                    return ChatMessage {
                        role: "tool".into(),
                        content: Some(m.content.clone()),
                        tool_calls: None,
                        tool_call_id: m.tool_call_id.clone(),
                    };
                }
                // assistant role：可能帶 tool_calls（純工具呼叫時 content 為 null）
                if m.role == Role::Assistant {
                    let tool_calls = m.tool_calls.as_ref().map(|tcs| {
                        tcs.iter()
                            .map(|tc| OpenAiToolCall {
                                id: tc.id.clone(),
                                r#type: "function".into(),
                                function: OpenAiToolCallFunction {
                                    name: tc.name.clone(),
                                    arguments: tc.arguments.clone(),
                                },
                            })
                            .collect::<Vec<_>>()
                    });
                    let content = if m.content.is_empty() && tool_calls.is_some() {
                        None // 純工具呼叫，content 送 null
                    } else {
                        Some(m.content.clone())
                    };
                    return ChatMessage {
                        role: "assistant".into(),
                        content,
                        tool_calls,
                        tool_call_id: None,
                    };
                }
                // user / system
                ChatMessage {
                    role: role_str(&m.role).into(),
                    content: Some(m.content.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                }
            })
            .collect();
        let tools: Vec<OpenAiTool> = req
            .tools
            .iter()
            .map(|t| OpenAiTool {
                r#type: "function".to_string(),
                function: OpenAiToolFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect();
        let tool_choice = if tools.is_empty() {
            None
        } else {
            Some("auto".into())
        };
        ChatRequest {
            model,
            messages,
            stream,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            tools,
            tool_choice,
        }
    }

    fn add_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut b = builder.bearer_auth(&self.api_key);
        for (name, value) in &self.extra_headers {
            b = b.header(name.as_str(), value.as_str());
        }
        b
    }

    pub async fn health_check(&self) -> Result<(), PortError> {
        let url = format!("{}/models", self.base_url);
        let resp = self
            .add_headers(self.client.get(&url))
            .send()
            .await
            .map_err(|e| PortError::Provider(e.to_string()))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(PortError::Provider(format!(
                "health check failed: HTTP {}",
                resp.status()
            )))
        }
    }

    pub async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, PortError> {
        let body = self.build_body(&req, false);
        let resp = self
            .add_headers(self.client.post(self.endpoint()).json(&body))
            .send()
            .await
            .map_err(|e| PortError::Provider(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PortError::Provider(format!("HTTP {status}: {text}")));
        }

        let chat: ChatResponse = resp
            .json()
            .await
            .map_err(|e| PortError::Provider(e.to_string()))?;

        let content = chat
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message)
            .and_then(|m| m.content)
            .unwrap_or_default();

        Ok(CompletionResponse {
            content,
            model: chat.model,
            usage: chat.usage.map(map_usage),
        })
    }

    pub async fn stream_complete(
        &self,
        req: CompletionRequest,
    ) -> Result<ProviderStream, PortError> {
        let body = self.build_body(&req, true);

        // debug: 印出送出的完整 request body（RUST_LOG=debug 時可見）
        tracing::debug!(
            endpoint = %self.endpoint(),
            request_body = %serde_json::to_string_pretty(&body).unwrap_or_default(),
            "openai_compat → stream_complete request"
        );

        let resp = self
            .add_headers(self.client.post(self.endpoint()).json(&body))
            .send()
            .await
            .map_err(|e| PortError::Provider(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PortError::Provider(format!("HTTP {status}: {text}")));
        }

        let bytes_stream = resp.bytes_stream();

        let s = stream! {
            tokio::pin!(bytes_stream);
            let mut buf = String::new();
            // Map from tool_call index -> (id, name, accumulated_arguments)
            let mut tool_call_buf: HashMap<usize, (String, String, String)> = HashMap::new();

            while let Some(chunk) = bytes_stream.next().await {
                match chunk {
                    Err(e) => {
                        yield Err(PortError::Provider(e.to_string()));
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

                                    tracing::trace!(raw_sse_line = %line, "openai_compat SSE");

                                    if !line.starts_with("data:") {
                                        continue;
                                    }
                                    let data = line["data:".len()..].trim();

                                    if data == "[DONE]" {
                                        // Emit tool calls if any were buffered
                                        if !tool_call_buf.is_empty() {
                                            let mut indices: Vec<usize> = tool_call_buf.keys().cloned().collect();
                                            indices.sort();
                                            let tool_calls: Vec<ToolCall> = indices.iter().filter_map(|idx| {
                                                let (id, name, args) = tool_call_buf.get(idx)?;
                                                Some(ToolCall {
                                                    id: id.clone(),
                                                    name: name.clone(),
                                                    arguments: args.clone(),
                                                })
                                            }).collect();
                                            if !tool_calls.is_empty() {
                                                yield Ok(StreamEvent::ToolCallDone { tool_calls });
                                            }
                                        }
                                        yield Ok(StreamEvent::Done { usage: None });
                                        return;
                                    }

                                    match serde_json::from_str::<StreamChunk>(data) {
                                        Err(e) => {
                                            tracing::debug!("skip unparseable SSE chunk: {e}");
                                        }
                                        Ok(chunk) => {
                                            let usage = chunk.usage.map(map_usage);

                                            for choice in chunk.choices {
                                                // Accumulate text content
                                                if let Some(content) = choice.delta.content
                                                    && !content.is_empty()
                                                {
                                                    yield Ok(StreamEvent::Delta { content });
                                                }
                                                // Accumulate tool call deltas
                                                if let Some(tc_deltas) = choice.delta.tool_calls {
                                                    for tc_delta in tc_deltas {
                                                        let entry = tool_call_buf
                                                            .entry(tc_delta.index)
                                                            .or_insert_with(|| (String::new(), String::new(), String::new()));
                                                        if let Some(id) = tc_delta.id {
                                                            entry.0 = id;
                                                        }
                                                        if let Some(func) = tc_delta.function {
                                                            if let Some(name) = func.name {
                                                                entry.1 = name;
                                                            }
                                                            if let Some(args) = func.arguments {
                                                                entry.2.push_str(&args);
                                                            }
                                                        }
                                                    }
                                                }
                                                if choice.finish_reason.is_some() {
                                                    // Emit tool calls if any were buffered
                                                    if !tool_call_buf.is_empty() {
                                                        let mut indices: Vec<usize> = tool_call_buf.keys().cloned().collect();
                                                        indices.sort();
                                                        let tool_calls: Vec<ToolCall> = indices.iter().filter_map(|idx| {
                                                            let (id, name, args) = tool_call_buf.get(idx)?;
                                                            Some(ToolCall {
                                                                id: id.clone(),
                                                                name: name.clone(),
                                                                arguments: args.clone(),
                                                            })
                                                        }).collect();
                                                        if !tool_calls.is_empty() {
                                                            yield Ok(StreamEvent::ToolCallDone { tool_calls });
                                                        }
                                                        tool_call_buf.clear();
                                                    }
                                                    if usage.is_some() {
                                                        yield Ok(StreamEvent::Done { usage });
                                                        return;
                                                    }
                                                }
                                            }
                                        }
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

fn role_str(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn map_usage(u: UsageStats) -> TokenUsage {
    TokenUsage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
    }
}
