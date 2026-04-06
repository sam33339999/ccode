# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Language

**必須使用繁體中文回應所有訊息。**

## Commands

```sh
# Install binary
cargo install --path crates/cli --force

# Build & check
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace

# Run a single test
cargo test -p <crate-name> <test_name>

# Run architecture enforcement test
cargo test -p ccode-bootstrap --test workspace_architecture

# Run acceptance parity gate
cargo test -p ccode-bootstrap --test acceptance_parity_gate

# Run spec governance gate
cargo test -p ccode-bootstrap --test spec_governance_gate
```

## Architecture

This is a Rust Cargo workspace implementing a layered, hexagonal architecture for an AI agent CLI.

### Crate Dependency Layers (strict one-way)

```
domain → ports → application → (provider / tools / mcp-runtime / remote-runtime / session / cron / state-store) → bootstrap → cli
```

- **`domain`** — pure business types and policies; zero internal workspace dependencies
- **`ports`** — trait boundaries (repository, provider, event bus traits); depends only on `domain`
- **`application`** — use-case orchestration and service implementations; depends only on `ports` and `domain`
- **`provider`** — LLM provider adapters (Anthropic, OpenAI-compat, ZhipuAI, llamacpp, OpenRouter); implements `ports` traits
- **`tools`** — tool lifecycle and execution (fs, shell, web, agent spawn, MCP, cron)
- **`mcp-runtime`** — MCP protocol handling
- **`remote-runtime`** — remote session / WebSocket / bridge behavior
- **`session`** — session persistence and management
- **`cron`** — scheduled agent job runtime
- **`state-store`** — local/team memory persistence adapters
- **`config`** — environment parsing and configurable defaults
- **`platform`** — fs/process/network OS abstractions
- **`bootstrap`** — composition root; wires all crates together; hosts architecture enforcement tests
- **`cli`** — thin binary entry point using `clap`; subcommands: `health`, `sessions`, `agent`, `repl`, `tui`, `cron`

### Architecture Enforcement

Forbidden dependency edges are enforced by `crates/bootstrap/tests/workspace_architecture.rs` (runs via `cargo metadata`). Never introduce edges that violate the layer rules above.

### Key Patterns

- **Commands/Queries split**: `crates/application/src/commands/` (writes) vs `crates/queries/` (reads)
- **Acceptance tests live in `application`**: `agent_triggers_acceptance_tests.rs`, `bridge_mode_acceptance_tests.rs`, `coordinator_mode_acceptance_tests.rs`, `ultraplan_acceptance_tests.rs`, etc.
- **Spec contracts**: `crates/application/src/spec_contracts.rs` — do not break these
- **Output rendering**: `crates/cli/src/cmd/output.rs` — centralized terminal output and error formatting
- **Provider routing**: `crates/provider/src/router.rs` and `factory.rs` — provider selection logic

### macOS Binary Install Note

After `cargo install`, macOS Gatekeeper may kill the binary silently. Fix:
```sh
sudo spctl --add /usr/local/bin/ccode
```
