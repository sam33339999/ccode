# LLM_COMPAT Contract Spec (Rust)

## 1. Objective

Define a production-grade Rust contract for provider-agnostic LLM communication.
The agent must be able to call Anthropic Messages API and OpenAI Chat Completions API through
a single unified interface, with no provider-specific types leaking into upper layers.

## 2. Evidence from Current Code

1. `crates/llm-compat/src/request.rs`: `AnthropicRequest`, `OpenAiRequest` wire types.
2. `crates/llm-compat/src/response.rs`: `AnthropicResponse`, `OpenAiResponse`, streaming chunk types.
3. `crates/llm-compat/src/convert.rs`: bidirectional conversion functions (Anthropic <-> OpenAI).
4. `crates/llm-compat/tests/conversion.rs`: round-trip conversion tests.

Current state: direct Anthropic <-> OpenAI conversion without a Canonical intermediate layer.

## 3. Rust Ownership Boundaries

### Crate mapping

1. `crates/api-types`
   Purpose: define Canonical (provider-agnostic) message, tool_use, and streaming types.

2. `crates/llm-compat`
   Purpose: convert between Canonical types and provider wire formats. House HTTP client traits and streaming adapters.

3. `crates/config`
   Purpose: provider selection, API keys, model aliases, endpoint URLs, retry/timeout policies.

4. `crates/app-services`
   Purpose: agent loop operates exclusively on Canonical types. Calls `LlmClient` trait (defined in `llm-compat`, parameterized by Canonical types from `api-types`).

### Architectural rule

```
api-types (Canonical types)
    ↓
llm-compat (conversion + client trait + provider impls)
    ↓                          ↑
app-services (agent loop)    config (provider selection)
```

`app-services` depends on `llm-compat` via trait only.
`llm-compat` depends on `api-types` only.
No provider wire types may appear in `app-services` or above.

## 4. Core Contracts (Rust)

### 4.1 Canonical Types (`crates/api-types/src/llm.rs`)

```rust
use serde::{Deserialize, Serialize};

/// Provider-agnostic content block
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub stream: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub id: String,
    pub model: String,
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<StopReason>,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Streaming event (provider-agnostic)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    #[serde(rename = "content_block_start")]
    ContentBlockStart { index: usize, block: ContentBlock },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: usize, delta: DeltaBlock },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: usize },
    #[serde(rename = "message_stop")]
    MessageStop {
        stop_reason: Option<StopReason>,
        usage: TokenUsage,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DeltaBlock {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
}
```

### 4.2 Client Trait (`crates/llm-compat/src/client.rs`)

```rust
use api_types::llm::*;

#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    /// Send a request and receive a complete response
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError>;

    /// Send a request and receive a stream of events
    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, LlmError>> + Send>>, LlmError>;
}
```

### 4.3 Provider Adapters (`crates/llm-compat/src/providers/`)

```rust
/// Anthropic Messages API adapter
pub struct AnthropicAdapter {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

/// OpenAI Chat Completions API adapter
pub struct OpenAiAdapter {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}
```

Each adapter implements `LlmClient` by:
1. Converting `LlmRequest` (Canonical) -> provider wire format
2. Sending HTTP request
3. Converting provider response -> `LlmResponse` / `StreamEvent` (Canonical)

### 4.4 Conversion Layer (`crates/llm-compat/src/convert.rs`)

Current bidirectional conversion is preserved and extended to triple conversion:

```
AnthropicRequest ←→ LlmRequest (Canonical) ←→ OpenAiRequest
AnthropicResponse ←→ LlmResponse (Canonical) ←→ OpenAiResponse
AnthropicStreamEvent ←→ StreamEvent (Canonical) ←→ OpenAiChunk
```

Existing functions become internal implementation details of the provider adapters.
Public API surface is only `LlmClient` trait + Canonical types.

## 5. Tool Use Mapping

| Canonical | Anthropic Wire | OpenAI Wire |
|---|---|---|
| `ContentBlock::ToolUse` | `type: "tool_use"` block | `tool_calls[].function` |
| `ContentBlock::ToolResult` | `type: "tool_result"` block | `role: "tool"` message |
| `ToolDefinition` | `tools[].input_schema` | `tools[].function.parameters` |
| `StopReason::ToolUse` | `stop_reason: "tool_use"` | `finish_reason: "tool_calls"` |

## 6. Streaming Mapping

| Canonical StreamEvent | Anthropic SSE | OpenAI SSE |
|---|---|---|
| `ContentBlockStart` | `content_block_start` | first chunk with role |
| `ContentBlockDelta::TextDelta` | `content_block_delta` (text) | `choices[].delta.content` |
| `ContentBlockDelta::InputJsonDelta` | `content_block_delta` (input_json) | `choices[].delta.tool_calls[].function.arguments` |
| `ContentBlockStop` | `content_block_stop` | (implicit, next block or finish) |
| `MessageStop` | `message_stop` | `choices[].finish_reason != null` |

## 7. Error Taxonomy

```rust
#[derive(thiserror::Error, Debug)]
pub enum LlmError {
    #[error("authentication failed: {0}")]
    AuthError(String),

    #[error("rate limited, retry after {retry_after_ms:?}ms")]
    RateLimited { retry_after_ms: Option<u64> },

    #[error("model not found or not available: {0}")]
    ModelNotAvailable(String),

    #[error("request too large: {0}")]
    RequestTooLarge(String),

    #[error("provider returned invalid response: {0}")]
    InvalidResponse(String),

    #[error("stream interrupted: {0}")]
    StreamInterrupted(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("timeout after {0:?}")]
    Timeout(std::time::Duration),

    #[error("provider error ({status}): {message}")]
    ProviderError { status: u16, message: String },
}
```

Policy: provider-specific HTTP errors are mapped to `LlmError` variants inside the adapter.
`app-services` never sees raw HTTP status codes or provider error formats.

## 8. Constants Classification (LLM_COMPAT)

1. `api-types`: Canonical enum variants (`Role`, `StopReason`, `ContentBlock` tag names), `ToolDefinition` schema.
2. `config`: provider selection key, API key env var names, base URL defaults, model alias table, retry/timeout defaults.
3. `llm-compat` (crate-local): provider wire format field names, SSE event type strings, HTTP header constants, content-type values.

## 9. Migration Path from Current Code

### Step 1: Introduce Canonical types in `api-types`
No breaking changes. New types alongside existing code.

### Step 2: Refactor `llm-compat` conversion
- Current: `anthropic_request_to_openai_request()` (direct A↔O)
- Target: `anthropic_request_to_canonical()` + `canonical_to_openai_request()` (via Canonical)
- Keep old functions as deprecated aliases during migration.

### Step 3: Add `LlmClient` trait and provider adapters
- `AnthropicAdapter` and `OpenAiAdapter` implement `LlmClient`.
- Each uses the refactored conversion functions internally.

### Step 4: Add streaming support
- Implement SSE parsing for both providers.
- Map provider stream events to `StreamEvent` Canonical type.

### Step 5: Add tool_use support
- Extend conversion for tool definitions, tool_use blocks, and tool_result blocks.
- Ensure round-trip correctness for the agent loop.

## 10. Acceptance Criteria

### 10.1 Contract tests

1. `LlmRequest` round-trips through Anthropic wire format without data loss.
2. `LlmRequest` round-trips through OpenAI wire format without data loss.
3. `ToolDefinition` schema mapping is bijective between Canonical and both providers.
4. `StopReason::ToolUse` maps correctly to/from both provider representations.
5. Streaming events maintain ordering and content integrity through conversion.

### 10.2 Integration tests

1. `AnthropicAdapter::complete()` sends valid Messages API request and parses response.
2. `OpenAiAdapter::complete()` sends valid Chat Completions request and parses response.
3. `AnthropicAdapter::stream()` yields correct `StreamEvent` sequence from SSE.
4. `OpenAiAdapter::stream()` yields correct `StreamEvent` sequence from SSE.
5. Tool use flow: request with tools -> response with tool_use -> follow-up with tool_result.

### 10.3 Error handling tests

1. 401/403 -> `LlmError::AuthError`.
2. 429 -> `LlmError::RateLimited` with retry-after extraction.
3. 413/400 (too large) -> `LlmError::RequestTooLarge`.
4. Network timeout -> `LlmError::Timeout`.
5. Malformed response body -> `LlmError::InvalidResponse`.

### 10.4 Agent loop integration

1. `app-services` agent loop can alternate between `complete()` and tool execution using only Canonical types.
2. Switching provider in `config` requires zero code changes in `app-services`.
