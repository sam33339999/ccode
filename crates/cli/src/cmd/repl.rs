use anyhow::Context;
use ccode_application::commands::agent_run::AgentRunCommand;
use clap::Args;
use rustyline::{DefaultEditor, error::ReadlineError};
use std::collections::HashSet;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::output::{
    AI_RESPONSE, ErrorContext, GRAY, RESET, StreamFormatter, StreamProgress, ThinkingSpinner,
    ToolConfirmationDecision, confirmation_prompt, parse_confirmation_input, render_error_message,
};
use ccode_application::commands::agent_run::estimate_tokens_from_chars;
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
        "{GRAY}[provider: {} | model: {}]{RESET}",
        provider.name(),
        provider.default_model()
    );
    let provider_name_for_errors = provider.name().to_string();

    let registry = Arc::clone(&state.tool_registry);
    let tool_ctx = Arc::new(state.tool_ctx());
    let tool_definitions = registry.definitions();

    eprintln!(
        "{GRAY}[tools: {}]{RESET}",
        tool_definitions
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    eprintln!("{GRAY}[type 'exit' or Ctrl+D to quit | /help for commands]{RESET}\n");

    let cmd = Arc::new(
        AgentRunCommand::new(state.session_repo, provider).with_context(state.context_policy),
    );
    let mut session_id: Option<String> = args.session;
    let persona = ccode_bootstrap::skill::augment_with_skill_catalog(args.persona, &state.skill_catalog);
    let no_confirm = args.no_confirm;
    let skills = state.skills.clone();

    let handle = tokio::runtime::Handle::current();

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut editor = DefaultEditor::new()?;
        // Persists across all turns within this REPL session
        let always_allowed: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let formatter: Arc<Mutex<StreamFormatter>> = Arc::new(Mutex::new(StreamFormatter::new()));
        // Persona is applied only on the first turn (new session); subsequent turns pass None
        let mut persona_once: Option<String> = persona;
        let skills = skills;

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
                    match input.as_str() {
                        "/help" => {
                            eprintln!("{GRAY}Commands:");
                            eprintln!("  /clear       — start a new session (discard history)");
                            eprintln!("  /compact     — compress session history, show token savings");
                            eprintln!("  /help        — show this list");
                            eprintln!("  /skill-name  — activate an Agent Skill by name");
                            eprintln!("  exit         — quit{RESET}");
                            if !skills.is_empty() {
                                let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
                                eprintln!("{GRAY}Available skills: {}{RESET}", names.join(", "));
                            }
                            continue;
                        }
                        "/clear" => {
                            session_id = None;
                            eprintln!("{GRAY}[clear] session reset — next message starts fresh{RESET}");
                            continue;
                        }
                        "/compact" => {
                            if let Some(ref sid) = session_id {
                                match handle.block_on(cmd.compact_session(sid)) {
                                    Ok(outcome) => {
                                        let before_tok = estimate_tokens_from_chars(outcome.before_chars);
                                        let after_tok = estimate_tokens_from_chars(outcome.after_chars);
                                        eprintln!(
                                            "{GRAY}[compact] msgs {before}→{after} | ~{before_tok}→{after_tok} tok (saved ~{saved} tok){RESET}",
                                            before = outcome.before_messages,
                                            after = outcome.after_messages,
                                            saved = before_tok.saturating_sub(after_tok),
                                        );
                                    }
                                    Err(e) => eprintln!("{GRAY}[compact] failed: {e}{RESET}"),
                                }
                            } else {
                                eprintln!("{GRAY}[compact] no active session{RESET}");
                            }
                            continue;
                        }
                        _ => {}
                    }

                    // User-explicit skill activation: /skill-name
                    let input = if input.starts_with('/')
                        && !matches!(input.as_str(), "/help" | "/clear" | "/compact")
                    {
                        let skill_name = input[1..].trim();
                        match ccode_bootstrap::skill::load_skill_body(skill_name, &skills) {
                            Some(body) => {
                                eprintln!("{GRAY}[skill loaded: {skill_name}]{RESET}");
                                body
                            }
                            None => {
                                let available: Vec<&str> =
                                    skills.iter().map(|s| s.name.as_str()).collect();
                                if available.is_empty() {
                                    eprintln!("{GRAY}[no skills installed]{RESET}");
                                } else {
                                    eprintln!(
                                        "{GRAY}[skill not found: {skill_name} — available: {}]{RESET}",
                                        available.join(", ")
                                    );
                                }
                                continue;
                            }
                        }
                    } else {
                        input
                    };

                    let _ = editor.add_history_entry(&input);

                    // Shared stop flag: created before execute_tool so both the tool
                    // handler and on_delta can clear the spinner before writing output.
                    let spinner_stop = Arc::new(AtomicBool::new(false));
                    let spinner_stop_for_tool = spinner_stop.clone();

                    let registry_clone = registry.clone();
                    let tool_ctx_clone = tool_ctx.clone();
                    let tools = tool_definitions.clone();
                    let always_allowed_clone = always_allowed.clone();
                    let formatter_clone = formatter.clone();
                    let session_for_context =
                        session_id.clone().unwrap_or_else(|| "new".to_string());
                    let provider_for_context = provider_name_for_errors.clone();
                    let session_for_error = session_for_context.clone();
                    let provider_for_error = provider_for_context.clone();
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
                        let spinner_stop = spinner_stop_for_tool.clone();
                        Box::pin(async move {
                            // Stop the spinner before writing any tool output so the
                            // confirmation prompt is never overwritten by the animation.
                            ThinkingSpinner::clear_and_stop(&spinner_stop);

                            let start_line =
                                formatter.lock().unwrap().tool_start_line(&name, &tool_args);
                            eprint!("{GRAY}{start_line}{RESET}");
                            let _ = std::io::stderr().flush();

                            let is_always = always_allowed.lock().unwrap().contains(&name);
                            if !no_confirm && !is_always {
                                eprint!("\n{GRAY}{}{RESET}", confirmation_prompt(&name, &tool_args));
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
                            eprint!("{GRAY}{result_line}{RESET}");
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

                    let formatter_for_delta = formatter.clone();
                    let progress_for_delta = Arc::new(Mutex::new(StreamProgress::new()));
                    let progress_for_report = progress_for_delta.clone();

                    let spinner_stop_for_delta = spinner_stop.clone();
                    let _spinner = ThinkingSpinner::start_with(spinner_stop.clone());
                    let header_printed = Arc::new(AtomicBool::new(false));
                    let header_printed_clone = header_printed.clone();

                    let on_delta = move |content: String| {
                        if !header_printed_clone.swap(true, Ordering::Relaxed) {
                            ThinkingSpinner::clear_and_stop(&spinner_stop_for_delta);
                            print!("{AI_RESPONSE}Agent: ");
                            let _ = std::io::stdout().flush();
                        }
                        let rendered = formatter_for_delta
                            .lock()
                            .unwrap()
                            .push_delta(content.as_str());
                        print!("{AI_RESPONSE}{}{RESET}", rendered);
                        let _ = std::io::stdout().flush();
                        // Track chars for final stats without printing inline
                        let _ = progress_for_delta.lock().unwrap().on_delta(content.as_str());
                    };

                    let run_outcome = match handle.block_on(async {
                        let run_fut = cmd.run_with_metrics(
                            session_id.clone(),
                            persona_once.take(),
                            input,
                            tools,
                            &on_delta,
                            &execute_tool,
                        );

                        tokio::select! {
                            result = run_fut => {
                                result
                                    .map(Some)
                                    .with_context(|| format!(
                                        "error_context provider={} session={}",
                                        provider_for_context, session_for_context
                                    ))
                            }
                            _ = tokio::signal::ctrl_c() => {
                                println!("{RESET}");
                                eprintln!("{GRAY}^C [interrupted]{RESET}");
                                Ok(None)
                            }
                        }
                    }) {
                        Ok(outcome) => outcome,
                        Err(e) => {
                            println!("{RESET}");
                            eprintln!(
                                "{}",
                                render_error_message(
                                    &e.to_string(),
                                    &ErrorContext {
                                        session_id: session_for_error,
                                        provider_name: provider_for_error,
                                    }
                                )
                            );
                            continue;
                        }
                    };

                    if let Some(outcome) = run_outcome {
                        let output_tokens = outcome
                            .metrics
                            .last_usage
                            .as_ref()
                            .map(|usage| usage.completion_tokens as usize)
                            .unwrap_or(outcome.metrics.estimated_output_tokens);
                        let progress_line =
                            progress_for_report.lock().unwrap().render_line(output_tokens);

                        println!("{RESET}");
                        eprintln!("\r{GRAY}{progress_line}{RESET}");
                        if let Some(usage) = &outcome.metrics.last_usage {
                            eprintln!(
                                "{GRAY}[ctx] in={} out={} total={}{RESET}",
                                usage.prompt_tokens,
                                usage.completion_tokens,
                                usage.total_tokens
                            );
                        }
                        session_id = Some(outcome.session_id.to_string());
                    }
                }
                Err(ReadlineError::Eof) | Err(ReadlineError::Interrupted) => break,
                Err(e) => return Err(e.into()),
            }
        }

        if let Some(id) = &session_id {
            eprintln!("\n{GRAY}[session: {id}]{RESET}");
        }
        Ok(())
    })
    .await??;

    Ok(())
}
