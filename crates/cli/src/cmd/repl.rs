use anyhow::Context;
use ccode_application::commands::agent_run::AgentRunCommand;
use clap::Args;
use std::io::{self, BufRead, IsTerminal};
use std::sync::{Arc, Mutex};

#[derive(Args)]
#[command(
    about = "Chat mode with stdin auto-detection: TTY enters TUI, piped stdin runs line-by-line"
)]
pub struct ReplArgs {
    /// Resume an existing session by ID
    #[arg(short, long)]
    pub session: Option<String>,
    /// System prompt / persona for the agent (e.g. "You are a senior Rust engineer")
    #[arg(long)]
    pub persona: Option<String>,
    /// Skip tool confirmation prompts
    #[arg(long)]
    pub no_confirm: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplMode {
    Tui,
    Pipe,
}

fn mode_from_stdin_is_terminal(stdin_is_terminal: bool) -> ReplMode {
    if stdin_is_terminal {
        ReplMode::Tui
    } else {
        ReplMode::Pipe
    }
}

fn detect_repl_mode() -> ReplMode {
    mode_from_stdin_is_terminal(io::stdin().is_terminal())
}

pub async fn run(args: ReplArgs) -> anyhow::Result<()> {
    match detect_repl_mode() {
        ReplMode::Tui => super::tui::run_ui().await,
        ReplMode::Pipe => run_pipe_mode(args).await,
    }
}

async fn run_pipe_mode(args: ReplArgs) -> anyhow::Result<()> {
    let state = ccode_bootstrap::wire_from_config_with_cwd(std::env::current_dir().ok())?;

    let provider = state
        .provider
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no LLM provider configured — set OPENROUTER_API_KEY"))?;

    let provider_name = provider.name().to_string();
    let tools = state.tool_registry.definitions();
    let tool_ctx = Arc::new(state.tool_ctx());
    let tool_registry = Arc::clone(&state.tool_registry);

    let cmd = AgentRunCommand::new(state.session_repo, provider).with_context(state.context_policy);

    let mut session_id = args.session;
    let mut persona_once =
        ccode_bootstrap::skill::augment_with_skill_catalog(args.persona, &state.skill_catalog);

    for line in io::stdin().lock().lines() {
        let input = line?.trim().to_string();
        if input.is_empty() {
            continue;
        }

        let response = Arc::new(Mutex::new(String::new()));
        let response_for_delta = Arc::clone(&response);
        let on_delta = move |content: String| {
            response_for_delta.lock().unwrap().push_str(&content);
        };

        let registry = Arc::clone(&tool_registry);
        let ctx = Arc::clone(&tool_ctx);
        let execute_tool = move |name: String,
                                 tool_args: serde_json::Value|
              -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<String, String>> + Send>,
        > {
            let registry = Arc::clone(&registry);
            let ctx = Arc::clone(&ctx);
            Box::pin(async move {
                registry
                    .execute(&name, tool_args, &ctx)
                    .await
                    .map_err(|e| e.to_string())
            })
        };

        let session_for_errors = session_id.clone().unwrap_or_else(|| "new".to_string());
        let outcome = cmd
            .run_with_metrics(
                session_id.clone(),
                persona_once.take(),
                input,
                Vec::new(),
                tools.clone(),
                &on_delta,
                &execute_tool,
            )
            .await
            .with_context(|| {
                format!(
                    "error_context provider={} session={}",
                    provider_name, session_for_errors
                )
            })?;

        session_id = Some(outcome.session_id.to_string());
        let output = std::mem::take(&mut *response.lock().unwrap());
        println!("{output}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ReplMode, mode_from_stdin_is_terminal};

    #[test]
    fn tty_stdin_selects_tui_mode() {
        assert_eq!(mode_from_stdin_is_terminal(true), ReplMode::Tui);
    }

    #[test]
    fn non_tty_stdin_selects_pipe_mode() {
        assert_eq!(mode_from_stdin_is_terminal(false), ReplMode::Pipe);
    }
}
