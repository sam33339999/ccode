use anyhow::Context;
use ccode_application::commands::agent_run::AgentRunCommand;
use clap::Args;
use rustyline::{DefaultEditor, error::ReadlineError};
use std::collections::HashSet;
use std::io::Write;
use std::sync::{Arc, Mutex};

use super::output::{
    ErrorContext, StreamFormatter, StreamProgress, ToolConfirmationDecision, confirmation_prompt,
    parse_confirmation_input, render_error_message,
};
#[derive(Args)]
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

pub async fn run(args: ReplArgs) -> anyhow::Result<()> {
    let state = ccode_bootstrap::wire_from_config_with_cwd(std::env::current_dir().ok())?;

    let provider = state
        .provider
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no LLM provider configured — set OPENROUTER_API_KEY"))?;

    eprintln!(
        "[provider: {} | model: {}]",
        provider.name(),
        provider.default_model()
    );
    let provider_name_for_errors = provider.name().to_string();

    let registry = Arc::clone(&state.tool_registry);
    let tool_ctx = Arc::new(state.tool_ctx());
    let tool_definitions = registry.definitions();

    eprintln!(
        "[tools: {}]",
        tool_definitions
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    eprintln!("[type 'exit' or press Ctrl+D to quit]\n");

    let cmd = Arc::new(
        AgentRunCommand::new(state.session_repo, provider).with_context(state.context_policy),
    );
    let mut session_id: Option<String> = args.session;
    let persona = args.persona;
    let no_confirm = args.no_confirm;

    let handle = tokio::runtime::Handle::current();

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut editor = DefaultEditor::new()?;
        // Persists across all turns within this REPL session
        let always_allowed: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let formatter: Arc<Mutex<StreamFormatter>> = Arc::new(Mutex::new(StreamFormatter::new()));
        // Persona is applied only on the first turn (new session); subsequent turns pass None
        let mut persona_once: Option<String> = persona;

        loop {
            match editor.readline("You: ") {
                Ok(line) => {
                    let input = line.trim().to_string();
                    if input.is_empty() {
                        continue;
                    }
                    if input == "exit" || input == "quit" {
                        break;
                    }
                    let _ = editor.add_history_entry(&input);

                    let registry_clone = registry.clone();
                    let tool_ctx_clone = tool_ctx.clone();
                    let tools = tool_definitions.clone();
                    let always_allowed_clone = always_allowed.clone();
                    let formatter_clone = formatter.clone();
                    let session_for_context =
                        session_id.clone().unwrap_or_else(|| "new".to_string());
                    let provider_for_context = provider_name_for_errors.clone();
                    let session_for_tool = session_for_context.clone();
                    let provider_for_tool = provider_for_context.clone();

                    let execute_tool = move |name: String,
                                             tool_args: serde_json::Value|
                          -> std::pin::Pin<
                        Box<dyn std::future::Future<Output = Result<String, String>> + Send>,
                    > {
                        let registry = registry_clone.clone();
                        let tool_ctx = tool_ctx_clone.clone();
                        let always_allowed = always_allowed_clone.clone();
                        let formatter = formatter_clone.clone();
                        let session_for_errors = session_for_tool.clone();
                        let provider_name_for_errors = provider_for_tool.clone();
                        Box::pin(async move {
                            let start_line =
                                formatter.lock().unwrap().tool_start_line(&name, &tool_args);
                            eprint!("{start_line}");
                            let _ = std::io::stderr().flush();

                            let is_always = always_allowed.lock().unwrap().contains(&name);
                            if !no_confirm && !is_always {
                                eprint!("\n{}", confirmation_prompt(&name, &tool_args));
                                let _ = std::io::stderr().flush();
                                let mut input = String::new();
                                std::io::stdin().read_line(&mut input).ok();
                                match parse_confirmation_input(&input) {
                                    ToolConfirmationDecision::Deny => {
                                        return Err("user denied".to_string());
                                    }
                                    ToolConfirmationDecision::AllowAlways => {
                                        always_allowed.lock().unwrap().insert(name.clone());
                                    }
                                    ToolConfirmationDecision::AllowOnce => {}
                                }
                            }
                            let result = registry
                                .execute(&name, tool_args, &tool_ctx)
                                .await
                                .map_err(|e| e.to_string());

                            let result_line =
                                formatter.lock().unwrap().tool_result_line(&name, &result);
                            eprint!("{result_line}");
                            let _ = std::io::stderr().flush();

                            if let Err(err) = &result {
                                eprintln!(
                                    "{}",
                                    render_error_message(
                                        err,
                                        &ErrorContext {
                                            session_id: session_for_errors.clone(),
                                            provider_name: provider_name_for_errors.clone(),
                                        }
                                    )
                                );
                            }

                            result
                        })
                    };

                    print!("Agent: ");
                    let _ = std::io::stdout().flush();
                    let formatter_for_delta = formatter.clone();
                    let progress_for_delta = Arc::new(Mutex::new(StreamProgress::new()));
                    let progress_for_report = progress_for_delta.clone();

                    let outcome = handle.block_on(async {
                        cmd.run_with_metrics(
                            session_id.clone(),
                            persona_once.take(),
                            input,
                            tools,
                            &(|content: String| {
                                let rendered = formatter_for_delta
                                    .lock()
                                    .unwrap()
                                    .push_delta(content.as_str());
                                print!("{}", rendered);
                                let _ = std::io::stdout().flush();
                                if let Some(progress_line) =
                                    progress_for_delta.lock().unwrap().on_delta(content.as_str())
                                {
                                    eprint!("\r{progress_line}");
                                    let _ = std::io::stderr().flush();
                                }
                            }),
                            &execute_tool,
                        )
                        .await
                        .with_context(|| {
                            format!(
                                "error_context provider={} session={}",
                                provider_for_context, session_for_context
                            )
                        })
                    })?;

                    let output_tokens = outcome
                        .metrics
                        .last_usage
                        .as_ref()
                        .map(|usage| usage.completion_tokens as usize)
                        .unwrap_or(outcome.metrics.estimated_output_tokens);
                    let progress_line = progress_for_report.lock().unwrap().render_line(output_tokens);

                    println!();
                    eprintln!("\r{progress_line}");
                    session_id = Some(outcome.session_id.to_string());
                }
                Err(ReadlineError::Eof) | Err(ReadlineError::Interrupted) => break,
                Err(e) => return Err(e.into()),
            }
        }

        if let Some(id) = &session_id {
            eprintln!("\n[session: {id}]");
        }
        Ok(())
    })
    .await??;

    Ok(())
}
