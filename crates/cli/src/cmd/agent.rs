use anyhow::Context;
use ccode_application::commands::agent_run::AgentRunCommand;
use clap::Args;
use std::collections::HashSet;
use std::io::Write;
use std::sync::{Arc, Mutex};

use super::output::{
    ErrorContext, StreamFormatter, StreamProgress, ToolConfirmationDecision, confirmation_prompt,
    parse_confirmation_input, render_error_message,
};

#[derive(Args)]
pub struct AgentArgs {
    /// Resume an existing session by ID
    #[arg(short, long)]
    pub session: Option<String>,
    /// Message to send to the agent
    #[arg(short, long)]
    pub message: String,
    /// System prompt / persona for the agent (e.g. "You are a senior Rust engineer")
    #[arg(long)]
    pub persona: Option<String>,
    /// Skip tool confirmation prompts
    #[arg(long)]
    pub no_confirm: bool,
}

pub async fn run(args: AgentArgs) -> anyhow::Result<()> {
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
    let session_for_errors = args.session.clone().unwrap_or_else(|| "new".to_string());
    let provider_name_for_tools = provider_name_for_errors.clone();
    let session_for_tools = session_for_errors.clone();

    let tool_ctx = state.tool_ctx();
    let tool_definitions = state.tool_registry.definitions();
    let registry = Arc::clone(&state.tool_registry);

    eprintln!(
        "[tools: {}]",
        tool_definitions
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let cmd = AgentRunCommand::new(state.session_repo, provider).with_context(state.context_policy);

    // Track which tools the user has permanently allowed
    let always_allowed: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let no_confirm = args.no_confirm;
    let formatter = Arc::new(Mutex::new(StreamFormatter::new()));
    let progress = Arc::new(Mutex::new(StreamProgress::new()));

    let on_delta_formatter = Arc::clone(&formatter);
    let on_delta_progress = Arc::clone(&progress);
    let on_delta = move |content: String| {
        let rendered = on_delta_formatter
            .lock()
            .unwrap()
            .push_delta(content.as_str());
        print!("{}", rendered);
        let _ = std::io::stdout().flush();
        if let Some(progress_line) = on_delta_progress.lock().unwrap().on_delta(content.as_str()) {
            eprint!("\r{progress_line}");
            let _ = std::io::stderr().flush();
        }
    };

    let registry = Arc::new(registry);
    let always_allowed_clone = always_allowed.clone();
    let tool_ctx = Arc::new(tool_ctx);
    let tool_formatter = Arc::clone(&formatter);

    let execute_tool = move |name: String,
                             args: serde_json::Value|
          -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<String, String>> + Send>,
    > {
        let registry = registry.clone();
        let always_allowed = always_allowed_clone.clone();
        let tool_ctx = tool_ctx.clone();
        let formatter = tool_formatter.clone();
        let no_confirm = no_confirm;
        let provider_name_for_errors = provider_name_for_tools.clone();
        let session_for_errors = session_for_tools.clone();
        Box::pin(async move {
            let start_line = formatter.lock().unwrap().tool_start_line(&name, &args);
            eprint!("{start_line}");
            let _ = std::io::stderr().flush();

            // Check if already in always-allowed set
            let is_always = always_allowed.lock().unwrap().contains(&name);

            if !no_confirm && !is_always {
                eprint!("{}", confirmation_prompt(&name, &args));
                let _ = std::io::stderr().flush();

                let mut input = String::new();
                std::io::stdin().read_line(&mut input).ok();
                match parse_confirmation_input(&input) {
                    ToolConfirmationDecision::Deny => return Err("user denied".to_string()),
                    ToolConfirmationDecision::AllowAlways => {
                        always_allowed.lock().unwrap().insert(name.clone());
                    }
                    ToolConfirmationDecision::AllowOnce => {}
                }
            }

            let result = registry
                .execute(&name, args, &tool_ctx)
                .await
                .map_err(|e| e.to_string());

            let result_line = formatter.lock().unwrap().tool_result_line(&name, &result);
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

    let outcome = cmd
        .run_with_metrics(
            args.session,
            args.persona,
            args.message,
            tool_definitions,
            &on_delta,
            &execute_tool,
        )
        .await
        .with_context(|| {
            format!(
                "error_context provider={} session={}",
                provider_name_for_errors, session_for_errors
            )
        })?;

    let output_tokens = outcome
        .metrics
        .last_usage
        .as_ref()
        .map(|usage| usage.completion_tokens as usize)
        .unwrap_or(outcome.metrics.estimated_output_tokens);
    let progress_line = progress.lock().unwrap().render_line(output_tokens);

    println!();
    eprintln!("\r{progress_line}");
    eprintln!("[session: {}]", outcome.session_id);
    Ok(())
}
