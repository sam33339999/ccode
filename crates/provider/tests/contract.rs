//! Contract tests — llm-compat-contract.md §10.1
//!
//! Verifies serialisation round-trips, bijective ToolDefinition mapping,
//! and correct StopReason translation between Anthropic and OpenAI wire formats.
//! No HTTP is involved; all tests are pure data-transformation assertions.

use ccode_provider::compat::convert::{
    anthropic_response_to_openai_response, anthropic_tool_to_openai_function,
    openai_function_to_anthropic_tool, openai_response_to_anthropic_response,
};
use ccode_provider::compat::request::{
    AnthropicRequest, Content, Message, OpenAiMessage, OpenAiRequest, SystemContent,
};
use ccode_provider::compat::response::{
    AnthropicResponse, OpenAiChoice, OpenAiMessageResponse, OpenAiResponse, TextBlock, Usage,
};
use serde_json::json;

// ── AC1: LlmRequest round-trips through Anthropic wire format ─────────────────

#[test]
fn anthropic_request_serialises_all_llm_request_fields_without_data_loss() {
    let req = AnthropicRequest {
        model: "claude-3-5-sonnet-20241022".into(),
        messages: vec![Message {
            role: "user".into(),
            content: Content::Text("What is 2+2?".into()),
        }],
        max_tokens: Some(512),
        temperature: Some(0.7),
        system: Some(SystemContent::Text("You are a math tutor.".into())),
        stream: None,
        provider: None,
    };

    let serialised = serde_json::to_value(&req).expect("should serialise");

    assert_eq!(serialised["model"], "claude-3-5-sonnet-20241022");
    assert_eq!(serialised["messages"][0]["role"], "user");
    assert_eq!(serialised["messages"][0]["content"], "What is 2+2?");
    assert_eq!(serialised["max_tokens"], 512);
    assert_eq!(serialised["system"], "You are a math tutor.");
    assert!(
        serialised["temperature"].is_number(),
        "temperature must be present"
    );

    // Round-trip: deserialise back and compare
    let roundtrip: AnthropicRequest =
        serde_json::from_value(serialised).expect("should deserialise");
    assert_eq!(roundtrip.model, req.model);
    assert_eq!(roundtrip.max_tokens, req.max_tokens);
    assert_eq!(roundtrip.system, req.system);
    assert_eq!(roundtrip.messages.len(), 1);
}

#[test]
fn anthropic_response_deserialises_all_fields_without_data_loss() {
    let wire = json!({
        "id": "msg_abc123",
        "type": "message",
        "role": "assistant",
        "model": "claude-3-5-sonnet-20241022",
        "content": [{"type": "text", "text": "The answer is 4."}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 25, "output_tokens": 8}
    });

    let resp: AnthropicResponse = serde_json::from_value(wire).expect("should deserialise");

    assert_eq!(resp.id, "msg_abc123");
    assert_eq!(resp.model, "claude-3-5-sonnet-20241022");
    assert_eq!(resp.content.len(), 1);
    assert_eq!(resp.content[0].block_type, "text");
    assert_eq!(resp.content[0].text, "The answer is 4.");
    assert_eq!(resp.stop_reason, Some("end_turn".into()));
    assert_eq!(resp.usage.input_tokens, 25);
    assert_eq!(resp.usage.output_tokens, 8);
}

// ── AC2: LlmRequest round-trips through OpenAI wire format ────────────────────

#[test]
fn openai_request_serialises_all_llm_request_fields_without_data_loss() {
    let req = OpenAiRequest {
        model: "gpt-4o".into(),
        messages: vec![
            OpenAiMessage {
                role: "system".into(),
                content: "You are helpful.".into(),
            },
            OpenAiMessage {
                role: "user".into(),
                content: "What is 2+2?".into(),
            },
        ],
        max_tokens: Some(256),
        temperature: Some(0.5),
        stream: None,
        enable_thinking: None,
    };

    let serialised = serde_json::to_value(&req).expect("should serialise");

    assert_eq!(serialised["model"], "gpt-4o");
    assert_eq!(serialised["messages"][0]["role"], "system");
    assert_eq!(serialised["messages"][0]["content"], "You are helpful.");
    assert_eq!(serialised["messages"][1]["role"], "user");
    assert_eq!(serialised["messages"][1]["content"], "What is 2+2?");
    assert_eq!(serialised["max_tokens"], 256);
    assert!(
        serialised["temperature"].is_number(),
        "temperature must be present"
    );

    let roundtrip: OpenAiRequest = serde_json::from_value(serialised).expect("should deserialise");
    assert_eq!(roundtrip.model, req.model);
    assert_eq!(roundtrip.messages.len(), 2);
    assert_eq!(roundtrip.max_tokens, req.max_tokens);
}

#[test]
fn openai_response_deserialises_all_fields_without_data_loss() {
    let wire = json!({
        "id": "chatcmpl-abc123",
        "object": "chat.completion",
        "created": 1_700_000_000_u64,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "The answer is 4."},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 25, "completion_tokens": 8, "total_tokens": 33}
    });

    let resp: ccode_provider::compat::response::OpenAiResponse =
        serde_json::from_value(wire).expect("should deserialise");

    assert_eq!(resp.id, "chatcmpl-abc123");
    assert_eq!(resp.model, "gpt-4o");
    assert_eq!(
        resp.choices[0].message.content.as_deref(),
        Some("The answer is 4.")
    );
    assert_eq!(resp.choices[0].finish_reason.as_deref(), Some("stop"));
    let u = resp.usage.as_ref().unwrap();
    assert_eq!(u.prompt_tokens, 25);
    assert_eq!(u.completion_tokens, 8);
    assert_eq!(u.total_tokens, 33);
}

// ── AC3: ToolDefinition schema mapping is bijective ──────────────────────────

#[test]
fn tool_definition_mapping_bijective_anthropic_to_openai_and_back() {
    let anthropic_tool = json!({
        "name": "get_weather",
        "description": "Get current weather for a location",
        "input_schema": {
            "type": "object",
            "properties": {
                "location": {"type": "string"},
                "unit": {"type": "string", "enum": ["celsius", "fahrenheit"]}
            },
            "required": ["location"]
        }
    });

    let openai_fn =
        anthropic_tool_to_openai_function(anthropic_tool.clone()).expect("anthropic→openai");

    // OpenAI uses "parameters", not "input_schema"
    assert!(
        openai_fn.get("parameters").is_some(),
        "OpenAI function must have 'parameters'"
    );
    assert!(
        openai_fn.get("input_schema").is_none(),
        "OpenAI function must not have 'input_schema'"
    );
    assert_eq!(openai_fn["name"], "get_weather");
    assert_eq!(
        openai_fn["description"],
        "Get current weather for a location"
    );

    // Bijection: round-trip must equal the original
    let roundtrip =
        openai_function_to_anthropic_tool(openai_fn).expect("openai→anthropic roundtrip");
    assert_eq!(roundtrip, anthropic_tool, "round-trip must equal original");
}

#[test]
fn tool_definition_mapping_bijective_openai_to_anthropic_and_back() {
    let openai_fn = json!({
        "name": "execute_code",
        "description": "Execute code in sandbox",
        "parameters": {
            "type": "object",
            "properties": {
                "code": {"type": "string"},
                "language": {"type": "string", "enum": ["python", "javascript"]}
            },
            "required": ["code"]
        }
    });

    let anthropic_tool =
        openai_function_to_anthropic_tool(openai_fn.clone()).expect("openai→anthropic");

    // Anthropic uses "input_schema", not "parameters"
    assert!(
        anthropic_tool.get("input_schema").is_some(),
        "Anthropic tool must have 'input_schema'"
    );
    assert!(
        anthropic_tool.get("parameters").is_none(),
        "Anthropic tool must not have 'parameters'"
    );

    let roundtrip =
        anthropic_tool_to_openai_function(anthropic_tool).expect("anthropic→openai roundtrip");
    assert_eq!(roundtrip, openai_fn, "round-trip must equal original");
}

#[test]
fn tool_definition_schema_content_preserved_across_mapping() {
    let schema = json!({
        "type": "object",
        "properties": {
            "query": {"type": "string", "description": "Search query"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 100}
        },
        "required": ["query"]
    });

    let anthropic_tool = json!({
        "name": "search",
        "description": "Full-text search",
        "input_schema": schema.clone()
    });

    let openai_fn = anthropic_tool_to_openai_function(anthropic_tool).expect("convert");
    assert_eq!(
        openai_fn["parameters"], schema,
        "schema content must be preserved"
    );
}

// ── AC4: StopReason::ToolUse maps correctly ──────────────────────────────────

#[test]
fn stop_reason_tool_calls_maps_to_tool_use() {
    let openai_resp = openai_resp_fixture("tool_calls");
    let anthropic = openai_response_to_anthropic_response(openai_resp, "claude");
    assert_eq!(
        anthropic.stop_reason.as_deref(),
        Some("tool_use"),
        "OpenAI 'tool_calls' must become Anthropic 'tool_use'"
    );
}

#[test]
fn stop_reason_tool_use_maps_to_tool_calls() {
    let anthropic_resp = anthropic_resp_fixture("tool_use");
    let openai = anthropic_response_to_openai_response(anthropic_resp, "gpt-4o");
    assert_eq!(
        openai.choices[0].finish_reason.as_deref(),
        Some("tool_calls"),
        "Anthropic 'tool_use' must become OpenAI 'tool_calls'"
    );
}

#[test]
fn stop_reason_non_tool_values_pass_through_unchanged() {
    // "stop" from OpenAI should remain "stop" in Anthropic
    let openai_stop = openai_resp_fixture("stop");
    let anthropic = openai_response_to_anthropic_response(openai_stop, "claude");
    assert_eq!(anthropic.stop_reason.as_deref(), Some("stop"));

    // "end_turn" from Anthropic should remain "end_turn" in OpenAI
    let anthropic_end_turn = anthropic_resp_fixture("end_turn");
    let openai = anthropic_response_to_openai_response(anthropic_end_turn, "gpt-4o");
    assert_eq!(openai.choices[0].finish_reason.as_deref(), Some("end_turn"));
}

#[test]
fn stop_reason_mapping_is_symmetric() {
    // OpenAI → Anthropic → OpenAI must round-trip
    let original = "tool_calls";
    let via_anthropic =
        openai_response_to_anthropic_response(openai_resp_fixture(original), "claude")
            .stop_reason
            .unwrap();
    let back = anthropic_response_to_openai_response(anthropic_resp_fixture(&via_anthropic), "gpt")
        .choices[0]
        .finish_reason
        .clone()
        .unwrap();
    assert_eq!(back, original);
}

// ── Fixtures ─────────────────────────────────────────────────────────────────

fn openai_resp_fixture(stop: &str) -> OpenAiResponse {
    OpenAiResponse {
        id: "test".into(),
        object: "chat.completion".into(),
        created: 0,
        model: "gpt-4o".into(),
        choices: vec![OpenAiChoice {
            index: 0,
            message: OpenAiMessageResponse {
                role: "assistant".into(),
                content: None,
                reasoning_content: None,
            },
            finish_reason: Some(stop.into()),
        }],
        usage: None,
    }
}

fn anthropic_resp_fixture(stop: &str) -> AnthropicResponse {
    AnthropicResponse {
        id: "test".into(),
        response_type: "message".into(),
        role: "assistant".into(),
        content: vec![TextBlock {
            block_type: "text".into(),
            text: String::new(),
        }],
        model: "claude".into(),
        stop_reason: Some(stop.into()),
        usage: Usage {
            input_tokens: 1,
            output_tokens: 1,
        },
    }
}
