use super::request::{
    AnthropicRequest, Content, Message, OpenAiMessage, OpenAiRequest, SystemContent,
};
use super::response::{
    AnthropicResponse, OpenAiChoice, OpenAiMessageResponse, OpenAiResponse, OpenAiUsage, TextBlock,
    Usage,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct AnthropicToOpenAiOptions {
    pub enable_thinking: Option<bool>,
}

pub fn anthropic_request_to_openai_request(
    anthropic_req: AnthropicRequest,
    options: AnthropicToOpenAiOptions,
) -> OpenAiRequest {
    let mut messages: Vec<OpenAiMessage> = anthropic_req
        .messages
        .into_iter()
        .map(|msg| OpenAiMessage {
            role: msg.role,
            content: content_to_text(msg.content),
        })
        .collect();

    if let Some(system) = &anthropic_req.system {
        messages.insert(
            0,
            OpenAiMessage {
                role: "system".to_string(),
                content: system.to_text(),
            },
        );
    }

    OpenAiRequest {
        model: anthropic_req.model,
        messages,
        max_tokens: anthropic_req.max_tokens,
        temperature: anthropic_req.temperature,
        stream: None,
        enable_thinking: options.enable_thinking,
    }
}

pub fn openai_request_to_anthropic_request(openai_req: OpenAiRequest) -> AnthropicRequest {
    let mut system: Option<SystemContent> = None;
    let mut messages = Vec::with_capacity(openai_req.messages.len());

    for message in openai_req.messages {
        if message.role == "system" && system.is_none() {
            system = Some(SystemContent::Text(message.content));
            continue;
        }

        messages.push(Message {
            role: message.role,
            content: Content::Text(message.content),
        });
    }

    AnthropicRequest {
        model: openai_req.model,
        messages,
        max_tokens: openai_req.max_tokens,
        temperature: openai_req.temperature,
        system,
        stream: openai_req.stream,
        provider: None,
    }
}

pub fn openai_response_to_anthropic_response(
    openai_resp: OpenAiResponse,
    original_model: impl Into<String>,
) -> AnthropicResponse {
    let stop_reason = openai_resp
        .choices
        .first()
        .and_then(|choice| choice.finish_reason.clone());

    let content = openai_resp
        .choices
        .into_iter()
        .filter_map(|choice| {
            let text = choice.message.content.filter(|s| !s.is_empty());
            if text.is_none() {
                if choice.message.reasoning_content.is_some() {
                    tracing::warn!(
                        "thinking model returned empty content (all tokens consumed by reasoning)"
                    );
                }
                return None;
            }

            Some(TextBlock {
                block_type: "text".to_string(),
                text: text.expect("content checked is_some"),
            })
        })
        .collect();

    let usage = openai_resp
        .usage
        .map(|usage| Usage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
        })
        .unwrap_or(Usage {
            input_tokens: 0,
            output_tokens: 0,
        });

    AnthropicResponse {
        id: openai_resp.id,
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        content,
        model: original_model.into(),
        stop_reason,
        usage,
    }
}

pub fn anthropic_response_to_openai_response(
    anthropic_resp: AnthropicResponse,
    target_model: impl Into<String>,
) -> OpenAiResponse {
    let text = anthropic_resp
        .content
        .into_iter()
        .filter(|block| block.block_type == "text")
        .map(|block| block.text)
        .collect::<Vec<_>>()
        .join("\n");

    OpenAiResponse {
        id: anthropic_resp.id,
        object: "chat.completion".to_string(),
        created: 0,
        model: target_model.into(),
        choices: vec![OpenAiChoice {
            index: 0,
            message: OpenAiMessageResponse {
                role: anthropic_resp.role,
                content: Some(text),
                reasoning_content: None,
            },
            finish_reason: anthropic_resp.stop_reason,
        }],
        usage: Some(OpenAiUsage {
            prompt_tokens: anthropic_resp.usage.input_tokens,
            completion_tokens: anthropic_resp.usage.output_tokens,
            total_tokens: anthropic_resp.usage.input_tokens + anthropic_resp.usage.output_tokens,
        }),
    }
}

fn content_to_text(content: Content) -> String {
    match content {
        Content::Text(text) => text,
        Content::Blocks(blocks) => blocks
            .into_iter()
            .filter_map(|block| block.text)
            .collect::<Vec<_>>()
            .join("\n"),
    }
}
