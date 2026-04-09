//! Integration tests — llm-compat-contract.md §10.1, §10.2, §10.3
//!
//! Uses wiremock to mock HTTP endpoints.  Tests verify:
//!   AC1/AC2  — LlmRequest maps to the correct Anthropic / OpenAI wire format
//!   AC6/AC7  — complete() sends a valid request and parses the response
//!   AC8/AC9  — stream() yields the correct StreamEvent sequence
//!   AC10     — tool-use flow (request → tool_use → tool_result → final answer)
//!   AC11–AC15 — HTTP error codes map to the correct LlmError variant
//!   AC17     — provider switching requires zero code changes in the caller

use std::time::Duration;

use ccode_config::schema::{
    AnthropicConfig, Config, GeminiConfig, LlamaCppConfig, OpenAiConfig, ZhipuConfig,
};
use ccode_domain::message::{Attachment, AttachmentData, Message, Role};
use ccode_ports::provider::{LlmClient, LlmError, LlmRequest, StreamEvent, ToolDefinition};
use ccode_provider::anthropic::AnthropicAdapter;
use ccode_provider::factory;
use ccode_provider::gemini::GeminiAdapter;
use ccode_provider::llamacpp::LlamaCppAdapter;
use ccode_provider::openai::OpenAiAdapter;
use ccode_provider::openrouter::OpenRouterAdapter;
use ccode_provider::zhipu::ZhipuAdapter;
use futures::StreamExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── SSE / response body helpers ───────────────────────────────────────────────

fn anthropic_ok_body(text: &str) -> serde_json::Value {
    serde_json::json!({
        "model": "claude-3-5-sonnet-20241022",
        "content": [{"type": "text", "text": text}],
        "usage": {"input_tokens": 10, "output_tokens": 5}
    })
}

fn openai_ok_body(text: &str) -> serde_json::Value {
    serde_json::json!({
        "model": "gpt-4o",
        "choices": [{"message": {"role": "assistant", "content": text}}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    })
}

/// Build a complete Anthropic SSE body that streams `text` as a text block.
fn anthropic_text_sse(text: &str) -> String {
    let events = [
        serde_json::json!({"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":0}}}),
        serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text"}}),
        serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
        serde_json::json!({"type":"content_block_stop","index":0}),
        serde_json::json!({"type":"message_delta","usage":{"output_tokens":5}}),
    ];
    events
        .iter()
        .map(|e| format!("data: {}\n\n", serde_json::to_string(e).unwrap()))
        .collect()
}

/// Build a complete OpenAI SSE body that streams `text` as a content delta.
fn openai_text_sse(text: &str) -> String {
    let c1 = serde_json::json!({"choices":[{"delta":{"role":"assistant","content":text},"finish_reason":null}]});
    let c2 = serde_json::json!({"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}});
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        serde_json::to_string(&c1).unwrap(),
        serde_json::to_string(&c2).unwrap()
    )
}

/// Anthropic SSE body that emits a single tool_use block.
fn anthropic_tool_call_sse() -> String {
    let events = [
        serde_json::json!({"type":"message_start","message":{"usage":{"input_tokens":20,"output_tokens":0}}}),
        serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_abc","name":"search"}}),
        // Send entire tool input in one chunk to avoid JSON-escape complexity
        serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":r#"{"query":"rust"}"#}}),
        serde_json::json!({"type":"content_block_stop","index":0}),
        serde_json::json!({"type":"message_delta","usage":{"output_tokens":15}}),
    ];
    events
        .iter()
        .map(|e| format!("data: {}\n\n", serde_json::to_string(e).unwrap()))
        .collect()
}

fn simple_request() -> LlmRequest {
    LlmRequest {
        messages: vec![Message::new("u1", Role::User, "Hello", 0)],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    }
}

// ── AC1: Anthropic wire format matches LlmRequest fields ─────────────────────

#[tokio::test]
async fn anthropic_adapter_sends_correct_wire_format() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_ok_body("hi")))
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("test-key", server.uri(), "claude-3-5-sonnet-20241022");
    let req = LlmRequest {
        messages: vec![
            Message::new("s1", Role::System, "You are helpful.", 0),
            Message::new("u1", Role::User, "What is 2+2?", 0),
        ],
        model: Some("claude-3-5-sonnet-20241022".into()),
        max_tokens: Some(128),
        temperature: Some(0.5),
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();

    // Anthropic wire format: system extracted to top-level field
    assert_eq!(
        body["model"], "claude-3-5-sonnet-20241022",
        "model preserved"
    );
    assert_eq!(body["system"], "You are helpful.", "system at top-level");
    assert_eq!(body["max_tokens"], 128, "max_tokens preserved");
    assert!(body["temperature"].is_number(), "temperature preserved");
    // Only the user message remains in the messages array
    assert_eq!(
        body["messages"].as_array().unwrap().len(),
        1,
        "only non-system messages in array"
    );
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "What is 2+2?");

    // Auth headers
    let hdrs = &received[0].headers;
    assert_eq!(hdrs.get("x-api-key").unwrap(), "test-key");
    assert!(
        hdrs.get("anthropic-version").is_some(),
        "anthropic-version header required"
    );
}

// ── AC2: OpenAI wire format matches LlmRequest fields ────────────────────────

#[tokio::test]
async fn openai_adapter_sends_correct_wire_format() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_ok_body("hi")))
        .mount(&server)
        .await;

    let adapter =
        OpenRouterAdapter::new("test-key", server.uri(), "gpt-4o", None, None, false, None);
    let req = LlmRequest {
        messages: vec![
            Message::new("s1", Role::System, "You are helpful.", 0),
            Message::new("u1", Role::User, "What is 2+2?", 0),
        ],
        model: Some("gpt-4o".into()),
        max_tokens: Some(128),
        temperature: Some(0.5),
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();

    // OpenAI wire format: system stays in messages array
    assert_eq!(body["model"], "gpt-4o", "model preserved");
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2, "both messages preserved");
    assert_eq!(msgs[0]["role"], "system", "system role preserved");
    assert_eq!(
        msgs[0]["content"], "You are helpful.",
        "system content preserved"
    );
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[1]["content"], "What is 2+2?");
    assert_eq!(body["max_tokens"], 128);

    // Bearer auth
    let auth = received[0]
        .headers
        .get("authorization")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(auth.starts_with("Bearer "), "must use Bearer auth");
}

#[test]
fn openrouter_adapter_capabilities_follow_config() {
    let adapter = OpenRouterAdapter::new(
        "test-key",
        "http://example.com",
        "gpt-4o",
        None,
        None,
        true,
        Some(123_456),
    );

    let caps = adapter.capabilities();
    assert!(caps.vision);
    assert_eq!(caps.context_window, Some(123_456));
}

#[test]
fn anthropic_adapter_capabilities_follow_config() {
    let adapter = AnthropicAdapter::new_with_capabilities(
        "test-key",
        "http://example.com",
        "claude-3-5-sonnet-20241022",
        true,
        Some(123_456),
    );

    let caps = adapter.capabilities();
    assert!(caps.vision);
    assert_eq!(caps.context_window, Some(123_456));
}

#[test]
fn anthropic_factory_passes_capabilities_from_config() {
    let mut config = Config::default();
    config.providers.anthropic = Some(AnthropicConfig {
        api_key: Some("test-key".to_string()),
        default_model: Some("claude-3-5-sonnet-20241022".to_string()),
        base_url: Some("http://example.com".to_string()),
        vision: Some(true),
        context_window: Some(200_000),
    });

    let client = factory::build("anthropic", &config).expect("anthropic client should build");
    let caps = client.capabilities();
    assert!(caps.vision);
    assert_eq!(caps.context_window, Some(200_000));
}

#[test]
fn openai_adapter_capabilities_follow_config() {
    let adapter = OpenAiAdapter::new(
        "test-key",
        "http://example.com",
        "gpt-4o",
        true,
        Some(123_456),
    );

    let caps = adapter.capabilities();
    assert!(caps.vision);
    assert_eq!(caps.context_window, Some(123_456));
}

#[test]
fn openai_factory_passes_capabilities_from_config() {
    let mut config = Config::default();
    config.providers.openai = Some(OpenAiConfig {
        api_key: Some("test-key".to_string()),
        default_model: Some("gpt-4o".to_string()),
        vision: Some(true),
        context_window: Some(200_000),
    });

    let client = factory::build("openai", &config).expect("openai client should build");
    let caps = client.capabilities();
    assert!(caps.vision);
    assert_eq!(caps.context_window, Some(200_000));
}

#[test]
fn zhipu_adapter_capabilities_follow_config() {
    let adapter = ZhipuAdapter::new(
        "test-key",
        "http://example.com",
        "glm-4.6v",
        None,
        true,
        Some(123_456),
    );

    let caps = adapter.capabilities();
    assert!(caps.vision);
    assert_eq!(caps.context_window, Some(123_456));
}

#[test]
fn zhipu_factory_passes_capabilities_from_config() {
    let mut config = Config::default();
    config.providers.zhipu = Some(ZhipuConfig {
        api_key: Some("test-key".to_string()),
        default_model: Some("glm-4.6v".to_string()),
        base_url: Some("http://example.com".to_string()),
        title: None,
        vision: Some(true),
        context_window: Some(128_000),
    });

    let client = factory::build("zhipu", &config).expect("zhipu client should build");
    let caps = client.capabilities();
    assert!(caps.vision);
    assert_eq!(caps.context_window, Some(128_000));
}

#[test]
fn llamacpp_adapter_capabilities_follow_config() {
    let adapter = LlamaCppAdapter::new(
        "test-key",
        "http://example.com",
        "llava-1.6",
        true,
        Some(16_384),
    );

    let caps = adapter.capabilities();
    assert!(caps.vision);
    assert_eq!(caps.context_window, Some(16_384));
}

#[test]
fn llamacpp_factory_passes_capabilities_from_config() {
    let mut config = Config::default();
    config.providers.llamacpp = Some(LlamaCppConfig {
        api_key: Some("test-key".to_string()),
        default_model: Some("llava-1.6".to_string()),
        base_url: Some("http://example.com".to_string()),
        vision: Some(true),
        context_window: Some(16_384),
    });

    let client = factory::build("llamacpp", &config).expect("llamacpp client should build");
    let caps = client.capabilities();
    assert!(caps.vision);
    assert_eq!(caps.context_window, Some(16_384));
}

#[test]
fn gemini_adapter_capabilities_follow_config() {
    let adapter = GeminiAdapter::new(
        "test-key",
        "http://example.com/v1beta/openai",
        "gemini-2.5-pro",
        true,
        Some(1_048_576),
    );

    let caps = adapter.capabilities();
    assert!(caps.vision);
    assert_eq!(caps.context_window, Some(1_048_576));
}

#[test]
fn gemini_factory_passes_capabilities_from_config() {
    let mut config = Config::default();
    config.providers.gemini = Some(GeminiConfig {
        api_key: Some("test-key".to_string()),
        default_model: Some("gemini-2.5-pro".to_string()),
        base_url: Some("http://example.com/v1beta/openai".to_string()),
        vision: Some(true),
        context_window: Some(1_048_576),
    });

    let client = factory::build("gemini", &config).expect("gemini client should build");
    let caps = client.capabilities();
    assert!(caps.vision);
    assert_eq!(caps.context_window, Some(1_048_576));
}

#[tokio::test]
async fn openai_adapter_serializes_base64_image_as_image_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_ok_body("ok")))
        .mount(&server)
        .await;

    let adapter = OpenAiAdapter::new("key", server.uri(), "gpt-4o", true, None);
    let mut user = Message::new("u1", Role::User, "Describe this image", 0);
    user.attachments = Some(vec![Attachment {
        media_type: "image/png".to_string(),
        data: AttachmentData::Base64("aGVsbG8=".to_string()),
    }]);
    let req = LlmRequest {
        messages: vec![user],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        "Describe this image"
    );
    assert_eq!(body["messages"][0]["content"][1]["type"], "image_url");
    assert_eq!(
        body["messages"][0]["content"][1]["image_url"]["url"],
        "data:image/png;base64,aGVsbG8="
    );
}

#[tokio::test]
async fn openai_adapter_serializes_url_image_as_image_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_ok_body("ok")))
        .mount(&server)
        .await;

    let adapter = OpenAiAdapter::new("key", server.uri(), "gpt-4o", true, None);
    let mut user = Message::new("u1", Role::User, "What is in this image?", 0);
    user.attachments = Some(vec![Attachment {
        media_type: "image/jpeg".to_string(),
        data: AttachmentData::Url("https://example.com/image.jpg".to_string()),
    }]);
    let req = LlmRequest {
        messages: vec![user],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        "What is in this image?"
    );
    assert_eq!(body["messages"][0]["content"][1]["type"], "image_url");
    assert_eq!(
        body["messages"][0]["content"][1]["image_url"]["url"],
        "https://example.com/image.jpg"
    );
}

#[tokio::test]
async fn openai_adapter_ignores_attachments_when_vision_disabled() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_ok_body("ok")))
        .mount(&server)
        .await;

    let adapter = OpenAiAdapter::new("key", server.uri(), "gpt-4o", false, None);
    let mut user = Message::new("u1", Role::User, "plain text only", 0);
    user.attachments = Some(vec![Attachment {
        media_type: "image/png".to_string(),
        data: AttachmentData::Base64("aGVsbG8=".to_string()),
    }]);
    let req = LlmRequest {
        messages: vec![user],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["messages"][0]["content"], "plain text only");
}

#[tokio::test]
async fn openrouter_adapter_serializes_base64_image_as_image_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_ok_body("ok")))
        .mount(&server)
        .await;

    let adapter = OpenRouterAdapter::new("key", server.uri(), "gpt-4o", None, None, true, None);
    let mut user = Message::new("u1", Role::User, "Describe this image", 0);
    user.attachments = Some(vec![Attachment {
        media_type: "image/png".to_string(),
        data: AttachmentData::Base64("aGVsbG8=".to_string()),
    }]);
    let req = LlmRequest {
        messages: vec![user],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        "Describe this image"
    );
    assert_eq!(body["messages"][0]["content"][1]["type"], "image_url");
    assert_eq!(
        body["messages"][0]["content"][1]["image_url"]["url"],
        "data:image/png;base64,aGVsbG8="
    );
}

#[tokio::test]
async fn openrouter_adapter_serializes_url_image_as_image_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_ok_body("ok")))
        .mount(&server)
        .await;

    let adapter = OpenRouterAdapter::new("key", server.uri(), "gpt-4o", None, None, true, None);
    let mut user = Message::new("u1", Role::User, "What is in this image?", 0);
    user.attachments = Some(vec![Attachment {
        media_type: "image/jpeg".to_string(),
        data: AttachmentData::Url("https://example.com/image.jpg".to_string()),
    }]);
    let req = LlmRequest {
        messages: vec![user],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        "What is in this image?"
    );
    assert_eq!(body["messages"][0]["content"][1]["type"], "image_url");
    assert_eq!(
        body["messages"][0]["content"][1]["image_url"]["url"],
        "https://example.com/image.jpg"
    );
}

#[tokio::test]
async fn openrouter_adapter_ignores_attachments_when_vision_disabled() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_ok_body("ok")))
        .mount(&server)
        .await;

    let adapter = OpenRouterAdapter::new("key", server.uri(), "gpt-4o", None, None, false, None);
    let mut user = Message::new("u1", Role::User, "plain text only", 0);
    user.attachments = Some(vec![Attachment {
        media_type: "image/png".to_string(),
        data: AttachmentData::Base64("aGVsbG8=".to_string()),
    }]);
    let req = LlmRequest {
        messages: vec![user],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["messages"][0]["content"], "plain text only");
}

#[tokio::test]
async fn llamacpp_adapter_serializes_base64_image_as_image_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_ok_body("ok")))
        .mount(&server)
        .await;

    let adapter = LlamaCppAdapter::new("key", server.uri(), "llava-1.6", true, None);
    let mut user = Message::new("u1", Role::User, "Describe this image", 0);
    user.attachments = Some(vec![Attachment {
        media_type: "image/png".to_string(),
        data: AttachmentData::Base64("aGVsbG8=".to_string()),
    }]);
    let req = LlmRequest {
        messages: vec![user],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        "Describe this image"
    );
    assert_eq!(body["messages"][0]["content"][1]["type"], "image_url");
    assert_eq!(
        body["messages"][0]["content"][1]["image_url"]["url"],
        "data:image/png;base64,aGVsbG8="
    );
}

#[tokio::test]
async fn llamacpp_adapter_ignores_attachments_when_vision_disabled() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_ok_body("ok")))
        .mount(&server)
        .await;

    let adapter = LlamaCppAdapter::new("key", server.uri(), "llava-1.6", false, None);
    let mut user = Message::new("u1", Role::User, "plain text only", 0);
    user.attachments = Some(vec![Attachment {
        media_type: "image/png".to_string(),
        data: AttachmentData::Base64("aGVsbG8=".to_string()),
    }]);
    let req = LlmRequest {
        messages: vec![user],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["messages"][0]["content"], "plain text only");
}

#[tokio::test]
async fn zhipu_adapter_serializes_url_image_as_image_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_ok_body("ok")))
        .mount(&server)
        .await;

    let adapter = ZhipuAdapter::new("key", server.uri(), "glm-4.6v", None, true, None);
    let mut user = Message::new("u1", Role::User, "What is in this image?", 0);
    user.attachments = Some(vec![Attachment {
        media_type: "image/jpeg".to_string(),
        data: AttachmentData::Url("https://example.com/image.jpg".to_string()),
    }]);
    let req = LlmRequest {
        messages: vec![user],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        "What is in this image?"
    );
    assert_eq!(body["messages"][0]["content"][1]["type"], "image_url");
    assert_eq!(
        body["messages"][0]["content"][1]["image_url"]["url"],
        "https://example.com/image.jpg"
    );
}

#[tokio::test]
async fn zhipu_adapter_ignores_attachments_when_vision_disabled() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_ok_body("ok")))
        .mount(&server)
        .await;

    let adapter = ZhipuAdapter::new("key", server.uri(), "glm-4.6v", None, false, None);
    let mut user = Message::new("u1", Role::User, "plain text only", 0);
    user.attachments = Some(vec![Attachment {
        media_type: "image/png".to_string(),
        data: AttachmentData::Base64("aGVsbG8=".to_string()),
    }]);
    let req = LlmRequest {
        messages: vec![user],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["messages"][0]["content"], "plain text only");
}

#[tokio::test]
async fn anthropic_adapter_serializes_base64_image_as_native_image_block() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_ok_body("ok")))
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new_with_capabilities(
        "key",
        server.uri(),
        "claude-3-5-sonnet-20241022",
        true,
        None,
    );
    let mut user = Message::new("u1", Role::User, "Describe this image", 0);
    user.attachments = Some(vec![Attachment {
        media_type: "image/png".to_string(),
        data: AttachmentData::Base64("aGVsbG8=".to_string()),
    }]);
    let req = LlmRequest {
        messages: vec![user],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        "Describe this image"
    );
    assert_eq!(body["messages"][0]["content"][1]["type"], "image");
    assert_eq!(
        body["messages"][0]["content"][1]["source"],
        serde_json::json!({
            "type": "base64",
            "media_type": "image/png",
            "data": "aGVsbG8="
        })
    );
}

#[tokio::test]
async fn anthropic_adapter_rejects_url_image_source() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_ok_body("ok")))
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new_with_capabilities(
        "key",
        server.uri(),
        "claude-3-5-sonnet-20241022",
        true,
        None,
    );
    let mut user = Message::new("u1", Role::User, "What is in this image?", 0);
    user.attachments = Some(vec![Attachment {
        media_type: "image/jpeg".to_string(),
        data: AttachmentData::Url("https://example.com/image.jpg".to_string()),
    }]);
    let req = LlmRequest {
        messages: vec![user],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    let err = adapter
        .complete(req)
        .await
        .expect_err("url image source should be rejected");
    match err {
        LlmError::InvalidResponse(msg) => {
            assert_eq!(msg, "Anthropic does not support image URL source")
        }
        other => panic!("expected InvalidResponse, got {other:?}"),
    }

    let received = server.received_requests().await.unwrap();
    assert!(
        received.is_empty(),
        "request should fail before network call when URL image is provided"
    );
}

#[tokio::test]
async fn anthropic_adapter_ignores_attachments_when_vision_disabled() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_ok_body("ok")))
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("key", server.uri(), "claude-3-5-sonnet-20241022");
    let mut user = Message::new("u1", Role::User, "plain text only", 0);
    user.attachments = Some(vec![Attachment {
        media_type: "image/png".to_string(),
        data: AttachmentData::Base64("aGVsbG8=".to_string()),
    }]);
    let req = LlmRequest {
        messages: vec![user],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    adapter.complete(req).await.expect("should succeed");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["messages"][0]["content"], "plain text only");
}

// ── AC6: AnthropicAdapter::complete() ────────────────────────────────────────

#[tokio::test]
async fn anthropic_adapter_complete_parses_response_correctly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(anthropic_ok_body("The answer is 4.")),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("key", server.uri(), "claude-3-5-sonnet-20241022");
    let resp = adapter
        .complete(simple_request())
        .await
        .expect("should succeed");

    assert_eq!(resp.content, "The answer is 4.");
    assert!(!resp.model.is_empty(), "model should be populated");
    let usage = resp.usage.expect("usage should be present");
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
    assert_eq!(usage.total_tokens, 15);
}

// ── AC7: OpenAiAdapter::complete() ───────────────────────────────────────────

#[tokio::test]
async fn openai_adapter_complete_parses_response_correctly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_ok_body("The answer is 4.")))
        .mount(&server)
        .await;

    let adapter = OpenRouterAdapter::new("key", server.uri(), "gpt-4o", None, None, false, None);
    let resp = adapter
        .complete(simple_request())
        .await
        .expect("should succeed");

    assert_eq!(resp.content, "The answer is 4.");
    assert!(!resp.model.is_empty());
}

// ── AC8: AnthropicAdapter::stream() ──────────────────────────────────────────

#[tokio::test]
async fn anthropic_adapter_stream_yields_ordered_delta_then_done() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(anthropic_text_sse("Hello World")),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("key", server.uri(), "claude-3-5-sonnet-20241022");
    let mut stream = adapter
        .stream(simple_request())
        .await
        .expect("stream should start");

    let mut content = String::new();
    let mut events: Vec<&str> = Vec::new(); // "delta" | "done"

    while let Some(event) = stream.next().await {
        match event.expect("stream event should be Ok") {
            StreamEvent::Delta { content: c } => {
                content.push_str(&c);
                events.push("delta");
            }
            StreamEvent::ToolCallDone { .. } => panic!("unexpected ToolCallDone"),
            StreamEvent::Done { usage } => {
                assert!(
                    usage.is_some(),
                    "Done from message_delta must include usage"
                );
                events.push("done");
            }
        }
    }

    assert_eq!(
        content, "Hello World",
        "content must be accumulated correctly"
    );
    assert!(
        events.last() == Some(&"done"),
        "Done must be the last event"
    );
    assert!(
        events.iter().all(|&e| e == "delta" || e == "done"),
        "only Delta and Done events expected"
    );
}

// ── AC9: OpenAiAdapter::stream() ─────────────────────────────────────────────

#[tokio::test]
async fn openai_adapter_stream_yields_ordered_delta_then_done() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(openai_text_sse("Hello World")),
        )
        .mount(&server)
        .await;

    let adapter = OpenRouterAdapter::new("key", server.uri(), "gpt-4o", None, None, false, None);
    let mut stream = adapter
        .stream(simple_request())
        .await
        .expect("stream should start");

    let mut content = String::new();
    let mut done_received = false;

    while let Some(event) = stream.next().await {
        match event.expect("stream event should be Ok") {
            StreamEvent::Delta { content: c } => content.push_str(&c),
            StreamEvent::ToolCallDone { .. } => panic!("unexpected ToolCallDone"),
            StreamEvent::Done { .. } => done_received = true,
        }
    }

    assert_eq!(content, "Hello World");
    assert!(done_received, "Done event must be received");
}

// ── AC10: Tool use flow (request → tool_use → tool_result → final answer) ────

#[tokio::test]
async fn anthropic_tool_use_flow_request_tool_result_final_answer() {
    let server = MockServer::start().await;

    // First response (tool call) — mounted first so it's tried first; consumed once.
    // wiremock matches mocks in FIFO order; exhausted mocks fall through to the next.
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(anthropic_tool_call_sse()),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Second response (follow-up after tool result) — mounted second, serves as fallback.
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(anthropic_text_sse("The result is 42.")),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("key", server.uri(), "claude-3-5-sonnet-20241022");
    let tools = vec![ToolDefinition {
        name: "search".into(),
        description: "Search documents".into(),
        parameters: serde_json::json!({"type":"object","properties":{"query":{"type":"string"}}}),
    }];

    // Step 1: initial request with tools → receive tool call
    let req1 = LlmRequest {
        messages: vec![Message::new("u1", Role::User, "Search for Rust", 0)],
        model: None,
        max_tokens: Some(128),
        temperature: None,
        tools: tools.clone(),
    };

    let mut stream1 = adapter.stream(req1).await.expect("stream1 should start");
    let mut tool_calls = Vec::new();
    while let Some(event) = stream1.next().await {
        if let Ok(StreamEvent::ToolCallDone { tool_calls: tcs }) = event {
            tool_calls.extend(tcs);
        }
    }

    assert_eq!(tool_calls.len(), 1, "should receive exactly one tool call");
    assert_eq!(tool_calls[0].id, "call_abc");
    assert_eq!(tool_calls[0].name, "search");
    assert!(
        tool_calls[0].arguments.contains("rust"),
        "arguments must include query"
    );

    // Step 2: build follow-up request with tool result (only canonical types)
    let mut asst_msg = Message::new("a1", Role::Assistant, "", 0);
    asst_msg.tool_calls = Some(tool_calls.clone());
    let tool_result_msg = Message::new_tool_result("t1", &tool_calls[0].id, "42 results found", 0);

    let req2 = LlmRequest {
        messages: vec![
            Message::new("u1", Role::User, "Search for Rust", 0),
            asst_msg,
            tool_result_msg,
        ],
        model: None,
        max_tokens: Some(128),
        temperature: None,
        tools,
    };

    let mut stream2 = adapter.stream(req2).await.expect("stream2 should start");
    let mut final_content = String::new();
    while let Some(event) = stream2.next().await {
        if let Ok(StreamEvent::Delta { content }) = event {
            final_content.push_str(&content);
        }
    }

    assert_eq!(final_content, "The result is 42.");
}

// ── AC11: 401/403 → LlmError::AuthError ──────────────────────────────────────

#[tokio::test]
async fn status_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("wrong-key", server.uri(), "model");
    let err = adapter.complete(simple_request()).await.unwrap_err();
    assert!(
        matches!(err, LlmError::AuthError(_)),
        "401 must map to AuthError, got: {err:?}"
    );
}

#[tokio::test]
async fn status_403_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(403).set_body_string("Forbidden"))
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("key", server.uri(), "model");
    let err = adapter.complete(simple_request()).await.unwrap_err();
    assert!(
        matches!(err, LlmError::AuthError(_)),
        "403 must map to AuthError, got: {err:?}"
    );
}

// ── AC12: 429 → LlmError::RateLimited ────────────────────────────────────────

#[tokio::test]
async fn status_429_with_retry_after_header_maps_to_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "5")
                .set_body_string("Too Many Requests"),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("key", server.uri(), "model");
    let err = adapter.complete(simple_request()).await.unwrap_err();
    match err {
        LlmError::RateLimited { retry_after_ms } => {
            assert_eq!(retry_after_ms, Some(5_000), "retry_after_ms should be 5000");
        }
        other => panic!("expected RateLimited, got: {other:?}"),
    }
}

#[tokio::test]
async fn status_429_without_retry_after_has_none_retry_delay() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(429).set_body_string("Too Many Requests"))
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("key", server.uri(), "model");
    let err = adapter.complete(simple_request()).await.unwrap_err();
    match err {
        LlmError::RateLimited { retry_after_ms } => {
            assert_eq!(retry_after_ms, None, "absent retry-after header → None");
        }
        other => panic!("expected RateLimited, got: {other:?}"),
    }
}

// ── AC13: 413/400-too-large → LlmError::RequestTooLarge ─────────────────────

#[tokio::test]
async fn status_413_maps_to_request_too_large() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(413).set_body_string("Request Too Large"))
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("key", server.uri(), "model");
    let err = adapter.complete(simple_request()).await.unwrap_err();
    assert!(
        matches!(err, LlmError::RequestTooLarge(_)),
        "413 must map to RequestTooLarge, got: {err:?}"
    );
}

#[tokio::test]
async fn status_400_with_context_length_message_maps_to_request_too_large() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string("This request exceeds the maximum context length"),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("key", server.uri(), "model");
    let err = adapter.complete(simple_request()).await.unwrap_err();
    assert!(
        matches!(err, LlmError::RequestTooLarge(_)),
        "400 with 'context length' must map to RequestTooLarge, got: {err:?}"
    );
}

// ── AC14: Network timeout → LlmError::Timeout ────────────────────────────────

#[tokio::test]
async fn network_timeout_maps_to_timeout_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;

    let http = reqwest::Client::builder()
        .timeout(Duration::from_millis(100))
        .build()
        .expect("build http client");

    let adapter = AnthropicAdapter::new_for_test(http, "key", server.uri(), "model");
    let err = adapter.complete(simple_request()).await.unwrap_err();
    assert!(
        matches!(err, LlmError::Timeout(_)),
        "timeout must map to LlmError::Timeout, got: {err:?}"
    );
}

// ── AC15: Malformed response body → LlmError::InvalidResponse ────────────────

#[tokio::test]
async fn malformed_response_body_maps_to_invalid_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not valid json {{{"))
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("key", server.uri(), "model");
    let err = adapter.complete(simple_request()).await.unwrap_err();
    assert!(
        matches!(err, LlmError::InvalidResponse(_)),
        "malformed body must map to InvalidResponse, got: {err:?}"
    );
}

// ── AC17: Switching provider requires zero code changes ──────────────────────

/// A generic single-turn function that works with *any* LlmClient.
/// app-services code looks exactly like this — no provider types visible.
async fn run_one_turn(
    client: &(impl ccode_ports::provider::LlmClient + ?Sized),
    user_input: &str,
) -> String {
    let req = LlmRequest {
        messages: vec![Message::new("u1", Role::User, user_input, 0)],
        model: None,
        max_tokens: Some(64),
        temperature: None,
        tools: vec![],
    };

    let mut stream = client.stream(req).await.unwrap();
    let mut content = String::new();

    while let Some(event) = stream.next().await {
        if let Ok(StreamEvent::Delta { content: c }) = event {
            content.push_str(&c);
        }
    }

    content
}

#[tokio::test]
async fn switching_provider_in_config_requires_no_code_changes() {
    // Anthropic mock
    let anthropic_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(anthropic_text_sse("Hello from Anthropic")),
        )
        .mount(&anthropic_server)
        .await;

    // OpenAI mock
    let openai_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(openai_text_sse("Hello from OpenAI")),
        )
        .mount(&openai_server)
        .await;

    let anthropic = AnthropicAdapter::new("key", anthropic_server.uri(), "claude");
    let openai = OpenRouterAdapter::new(
        "key",
        openai_server.uri(),
        "gpt-4o",
        None,
        None,
        false,
        None,
    );

    // Same generic function — zero provider-specific code
    let r1 = run_one_turn(&anthropic, "Say hello").await;
    let r2 = run_one_turn(&openai, "Say hello").await;

    assert_eq!(r1, "Hello from Anthropic");
    assert_eq!(r2, "Hello from OpenAI");
}
