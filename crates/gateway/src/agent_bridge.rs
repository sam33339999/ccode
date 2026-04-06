use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use ccode_application::commands::agent_run::AgentRunCommand;
use ccode_bootstrap::AppState;

#[allow(dead_code)]
pub async fn run_agent(
    state: &AppState,
    text: String,
    session_id: Option<String>,
) -> anyhow::Result<String> {
    let provider = state
        .provider
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no LLM provider configured — set OPENROUTER_API_KEY"))?;

    let cmd = AgentRunCommand::new(Arc::clone(&state.session_repo), provider)
        .with_context(state.context_policy.clone());
    let tool_definitions = state.tool_registry.definitions();

    let reply = Arc::new(Mutex::new(String::new()));
    let reply_for_delta = Arc::clone(&reply);
    let on_delta = move |content: String| {
        reply_for_delta.lock().unwrap().push_str(&content);
    };

    let registry = Arc::clone(&state.tool_registry);
    let tool_ctx = Arc::new(state.tool_ctx());
    let execute_tool = move |name: String,
                             args: serde_json::Value|
          -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
        let registry = Arc::clone(&registry);
        let tool_ctx = Arc::clone(&tool_ctx);
        Box::pin(async move {
            registry
                .execute(&name, args, &tool_ctx)
                .await
                .map_err(|err| err.to_string())
        })
    };

    cmd.run_with_metrics(
        session_id.clone(),
        None,
        text,
        tool_definitions,
        &on_delta,
        &execute_tool,
    )
    .await
    .with_context(|| {
        format!(
            "error_context session={}",
            session_id.as_deref().unwrap_or("new")
        )
    })?;

    Ok(reply.lock().unwrap().clone())
}

#[cfg(test)]
mod tests {
    use ccode_bootstrap::wire_dev;

    #[tokio::test]
    async fn run_agent_returns_error_when_provider_missing() {
        let state = wire_dev();
        let result = super::run_agent(&state, "hello".to_string(), None).await;
        assert!(result.is_err());
    }
}
