use super::types::*;
use async_stream::stream;
use ccode_domain::message::{AttachmentData, Role, ToolCall};
use ccode_ports::provider::{
    LlmError, LlmRequest, LlmResponse, LlmStream, StreamEvent, TokenUsage,
};
use futures::StreamExt;
use reqwest::header::HeaderMap;
use std::collections::HashMap;
use std::time::Duration;

type ToolCallBuffer = HashMap<usize, (String, String, String)>;

/// Shared HTTP client for OpenAI-compatible APIs.
///
/// Pass `extra_headers` to inject provider-specific headers (e.g. `X-Title`).
pub struct OpenAiCompatClient {
    client: reqwest::Client,
    pub api_key: String,
    pub base_url: String,
    pub default_model: String,
    extra_headers: Vec<(String, String)>,
    supports_vision: bool,
}

impl OpenAiCompatClient {
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        extra_headers: Vec<(String, String)>,
        supports_vision: bool,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            base_url: base_url.into(),
            default_model: default_model.into(),
            extra_headers,
            supports_vision,
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn build_body(&self, req: &LlmRequest, stream: bool) -> ChatRequest {
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
                        content: Some(ChatMessageContent::Text(m.content.clone())),
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
                        Some(ChatMessageContent::Text(m.content.clone()))
                    };
                    return ChatMessage {
                        role: "assistant".into(),
                        content,
                        tool_calls,
                        tool_call_id: None,
                    };
                }
                // user / system
                let content = if self.supports_vision {
                    let mut blocks = Vec::new();
                    if !m.content.is_empty() {
                        blocks.push(ChatContentBlock::Text {
                            text: m.content.clone(),
                        });
                    }
                    if let Some(attachments) = &m.attachments {
                        for attachment in attachments {
                            let url = match &attachment.data {
                                AttachmentData::Base64(data) => {
                                    format!("data:{};base64,{}", attachment.media_type, data)
                                }
                                AttachmentData::Url(url) => url.clone(),
                            };
                            blocks.push(ChatContentBlock::ImageUrl {
                                image_url: ChatImageUrl { url },
                            });
                        }
                    }
                    if blocks.is_empty() {
                        Some(ChatMessageContent::Text(m.content.clone()))
                    } else if blocks.len() == 1 {
                        match blocks.into_iter().next() {
                            Some(ChatContentBlock::Text { text }) => {
                                Some(ChatMessageContent::Text(text))
                            }
                            Some(other) => Some(ChatMessageContent::Blocks(vec![other])),
                            None => Some(ChatMessageContent::Text(m.content.clone())),
                        }
                    } else {
                        Some(ChatMessageContent::Blocks(blocks))
                    }
                } else {
                    Some(ChatMessageContent::Text(m.content.clone()))
                };
                ChatMessage {
                    role: role_str(&m.role).into(),
                    content,
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

    pub async fn health_check(&self) -> Result<(), LlmError> {
        let url = format!("{}/models", self.base_url);
        let resp = self
            .add_headers(self.client.get(&url))
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
            .add_headers(self.client.post(self.endpoint()).json(&body))
            .send()
            .await
            .map_err(map_reqwest_error)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(map_http_status_error(status.as_u16(), &headers, text));
        }

        let chat: ChatResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        let content = chat
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message)
            .and_then(|m| m.content)
            .unwrap_or_default();

        Ok(LlmResponse {
            content,
            model: chat.model,
            usage: chat.usage.map(map_usage),
        })
    }

    pub async fn stream(&self, req: LlmRequest) -> Result<LlmStream, LlmError> {
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
            .map_err(map_reqwest_error)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(map_http_status_error(status.as_u16(), &headers, text));
        }

        let bytes_stream = resp.bytes_stream();

        let s = stream! {
            tokio::pin!(bytes_stream);
            let mut buf = String::new();
            // Map from tool_call index -> (id, name, accumulated_arguments)
            let mut tool_call_buf: ToolCallBuffer = HashMap::new();

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
                                            let (events, done) = convert_chunk_to_events(chunk, &mut tool_call_buf);
                                            for event in events {
                                                yield Ok(event);
                                            }
                                            if done {
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

fn map_http_status_error(status: u16, headers: &HeaderMap, message: String) -> LlmError {
    match status {
        401 | 403 => LlmError::AuthError(message),
        413 => LlmError::RequestTooLarge(message),
        400 if is_request_too_large_message(&message) => LlmError::RequestTooLarge(message),
        429 => LlmError::RateLimited {
            retry_after_ms: retry_after_ms(headers),
        },
        404 => LlmError::ModelNotAvailable(message),
        _ => LlmError::ProviderError { status, message },
    }
}

fn is_request_too_large_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "too large",
        "request_too_large",
        "maximum context length",
        "context length",
        "token limit",
        "prompt is too long",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn retry_after_ms(headers: &HeaderMap) -> Option<u64> {
    let secs = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| raw.trim().parse::<u64>().ok())?;
    secs.checked_mul(1000)
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

fn flush_tool_calls(tool_call_buf: &mut ToolCallBuffer) -> Option<Vec<ToolCall>> {
    if tool_call_buf.is_empty() {
        return None;
    }
    let mut indices: Vec<usize> = tool_call_buf.keys().cloned().collect();
    indices.sort();
    let tool_calls: Vec<ToolCall> = indices
        .iter()
        .filter_map(|idx| {
            let (id, name, args) = tool_call_buf.get(idx)?;
            Some(ToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: args.clone(),
            })
        })
        .collect();
    tool_call_buf.clear();
    if tool_calls.is_empty() {
        None
    } else {
        Some(tool_calls)
    }
}

fn convert_chunk_to_events(
    chunk: StreamChunk,
    tool_call_buf: &mut ToolCallBuffer,
) -> (Vec<StreamEvent>, bool) {
    let mut events = Vec::new();
    let usage = chunk.usage.map(map_usage);
    let mut done = false;

    for choice in chunk.choices {
        if let Some(content) = choice.delta.content
            && !content.is_empty()
        {
            events.push(StreamEvent::Delta { content });
        }

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
            if let Some(tool_calls) = flush_tool_calls(tool_call_buf) {
                events.push(StreamEvent::ToolCallDone { tool_calls });
            }
            if usage.is_some() {
                events.push(StreamEvent::Done {
                    usage: usage.clone(),
                });
                done = true;
            }
        }
    }

    (events, done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_400_too_large_to_request_too_large() {
        let err = map_http_status_error(
            400,
            &HeaderMap::new(),
            "maximum context length exceeded".to_string(),
        );
        match err {
            LlmError::RequestTooLarge(msg) => assert!(msg.contains("context length")),
            other => panic!("expected RequestTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn extracts_retry_after_on_429() {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "12".parse().expect("valid header value"));
        let err = map_http_status_error(429, &headers, "slow down".to_string());
        match err {
            LlmError::RateLimited { retry_after_ms } => assert_eq!(retry_after_ms, Some(12_000)),
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[test]
    fn openai_stream_chunk_preserves_delta_then_tool_then_done_order() {
        let mut buf = ToolCallBuffer::new();

        let first = StreamChunk {
            choices: vec![StreamChoice {
                delta: DeltaContent {
                    content: Some("hello ".to_string()),
                    tool_calls: Some(vec![StreamToolCallDelta {
                        index: 0,
                        id: Some("call_1".to_string()),
                        function: Some(StreamToolCallFunctionDelta {
                            name: Some("lookup".to_string()),
                            arguments: Some("{\"q\":\"".to_string()),
                        }),
                    }]),
                },
                finish_reason: None,
            }],
            usage: None,
        };

        let second = StreamChunk {
            choices: vec![StreamChoice {
                delta: DeltaContent {
                    content: Some("world".to_string()),
                    tool_calls: Some(vec![StreamToolCallDelta {
                        index: 0,
                        id: None,
                        function: Some(StreamToolCallFunctionDelta {
                            name: None,
                            arguments: Some("rust\"}".to_string()),
                        }),
                    }]),
                },
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: Some(UsageStats {
                prompt_tokens: 5,
                completion_tokens: 7,
                total_tokens: 12,
            }),
        };

        let (events_1, done_1) = convert_chunk_to_events(first, &mut buf);
        let (events_2, done_2) = convert_chunk_to_events(second, &mut buf);

        assert!(!done_1);
        assert!(done_2);
        assert!(matches!(&events_1[0], StreamEvent::Delta { content } if content == "hello "));
        assert!(matches!(&events_2[0], StreamEvent::Delta { content } if content == "world"));
        assert!(
            matches!(&events_2[1], StreamEvent::ToolCallDone { tool_calls } if tool_calls.len() == 1 && tool_calls[0].arguments == "{\"q\":\"rust\"}")
        );
        assert!(matches!(&events_2[2], StreamEvent::Done { usage: Some(_) }));
    }
}
