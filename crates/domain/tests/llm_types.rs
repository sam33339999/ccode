use ccode_domain::llm::{
    ContentBlock, LlmRequest, LlmResponse, Message, Role, StopReason, StreamEvent, TokenUsage,
    ToolDefinition, constants,
};
use serde_json::json;

#[test]
fn canonical_llm_types_serde_and_constants_match_contract() {
    let message = Message {
        role: Role::User,
        content: vec![
            ContentBlock::Text {
                text: "hello".to_string(),
            },
            ContentBlock::Thinking {
                thinking: "reasoning".to_string(),
            },
        ],
    };

    let tool = ToolDefinition {
        name: "read_file".to_string(),
        description: "Read a file".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        }),
    };

    let req = LlmRequest {
        model: "claude-sonnet".to_string(),
        messages: vec![message.clone()],
        tools: vec![tool.clone()],
        max_tokens: 1024,
        system: Some("System prompt".to_string()),
        stop_sequences: vec!["</stop>".to_string()],
        temperature: Some(0.2),
    };

    let usage = TokenUsage {
        input_tokens: 12,
        output_tokens: 34,
    };

    let resp = LlmResponse {
        content: vec![
            ContentBlock::ToolUse {
                id: "toolu_1".to_string(),
                name: "read_file".to_string(),
                input: json!({ "path": "README.md" }),
            },
            ContentBlock::ToolResult {
                tool_use_id: "toolu_1".to_string(),
                content: "ok".to_string(),
                is_error: false,
            },
        ],
        stop_reason: StopReason::ToolUse,
        usage,
        model: "claude-sonnet".to_string(),
    };

    let _start = StreamEvent::MessageStart;
    let _delta = StreamEvent::ContentDelta {
        index: 0,
        text: "hel".to_string(),
    };
    let _tool_delta = StreamEvent::ToolCallDelta {
        index: 1,
        id: "toolu_1".to_string(),
        name: "read_file".to_string(),
        input_json: "{\"path\":\"README.md\"}".to_string(),
    };
    let _done = StreamEvent::MessageComplete {
        stop_reason: StopReason::EndTurn,
        usage: TokenUsage {
            input_tokens: 1,
            output_tokens: 2,
        },
    };

    let serialized = serde_json::to_value(&ContentBlock::ToolUse {
        id: "t1".to_string(),
        name: "run".to_string(),
        input: json!({}),
    })
    .expect("serialize content block");
    assert_eq!(serialized["type"], "tool_use");

    // api-types constants
    assert_eq!(constants::api_types::CONTENT_BLOCK_TEXT, "text");
    assert_eq!(constants::api_types::CONTENT_BLOCK_TOOL_USE, "tool_use");
    assert_eq!(
        constants::api_types::CONTENT_BLOCK_TOOL_RESULT,
        "tool_result"
    );
    assert_eq!(constants::api_types::CONTENT_BLOCK_THINKING, "thinking");
    assert_eq!(constants::api_types::STOP_REASON_END_TURN, "end_turn");
    assert_eq!(constants::api_types::STOP_REASON_TOOL_USE, "tool_use");
    assert_eq!(constants::api_types::STOP_REASON_MAX_TOKENS, "max_tokens");
    assert_eq!(
        constants::api_types::STOP_REASON_STOP_SEQUENCE,
        "stop_sequence"
    );
    assert_eq!(constants::api_types::ROLE_USER, "user");
    assert_eq!(constants::api_types::ROLE_ASSISTANT, "assistant");
    assert_eq!(constants::api_types::ROLE_SYSTEM, "system");

    // config constants
    assert_eq!(constants::config::ENV_OPENAI_API_KEY, "OPENAI_API_KEY");
    assert_eq!(
        constants::config::ENV_ANTHROPIC_API_KEY,
        "ANTHROPIC_API_KEY"
    );
    assert_eq!(
        constants::config::MODEL_ALIASES[0],
        ("claude-sonnet", "claude-3-7-sonnet-latest")
    );

    // core-domain constants
    assert!(constants::core_domain::ROLE_SYSTEM_ALLOWED_IN_REQUEST);
    assert!(constants::core_domain::STOP_REASON_REQUIRED_ON_RESPONSE);

    let _roundtrip_req: LlmRequest =
        serde_json::from_value(serde_json::to_value(req).expect("serialize request"))
            .expect("deserialize request");
    let _roundtrip_resp: LlmResponse =
        serde_json::from_value(serde_json::to_value(resp).expect("serialize response"))
            .expect("deserialize response");
    let _roundtrip_tool: ToolDefinition =
        serde_json::from_value(serde_json::to_value(tool).expect("serialize tool"))
            .expect("deserialize tool");
}
