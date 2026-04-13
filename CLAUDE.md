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

## Configuration

### 設定檔優先順序（Layered Config）

`ccode` 使用兩層設定檔，專案層優先於全域層：

```
.ccode/config.toml          ← 專案目錄（優先）
~/.ccode/config.toml        ← 全域使用者設定（備用）
```

兩層會深度合併（deep-merge）：專案層只需寫需要覆蓋的欄位，其餘繼承全域設定。

### Persona（系統提示詞）

可在 `config.toml` 的頂層設定 `persona`，作為每個新 session 的預設 system prompt。

**單行：**
```toml
persona = "You are a senior Rust engineer. Reply in Traditional Chinese."
```

**多行（TOML triple-quote，建議用於較長的提示詞）：**
```toml
persona = """
You are a senior Rust engineer with deep expertise in async Rust and
the Tokio ecosystem. When reviewing code:
- Point out correctness issues first
- Then suggest performance improvements
- Always reply in Traditional Chinese
"""
```

**CLI 旗標（優先於 config）：**
```sh
ccode agent  --persona "You are a Rust expert" --message "review this"
ccode tui    --persona "You are a Rust expert"
ccode repl   --persona "You are a Rust expert"   # pipe 模式有效；TTY 模式轉交 TUI
```

**優先順序：**
```
--persona CLI 旗標  >  .ccode/config.toml [persona]  >  ~/.ccode/config.toml [persona]
```

Persona 僅在新 session 的**第一輪**注入，不會重複插入。
