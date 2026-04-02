use std::collections::HashSet;
use std::io::Write;
use std::sync::{Arc, Mutex};
use clap::Args;
use rustyline::{DefaultEditor, error::ReadlineError};
use ccode_application::commands::agent_run::AgentRunCommand;
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
    let state = ccode_bootstrap::wire_from_config_with_cwd(
        std::env::current_dir().ok(),
    )
    .map_err(|e| anyhow::anyhow!("bootstrap error: {e}"))?;

    let provider = state
        .provider.clone()
        .ok_or_else(|| anyhow::anyhow!("no LLM provider configured — set OPENROUTER_API_KEY"))?;

    eprintln!("[provider: {} | model: {}]", provider.name(), provider.default_model());

    let registry = Arc::clone(&state.tool_registry);
    let tool_ctx = Arc::new(state.tool_ctx());
    let tool_definitions = registry.definitions();

    eprintln!("[tools: {}]", tool_definitions.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "));
    eprintln!("[type 'exit' or press Ctrl+D to quit]\n");

    let cmd = Arc::new(AgentRunCommand::new(state.session_repo, provider)
        .with_context(state.context_policy));
    let mut session_id: Option<String> = args.session;
    let persona = args.persona;
    let no_confirm = args.no_confirm;

    let handle = tokio::runtime::Handle::current();

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut editor = DefaultEditor::new()?;
        // Persists across all turns within this REPL session
        let always_allowed: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
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

                    let execute_tool = move |name: String, tool_args: serde_json::Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>> {
                        let registry = registry_clone.clone();
                        let tool_ctx = tool_ctx_clone.clone();
                        let always_allowed = always_allowed_clone.clone();
                        Box::pin(async move {
                            let is_always = always_allowed.lock().unwrap().contains(&name);
                            if !no_confirm && !is_always {
                                eprint!("\n[tool: {}] args: {}\nAllow? [y/n/always]: ", name, tool_args);
                                let _ = std::io::stderr().flush();
                                let mut input = String::new();
                                std::io::stdin().read_line(&mut input).ok();
                                match input.trim().to_lowercase().as_str() {
                                    "n" | "no" => return Err("user denied".to_string()),
                                    "always" => { always_allowed.lock().unwrap().insert(name.clone()); }
                                    _ => {}
                                }
                            }
                            registry.execute(&name, tool_args, &tool_ctx)
                                .await
                                .map_err(|e| e.to_string())
                        })
                    };

                    print!("Agent: ");
                    let _ = std::io::stdout().flush();

                    let sid = handle.block_on(cmd.run(
                        session_id.clone(),
                        persona_once.take(),
                        input,
                        tools,
                        &(|content: String| {
                            print!("{}", content);
                            let _ = std::io::stdout().flush();
                        }),
                        &execute_tool,
                    ))?;

                    println!();
                    session_id = Some(sid.to_string());
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
