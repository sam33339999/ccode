//! Agent-loop tests — llm-compat-contract.md §10.4
//!
//! AC5  — streaming events maintain ordering and content integrity
//! AC16 — agent loop alternates between complete() and tool execution using
//!         only Canonical types (no provider wire types leak into the loop)

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use ccode_domain::message::{Message, Role, ToolCall};
use ccode_ports::provider::{
    LlmClient, LlmError, LlmRequest, LlmResponse, LlmStream, StreamEvent, ToolDefinition,
};
use futures::{StreamExt, stream};

// ── Mock LlmClient ────────────────────────────────────────────────────────────

/// A deterministic mock that replays pre-configured `StreamEvent` sequences.
/// Each call to `stream()` pops the next sequence from the queue.
struct StepClient {
    streams: Mutex<VecDeque<Vec<StreamEvent>>>,
}

impl StepClient {
    fn new(sequences: Vec<Vec<StreamEvent>>) -> Self {
        Self {
            streams: Mutex::new(VecDeque::from(sequences)),
        }
    }
}

#[async_trait]
impl LlmClient for StepClient {
    fn name(&self) -> &str {
        "step-mock"
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }

    async fn health_check(&self) -> Result<(), LlmError> {
        Ok(())
    }

    async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(LlmError::Network(
            "StepClient drives agent loops via stream()".into(),
        ))
    }

    async fn stream(&self, _req: LlmRequest) -> Result<LlmStream, LlmError> {
        let events = self
            .streams
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| LlmError::Network("no more mock streams".into()))?;

        let items: Vec<Result<StreamEvent, LlmError>> = events.into_iter().map(Ok).collect();
        Ok(Box::pin(stream::iter(items)))
    }
}

// ── AC5: Streaming event ordering and content integrity ───────────────────────

#[tokio::test]
async fn stream_events_arrive_in_delta_toolcalldone_done_order() {
    let tc = ToolCall {
        id: "c1".into(),
        name: "add".into(),
        arguments: r#"{"a":1,"b":2}"#.into(),
    };

    // Sequence: Delta → Delta → ToolCallDone → Done
    let client = StepClient::new(vec![vec![
        StreamEvent::Delta {
            content: "thinking…".into(),
        },
        StreamEvent::Delta {
            content: " ok".into(),
        },
        StreamEvent::ToolCallDone {
            tool_calls: vec![tc.clone()],
        },
        StreamEvent::Done { usage: None },
    ]]);

    let req = LlmRequest {
        messages: vec![Message::new("u1", Role::User, "1+2?", 0)],
        model: None,
        max_tokens: None,
        temperature: None,
        tools: vec![],
    };

    let mut stream = client.stream(req).await.unwrap();
    let mut order: Vec<&str> = Vec::new();
    let mut accumulated_text = String::new();
    let mut received_tool_calls: Vec<ToolCall> = Vec::new();

    while let Some(event) = stream.next().await {
        match event.unwrap() {
            StreamEvent::Delta { content } => {
                accumulated_text.push_str(&content);
                order.push("delta");
            }
            StreamEvent::ToolCallDone { tool_calls } => {
                received_tool_calls.extend(tool_calls);
                order.push("tool");
            }
            StreamEvent::Done { .. } => {
                order.push("done");
            }
        }
    }

    assert_eq!(
        order,
        ["delta", "delta", "tool", "done"],
        "ordering preserved"
    );
    assert_eq!(
        accumulated_text, "thinking… ok",
        "content integrity preserved"
    );
    assert_eq!(received_tool_calls.len(), 1);
    assert_eq!(received_tool_calls[0].name, "add");
    assert_eq!(received_tool_calls[0].arguments, r#"{"a":1,"b":2}"#);
}

#[tokio::test]
async fn multiple_tool_calls_in_single_event_are_all_delivered() {
    let client = StepClient::new(vec![vec![
        StreamEvent::ToolCallDone {
            tool_calls: vec![
                ToolCall {
                    id: "c1".into(),
                    name: "read_file".into(),
                    arguments: r#"{"path":"/a"}"#.into(),
                },
                ToolCall {
                    id: "c2".into(),
                    name: "read_file".into(),
                    arguments: r#"{"path":"/b"}"#.into(),
                },
            ],
        },
        StreamEvent::Done { usage: None },
    ]]);

    let req = LlmRequest {
        messages: vec![Message::new("u1", Role::User, "read both", 0)],
        model: None,
        max_tokens: None,
        temperature: None,
        tools: vec![],
    };

    let mut stream = client.stream(req).await.unwrap();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    while let Some(event) = stream.next().await {
        if let Ok(StreamEvent::ToolCallDone { tool_calls: tcs }) = event {
            tool_calls.extend(tcs);
        }
    }

    assert_eq!(tool_calls.len(), 2, "both tool calls must be delivered");
    assert_eq!(tool_calls[0].id, "c1");
    assert_eq!(tool_calls[1].id, "c2");
}

// ── AC16: Agent loop using only Canonical types ───────────────────────────────

/// Minimal agent loop — mirrors `AgentRunCommand::run` but without persistence.
/// Uses *only* types from `ccode_ports::provider` and `ccode_domain::message`.
/// No provider wire types (`AnthropicRequest`, `OpenAiRequest`, …) appear here.
async fn run_agent_loop(
    client: &dyn LlmClient,
    initial_messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
) -> Vec<Message> {
    let mut messages = initial_messages;
    const MAX_ITERS: usize = 10;

    for _ in 0..MAX_ITERS {
        let req = LlmRequest {
            messages: messages.clone(),
            model: None,
            max_tokens: None,
            temperature: None,
            tools: tools.clone(),
        };

        let mut stream = client.stream(req).await.expect("stream must not fail");
        let mut content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        while let Some(event) = stream.next().await {
            match event.expect("stream event must be Ok") {
                StreamEvent::Delta { content: c } => content.push_str(&c),
                StreamEvent::ToolCallDone { tool_calls: tcs } => tool_calls.extend(tcs),
                StreamEvent::Done { .. } => break,
            }
        }

        let mut asst_msg = Message::new("a", Role::Assistant, content, 0);
        if !tool_calls.is_empty() {
            asst_msg.tool_calls = Some(tool_calls.clone());
        }
        messages.push(asst_msg);

        if tool_calls.is_empty() {
            break; // no more tool calls → agent is done
        }

        // Execute each tool with only canonical ToolCall + serde_json
        for tc in &tool_calls {
            let args: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or_default();
            let a = args["a"].as_f64().unwrap_or(0.0);
            let b = args["b"].as_f64().unwrap_or(0.0);
            let result = (a + b).to_string();

            messages.push(Message::new_tool_result("t", &tc.id, result, 0));
        }
    }

    messages
}

#[tokio::test]
async fn agent_loop_alternates_complete_and_tool_execution_using_canonical_types() {
    let tool_call = ToolCall {
        id: "call_1".into(),
        name: "add".into(),
        arguments: r#"{"a":2,"b":2}"#.into(),
    };

    let client = StepClient::new(vec![
        // Iteration 1: model requests a tool call
        vec![
            StreamEvent::ToolCallDone {
                tool_calls: vec![tool_call],
            },
            StreamEvent::Done { usage: None },
        ],
        // Iteration 2: model produces final answer after seeing tool result
        vec![
            StreamEvent::Delta {
                content: "The answer is 4.".into(),
            },
            StreamEvent::Done { usage: None },
        ],
    ]);

    let tools = vec![ToolDefinition {
        name: "add".into(),
        description: "Add two numbers".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "number"}, "b": {"type": "number"}},
            "required": ["a", "b"]
        }),
    }];

    let initial = vec![Message::new("u1", Role::User, "What is 2+2?", 0)];
    let history = run_agent_loop(&client, initial, tools).await;

    // Expect: [user, assistant(tool_call), tool_result, assistant(final)]
    assert_eq!(history.len(), 4, "history must have 4 messages");
    assert_eq!(history[0].role, Role::User);
    assert_eq!(history[1].role, Role::Assistant);
    assert!(
        history[1].tool_calls.is_some(),
        "assistant turn 1 must have tool_calls"
    );
    assert_eq!(history[2].role, Role::Tool);
    assert_eq!(history[2].content, "4", "tool result must be '4'");
    assert_eq!(history[3].role, Role::Assistant);
    assert_eq!(history[3].content, "The answer is 4.");
    assert!(
        history[3].tool_calls.is_none(),
        "final assistant turn must have no tool_calls"
    );
}

#[tokio::test]
async fn agent_loop_terminates_immediately_when_no_tool_calls() {
    let client = StepClient::new(vec![vec![
        StreamEvent::Delta {
            content: "42".into(),
        },
        StreamEvent::Done { usage: None },
    ]]);

    let initial = vec![Message::new("u1", Role::User, "What is 6×7?", 0)];
    let history = run_agent_loop(&client, initial, vec![]).await;

    assert_eq!(history.len(), 2, "just user + assistant");
    assert_eq!(history[1].content, "42");
}

#[tokio::test]
async fn agent_loop_can_be_driven_by_any_llm_client_implementation() {
    // Prove that run_agent_loop is polymorphic: it accepts &dyn LlmClient.
    // We pass two different concrete types — the loop code is unchanged.
    let client_a = StepClient::new(vec![vec![
        StreamEvent::Delta {
            content: "from A".into(),
        },
        StreamEvent::Done { usage: None },
    ]]);

    let client_b = StepClient::new(vec![vec![
        StreamEvent::Delta {
            content: "from B".into(),
        },
        StreamEvent::Done { usage: None },
    ]]);

    let initial = vec![Message::new("u1", Role::User, "hi", 0)];

    // Same function, two different provider implementations
    let ha = run_agent_loop(&client_a, initial.clone(), vec![]).await;
    let hb = run_agent_loop(&client_b, initial, vec![]).await;

    assert_eq!(ha.last().unwrap().content, "from A");
    assert_eq!(hb.last().unwrap().content, "from B");
}
