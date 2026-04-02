use ccode_application::commands::agent_run::AgentRunCommand;
use clap::Args;
use std::collections::HashSet;
use std::io::Write;
use std::sync::{Arc, Mutex};

use super::output::{StreamFormatter, classify_error, error_category_label};

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
    let state = ccode_bootstrap::wire_from_config_with_cwd(std::env::current_dir().ok())
        .map_err(|e| anyhow::anyhow!("bootstrap error: {e}"))?;

    let provider = state
        .provider
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no LLM provider configured — set OPENROUTER_API_KEY"))?;

    eprintln!(
        "[provider: {} | model: {}]",
        provider.name(),
        provider.default_model()
    );

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

    let on_delta_formatter = Arc::clone(&formatter);
    let on_delta = move |content: String| {
        let rendered = on_delta_formatter
            .lock()
            .unwrap()
            .push_delta(content.as_str());
        print!("{}", rendered);
        let _ = std::io::stdout().flush();
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
        Box::pin(async move {
            let start_line = formatter.lock().unwrap().tool_start_line(&name, &args);
            eprint!("{start_line}");
            let _ = std::io::stderr().flush();

            // Check if already in always-allowed set
            let is_always = always_allowed.lock().unwrap().contains(&name);

            if !no_confirm && !is_always {
                eprint!("[tool: {}] args: {}\nAllow? [y/n/always]: ", name, args);
                let _ = std::io::stderr().flush();

                let mut input = String::new();
                std::io::stdin().read_line(&mut input).ok();
                let input = input.trim().to_lowercase();

                match input.as_str() {
                    "n" | "no" => return Err("user denied".to_string()),
                    "always" => {
                        always_allowed.lock().unwrap().insert(name.clone());
                    }
                    _ => {} // "y" or anything else = allow
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
                let category = error_category_label(classify_error(err));
                eprintln!("[error:{category}] {err}");
            }

            result
        })
    };

    let session_id = cmd
        .run(
            args.session,
            args.persona,
            args.message,
            tool_definitions,
            &on_delta,
            &execute_tool,
        )
        .await?;

    println!();
    eprintln!("[session: {}]", session_id);
    Ok(())
}
