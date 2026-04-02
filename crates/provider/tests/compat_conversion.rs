use ccode_provider::compat::convert::{
    AnthropicToOpenAiOptions, anthropic_request_to_openai_request,
    anthropic_response_to_openai_response, openai_request_to_anthropic_request,
    openai_response_to_anthropic_response,
};
use ccode_provider::compat::request::{
    AnthropicRequest, Content, Message, OpenAiMessage, OpenAiRequest, SystemContent,
};
use ccode_provider::compat::response::{
    AnthropicResponse, OpenAiChoice, OpenAiMessageResponse, OpenAiResponse, OpenAiUsage, TextBlock,
    Usage,
};

#[test]
fn anthropic_request_converts_to_openai_request_with_system_and_thinking_option() {
    let req = AnthropicRequest {
        model: "claude-3-7-sonnet".into(),
        messages: vec![Message {
            role: "user".into(),
            content: Content::Text("Hello".into()),
        }],
        max_tokens: Some(128),
        temperature: Some(0.3),
        system: Some(SystemContent::Text("You are concise".into())),
        stream: Some(false),
        provider: None,
    };

    let out = anthropic_request_to_openai_request(
        req,
        AnthropicToOpenAiOptions {
            enable_thinking: Some(false),
        },
    );

    assert_eq!(out.model, "claude-3-7-sonnet");
    assert_eq!(out.messages.len(), 2);
    assert_eq!(out.messages[0].role, "system");
    assert_eq!(out.messages[0].content, "You are concise");
    assert_eq!(out.messages[1].role, "user");
    assert_eq!(out.messages[1].content, "Hello");
    assert_eq!(out.max_tokens, Some(128));
    assert_eq!(out.temperature, Some(0.3));
    assert_eq!(out.enable_thinking, Some(false));
    assert_eq!(out.stream, None);
}

#[test]
fn openai_request_converts_back_to_anthropic_request() {
    let req = OpenAiRequest {
        model: "gpt-4.1".into(),
        messages: vec![
            OpenAiMessage {
                role: "system".into(),
                content: "safety".into(),
            },
            OpenAiMessage {
                role: "user".into(),
                content: "Hi".into(),
            },
        ],
        max_tokens: Some(256),
        temperature: Some(0.7),
        stream: Some(true),
        enable_thinking: None,
    };

    let out = openai_request_to_anthropic_request(req);

    assert_eq!(out.model, "gpt-4.1");
    assert_eq!(out.system, Some(SystemContent::Text("safety".into())));
    assert_eq!(out.messages.len(), 1);
    assert_eq!(out.messages[0].role, "user");
    assert_eq!(out.max_tokens, Some(256));
    assert_eq!(out.temperature, Some(0.7));
    assert_eq!(out.stream, Some(true));
}

#[test]
fn openai_response_converts_to_anthropic_response() {
    let resp = OpenAiResponse {
        id: "chatcmpl-123".into(),
        object: "chat.completion".into(),
        created: 1,
        model: "gpt-4.1".into(),
        choices: vec![OpenAiChoice {
            index: 0,
            message: OpenAiMessageResponse {
                role: "assistant".into(),
                content: Some("Hello back".into()),
                reasoning_content: None,
            },
            finish_reason: Some("stop".into()),
        }],
        usage: Some(OpenAiUsage {
            prompt_tokens: 11,
            completion_tokens: 7,
            total_tokens: 18,
        }),
    };

    let out = openai_response_to_anthropic_response(resp, "sonnet-via-openai");

    assert_eq!(out.id, "chatcmpl-123");
    assert_eq!(out.response_type, "message");
    assert_eq!(out.role, "assistant");
    assert_eq!(out.model, "sonnet-via-openai");
    assert_eq!(out.stop_reason, Some("stop".into()));
    assert_eq!(out.content.len(), 1);
    assert_eq!(out.content[0].block_type, "text");
    assert_eq!(out.content[0].text, "Hello back");
    assert_eq!(out.usage.input_tokens, 11);
    assert_eq!(out.usage.output_tokens, 7);
}

#[test]
fn anthropic_response_converts_back_to_openai_response() {
    let resp = AnthropicResponse {
        id: "msg_123".into(),
        response_type: "message".into(),
        role: "assistant".into(),
        content: vec![TextBlock {
            block_type: "text".into(),
            text: "Hi".into(),
        }],
        model: "claude-3-7-sonnet".into(),
        stop_reason: Some("end_turn".into()),
        usage: Usage {
            input_tokens: 9,
            output_tokens: 3,
        },
    };

    let out = anthropic_response_to_openai_response(resp, "gpt-4.1-via-anthropic");

    assert_eq!(out.id, "msg_123");
    assert_eq!(out.object, "chat.completion");
    assert_eq!(out.model, "gpt-4.1-via-anthropic");
    assert_eq!(out.choices.len(), 1);
    assert_eq!(out.choices[0].message.content.as_deref(), Some("Hi"));
    assert_eq!(out.choices[0].finish_reason.as_deref(), Some("end_turn"));
    assert_eq!(out.usage.as_ref().map(|u| u.prompt_tokens), Some(9));
    assert_eq!(out.usage.as_ref().map(|u| u.completion_tokens), Some(3));
}
