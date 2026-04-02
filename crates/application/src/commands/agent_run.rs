use crate::error::AppError;
use async_trait::async_trait;
use ccode_domain::{
    message::{Message, Role},
    session::{Session, SessionId},
};
use ccode_ports::{
    provider::{LlmClient, LlmError, LlmRequest, LlmStream, StreamEvent, ToolDefinition},
    repositories::SessionRepository,
};
use futures::StreamExt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Configures context window management for the agentic loop.
/// All values have sensible defaults — only override what you need.
#[derive(Debug, Clone)]
pub struct ContextPolicy {
    /// Trigger compression when total context size (all messages concatenated)
    /// exceeds this many characters. Rough estimate: 4 chars ≈ 1 token.
    /// Default: 600_000 (~150k tokens, leaves buffer for a 200k-context model).
    pub compress_chars_threshold: usize,
    /// Number of most-recent messages kept verbatim after compression.
    /// Default: 8.
    pub keep_recent_messages: usize,
    /// Truncate any single tool result exceeding this many characters.
    /// Default: 40_000 (~10k tokens).
    pub tool_result_max_chars: usize,
}

impl Default for ContextPolicy {
    fn default() -> Self {
        Self {
            compress_chars_threshold: 600_000,
            keep_recent_messages: 8,
            tool_result_max_chars: 40_000,
        }
    }
}

pub struct AgentRunCommand<R> {
    repo: R,
    provider: Arc<dyn LlmClient>,
    context: ContextPolicy,
    computer_use_lifecycle: Option<Arc<dyn ComputerUseLifecycle>>,
}

type ExecuteToolFn = dyn Fn(String, serde_json::Value) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
    + Send
    + Sync;

#[derive(Debug, thiserror::Error)]
pub enum ComputerUseLifecycleError {
    #[error("transport error")]
    TransportError,
    #[error("invalid tool payload")]
    InvalidToolPayload,
    #[error("cleanup failed")]
    CleanupFailed,
}

#[async_trait]
pub trait ComputerUseLifecycle: Send + Sync {
    async fn before_tool_call(&self) -> Result<(), ComputerUseLifecycleError>;
    async fn after_turn_cleanup(&self) -> Result<(), ComputerUseLifecycleError>;
    async fn on_interrupt_cleanup(&self) -> Result<(), ComputerUseLifecycleError>;
}

impl<R: SessionRepository> AgentRunCommand<R> {
    pub fn new(repo: R, provider: Arc<dyn LlmClient>) -> Self {
        Self {
            repo,
            provider,
            context: ContextPolicy::default(),
            computer_use_lifecycle: None,
        }
    }

    pub fn with_context(mut self, policy: ContextPolicy) -> Self {
        self.context = policy;
        self
    }

    pub fn with_computer_use_lifecycle(mut self, lifecycle: Arc<dyn ComputerUseLifecycle>) -> Self {
        self.computer_use_lifecycle = Some(lifecycle);
        self
    }

    /// Full agentic loop — keeps calling the model and executing tools until
    /// the model produces no more tool calls (or max_iterations is reached).
    pub async fn run(
        &self,
        session_id: Option<String>,
        system_prompt: Option<String>,
        user_content: String,
        tools: Vec<ToolDefinition>,
        on_delta: &(dyn Fn(String) + Send + Sync),
        execute_tool: &ExecuteToolFn,
    ) -> Result<SessionId, AppError> {
        let lifecycle = self.computer_use_lifecycle.as_deref();
        let now = now_ms();

        let mut session = match session_id {
            Some(ref id) => {
                let sid = SessionId(id.clone());
                self.repo
                    .find_by_id(&sid)
                    .await?
                    .unwrap_or_else(|| Session::new(id.clone(), now))
            }
            None => Session::new(format!("sess-{now}"), now),
        };

        // Prepend system prompt only when starting a fresh session
        if let Some(ref prompt) = system_prompt
            && session.messages.is_empty()
        {
            let sys_id = format!("msg-{now}-sys");
            session.add_message(Message::new(sys_id, Role::System, prompt.clone(), now), now);
        }

        // Add the user message
        let msg_id = format!("msg-{now}-u");
        session.add_message(Message::new(msg_id, Role::User, user_content, now), now);
        self.repo.save(&session).await?;

        const MAX_ITERATIONS: usize = 10;

        for _iter in 0..MAX_ITERATIONS {
            // ── Context compression ────────────────────────────────────────────
            // Trigger on total character size (not message count) to catch large
            // tool results that would overflow the model's context window.
            let total_chars: usize = session.messages.iter().map(|m| m.content.len()).sum();
            if total_chars > self.context.compress_chars_threshold {
                session =
                    compress_context(session, &*self.provider, self.context.keep_recent_messages)
                        .await?;
                self.repo.save(&session).await?;
            }

            let req = LlmRequest {
                messages: session.messages.clone(),
                model: None,
                max_tokens: None,
                temperature: None,
                tools: tools.clone(),
            };

            let mut stream = self.provider.stream(req).await?;

            let mut assistant_content = String::new();
            let mut captured_tool_calls: Vec<ccode_domain::message::ToolCall> = Vec::new();
            let mut last_usage = None;

            while let Some(event) = stream.next().await {
                match event {
                    Err(err @ LlmError::StreamInterrupted(_)) => {
                        run_interrupt_cleanup(lifecycle).await;
                        return Err(AppError::from(err));
                    }
                    Err(err) => return Err(AppError::from(err)),
                    Ok(StreamEvent::Delta { content }) => {
                        on_delta(content.clone());
                        assistant_content.push_str(&content);
                    }
                    Ok(StreamEvent::ToolCallDone { tool_calls }) => {
                        captured_tool_calls.extend(tool_calls);
                    }
                    Ok(StreamEvent::Done { usage }) => {
                        last_usage = usage;
                        break;
                    }
                }
            }

            // Save assistant message with tool_calls if any
            let ts = now_ms();
            let msg_id = format!("msg-{ts}-a");
            let mut asst_msg = Message::new(msg_id, Role::Assistant, assistant_content, ts);
            if !captured_tool_calls.is_empty() {
                asst_msg.tool_calls = Some(captured_tool_calls.clone());
            }
            session.add_message(asst_msg, ts);
            self.repo.save(&session).await?;

            // If no tool calls, we're done
            if captured_tool_calls.is_empty() {
                break;
            }

            // Execute each tool call and add results to session
            for tc in &captured_tool_calls {
                if let Some(lifecycle) = lifecycle
                    && let Err(err) = lifecycle.before_tool_call().await
                {
                    tracing::warn!("computer-use before_tool_call hook failed: {err}");
                }

                let args_value: serde_json::Value =
                    serde_json::from_str(&tc.arguments).unwrap_or(serde_json::Value::Null);
                let result = execute_tool(tc.name.clone(), args_value).await;

                let result_content = match result {
                    Ok(s) => s,
                    Err(e) => format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\"")),
                };
                // Truncate oversized tool results before adding to context
                let result_content = if result_content.len() > self.context.tool_result_max_chars {
                    format!(
                        "{}…[truncated: {} chars total, showing first {}]",
                        &result_content[..self.context.tool_result_max_chars],
                        result_content.len(),
                        self.context.tool_result_max_chars,
                    )
                } else {
                    result_content
                };

                let ts2 = now_ms();
                let result_msg = Message::new_tool_result(
                    format!("msg-{ts2}-t"),
                    tc.id.clone(),
                    result_content,
                    ts2,
                );
                session.add_message(result_msg, ts2);
            }
            self.repo.save(&session).await?;

            let _ = last_usage; // suppress unused warning
        }

        run_after_turn_cleanup(lifecycle).await;
        Ok(session.id)
    }

    /// Append the user message to the session (creating it if needed), then
    /// start a streaming completion.  Returns `(session_id, stream)`.
    ///
    /// After draining the stream, call [`finish`] to persist the assistant reply.
    pub async fn start(
        &self,
        session_id: Option<String>,
        user_content: String,
    ) -> Result<(SessionId, LlmStream), AppError> {
        let now = now_ms();

        let mut session = match session_id {
            Some(ref id) => {
                let sid = SessionId(id.clone());
                self.repo
                    .find_by_id(&sid)
                    .await?
                    .unwrap_or_else(|| Session::new(id.clone(), now))
            }
            None => Session::new(format!("sess-{now}"), now),
        };

        let msg_id = format!("msg-{now}-u");
        session.add_message(Message::new(msg_id, Role::User, user_content, now), now);
        self.repo.save(&session).await?;

        let req = LlmRequest {
            messages: session.messages.clone(),
            model: None,
            max_tokens: None,
            temperature: None,
            tools: Vec::new(),
        };

        let stream = self.provider.stream(req).await?;
        Ok((session.id, stream))
    }

    /// Persist the collected assistant response to the session.
    pub async fn finish(
        &self,
        session_id: &SessionId,
        assistant_content: String,
    ) -> Result<(), AppError> {
        let now = now_ms();
        let mut session = self
            .repo
            .find_by_id(session_id)
            .await?
            .unwrap_or_else(|| Session::new(session_id.0.clone(), now));

        let msg_id = format!("msg-{now}-a");
        session.add_message(
            Message::new(msg_id, Role::Assistant, assistant_content, now),
            now,
        );
        self.repo.save(&session).await?;
        Ok(())
    }
}

async fn run_after_turn_cleanup(lifecycle: Option<&dyn ComputerUseLifecycle>) {
    if let Some(lifecycle) = lifecycle
        && let Err(err) = lifecycle.after_turn_cleanup().await
    {
        tracing::warn!("computer-use after_turn_cleanup hook failed: {err}");
    }
}

async fn run_interrupt_cleanup(lifecycle: Option<&dyn ComputerUseLifecycle>) {
    if let Some(lifecycle) = lifecycle
        && let Err(err) = lifecycle.on_interrupt_cleanup().await
    {
        tracing::warn!("computer-use on_interrupt_cleanup hook failed: {err}");
    }
}

/// Summarise all but the most recent `keep_recent` messages into a single
/// system-level summary message, replacing the old messages in the session.
async fn compress_context(
    mut session: Session,
    provider: &dyn LlmClient,
    keep_recent: usize,
) -> Result<Session, AppError> {
    let total = session.messages.len();
    if total <= keep_recent {
        return Ok(session);
    }

    let split_at = total - keep_recent;
    let to_compress: Vec<_> = session.messages.drain(..split_at).collect();
    let recent = session.messages.clone();

    // Build a text transcript for the LLM to summarise
    let transcript: String = to_compress
        .iter()
        .map(|m| {
            format!(
                "[{}]: {}",
                format!("{:?}", m.role).to_lowercase(),
                m.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let summary_prompt = format!(
        "Summarise the following conversation history concisely. \
         Preserve key facts, decisions, tool results, and any important context \
         the assistant will need to continue the task. \
         Do not include meta-commentary about the summary itself.\n\n{transcript}"
    );

    let req = LlmRequest {
        messages: vec![Message::new(
            "compress-req",
            Role::User,
            summary_prompt,
            now_ms(),
        )],
        model: None,
        max_tokens: Some(1024),
        temperature: Some(0.0),
        tools: vec![],
    };

    let summary_text = match provider.complete(req).await {
        Ok(r) => r.content,
        Err(_) => {
            // If compression fails, just restore and continue (non-fatal)
            let mut restored = to_compress;
            restored.extend(recent);
            session.messages = restored;
            return Ok(session);
        }
    };

    let now = now_ms();
    let summary_msg = Message::new(
        format!("msg-{now}-summary"),
        Role::System,
        format!("[Conversation summary — earlier context compressed]\n{summary_text}"),
        now,
    );

    let mut new_messages = vec![summary_msg];
    new_messages.extend(recent);
    session.messages = new_messages;

    tracing::info!(
        "[compress] session {} reduced from {} → {} messages",
        session.id,
        total,
        session.messages.len()
    );

    Ok(session)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ccode_domain::session::{Session, SessionId, SessionSummary};
    use ccode_ports::{
        PortError,
        provider::{LlmError, LlmResponse},
    };
    use futures::stream;
    use std::collections::VecDeque;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Default)]
    struct MockRepo {
        sessions: Mutex<Vec<Session>>,
    }

    #[async_trait]
    impl SessionRepository for MockRepo {
        async fn list(&self, _limit: usize) -> Result<Vec<SessionSummary>, PortError> {
            Ok(Vec::new())
        }

        async fn find_by_id(&self, id: &SessionId) -> Result<Option<Session>, PortError> {
            let sessions = self.sessions.lock().expect("poisoned");
            Ok(sessions.iter().find(|s| &s.id == id).cloned())
        }

        async fn save(&self, session: &Session) -> Result<(), PortError> {
            let mut sessions = self.sessions.lock().expect("poisoned");
            if let Some(existing) = sessions.iter_mut().find(|s| s.id == session.id) {
                *existing = session.clone();
            } else {
                sessions.push(session.clone());
            }
            Ok(())
        }

        async fn delete(&self, _id: &SessionId) -> Result<(), PortError> {
            Ok(())
        }
    }

    struct MockProvider {
        streams: Mutex<VecDeque<Vec<Result<StreamEvent, LlmError>>>>,
    }

    #[async_trait]
    impl LlmClient for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }

        fn default_model(&self) -> &str {
            "mock-model"
        }

        async fn health_check(&self) -> Result<(), LlmError> {
            Ok(())
        }

        async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
            Ok(LlmResponse {
                content: String::new(),
                model: "mock-model".to_string(),
                usage: None,
            })
        }

        async fn stream(&self, _req: LlmRequest) -> Result<LlmStream, LlmError> {
            let mut streams = self.streams.lock().expect("poisoned");
            let events = streams.pop_front().unwrap_or_default();
            Ok(Box::pin(stream::iter(events)))
        }
    }

    #[derive(Default)]
    struct MockLifecycle {
        before_calls: AtomicUsize,
        cleanup_calls: AtomicUsize,
        interrupt_cleanup_calls: AtomicUsize,
        fail_cleanup: bool,
    }

    #[async_trait]
    impl ComputerUseLifecycle for MockLifecycle {
        async fn before_tool_call(&self) -> Result<(), ComputerUseLifecycleError> {
            self.before_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn after_turn_cleanup(&self) -> Result<(), ComputerUseLifecycleError> {
            self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_cleanup {
                return Err(ComputerUseLifecycleError::CleanupFailed);
            }
            Ok(())
        }

        async fn on_interrupt_cleanup(&self) -> Result<(), ComputerUseLifecycleError> {
            self.interrupt_cleanup_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_cleanup {
                return Err(ComputerUseLifecycleError::CleanupFailed);
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn normal_exit_runs_cleanup_even_when_cleanup_returns_error() {
        let repo = MockRepo::default();
        let provider = Arc::new(MockProvider {
            streams: Mutex::new(VecDeque::from([vec![Ok(StreamEvent::Done {
                usage: None,
            })]])),
        });
        let lifecycle = Arc::new(MockLifecycle {
            fail_cleanup: true,
            ..Default::default()
        });
        let cmd =
            AgentRunCommand::new(repo, provider).with_computer_use_lifecycle(lifecycle.clone());

        let session_id = cmd
            .run(
                None,
                None,
                "hello".to_string(),
                Vec::new(),
                &|_| {},
                &|_, _| Box::pin(async { Ok(String::new()) }),
            )
            .await
            .expect("cleanup failure must be non-fatal");

        assert!(!session_id.0.is_empty());
        assert_eq!(lifecycle.cleanup_calls.load(Ordering::SeqCst), 1);
        assert_eq!(lifecycle.interrupt_cleanup_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stream_interrupt_runs_interrupt_cleanup() {
        let repo = MockRepo::default();
        let provider = Arc::new(MockProvider {
            streams: Mutex::new(VecDeque::from([vec![Err(LlmError::StreamInterrupted(
                "ctrl-c".to_string(),
            ))]])),
        });
        let lifecycle = Arc::new(MockLifecycle::default());
        let cmd =
            AgentRunCommand::new(repo, provider).with_computer_use_lifecycle(lifecycle.clone());

        let result = cmd
            .run(
                None,
                None,
                "hello".to_string(),
                Vec::new(),
                &|_| {},
                &|_, _| Box::pin(async { Ok(String::new()) }),
            )
            .await;

        assert!(matches!(
            result,
            Err(AppError::Llm(LlmError::StreamInterrupted(_)))
        ));
        assert_eq!(lifecycle.interrupt_cleanup_calls.load(Ordering::SeqCst), 1);
        assert_eq!(lifecycle.cleanup_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn lifecycle_before_tool_call_is_invoked_before_tool_execution() {
        let repo = MockRepo::default();
        let provider = Arc::new(MockProvider {
            streams: Mutex::new(VecDeque::from([
                vec![
                    Ok(StreamEvent::ToolCallDone {
                        tool_calls: vec![ccode_domain::message::ToolCall {
                            id: "t1".to_string(),
                            name: "echo".to_string(),
                            arguments: "{}".to_string(),
                        }],
                    }),
                    Ok(StreamEvent::Done { usage: None }),
                ],
                vec![Ok(StreamEvent::Done { usage: None })],
            ])),
        });
        let lifecycle = Arc::new(MockLifecycle::default());
        let cmd =
            AgentRunCommand::new(repo, provider).with_computer_use_lifecycle(lifecycle.clone());

        let _ = cmd
            .run(
                None,
                None,
                "hello".to_string(),
                vec![ToolDefinition {
                    name: "echo".to_string(),
                    description: "echo".to_string(),
                    parameters: serde_json::json!({"type":"object"}),
                }],
                &|_| {},
                &|_, _| Box::pin(async { Ok("ok".to_string()) }),
            )
            .await
            .expect("run should succeed");

        assert_eq!(lifecycle.before_calls.load(Ordering::SeqCst), 1);
        assert_eq!(lifecycle.cleanup_calls.load(Ordering::SeqCst), 1);
    }
}
