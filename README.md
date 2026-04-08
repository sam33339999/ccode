# ccode Workspace

This repository is a Rust Cargo workspace for the `ccode` architecture.

```bash
CLAUDE_CODE_COORDINATOR_MODE=coordinator
```

## 0. Install

```sh
cargo install --path crates/cli --force
```

### macOS: binary killed immediately after install

After reinstalling, macOS Gatekeeper may block the binary with no error output:

```
[1]  12345 killed  ccode repl
```

**Diagnosis:**

```sh
spctl --assess --verbose /usr/local/bin/ccode
# → /usr/local/bin/ccode: rejected
```

The binary is `adhoc`-signed only (no Apple Developer Team ID). macOS treats a freshly installed binary as untrusted until it is explicitly allowed. The old binary worked because macOS had already recorded it as allowed — reinstalling resets that record.

**Fix:**

```sh
sudo spctl --add /usr/local/bin/ccode
```

Or: **System Settings → Privacy & Security → scroll down → "Allow Anyway"**

This is not caused by any dependency change. It happens every time the binary is reinstalled.

---

## 1. Build and test

```sh
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

## 2. Required workspace crates

The architecture baseline requires these 13 crates to exist in the workspace:

1. `domain` (`ccode-domain`)
2. `ports` (`ccode-ports`)
3. `config` (`ccode-config`)
4. `provider` (`ccode-provider`)
5. `tools` (`ccode-tools`)
6. `mcp-runtime` (`ccode-mcp-runtime`)
7. `remote-runtime` (`ccode-remote-runtime`)
8. `platform` (`ccode-platform`)
9. `application` (`ccode-application`)
10. `session` (`ccode-session`)
11. `cron` (`ccode-cron`)
12. `cli` (`ccode-cli`)
13. `state-store` (`ccode-state-store`)

`bootstrap` (`ccode-bootstrap`) is also part of the workspace as the composition root.

## 3. Architecture check

Dependency-direction validation is enforced by automated test using `cargo metadata`:

- Test: `crates/bootstrap/tests/workspace_architecture.rs`
- Run directly: `cargo test -p ccode-bootstrap --test workspace_architecture`

## 4. Dependency direction rules

The automated check encodes these rules:

- `domain` has zero internal workspace dependencies.
- `ports` depends only on `domain`.
- `application` depends only on `ports` and `domain`.
- `cli` depends only on `application` and `bootstrap` (for internal workspace crates).

Forbidden edges:

- `domain` must not depend on `ports`, `provider`, `tools`, or `cli`.
- `ports` must not depend on `provider`, `tools`, or `cli`.

---

## 5. Configuration reference

Config file path: `~/.ccode/config.toml` (or `~/.clawler/config.toml`).  
All `api_key` / `base_url` / `default_model` fields are optional — the corresponding env vars are noted inline.

```toml
# ── Routing ───────────────────────────────────────────────────────────────────
# Controls which LLM provider handles each request

[routing]
# Routing strategy
# "manual"         — always use default_provider (default)
# "failover"       — try next provider on failure
# "round_robin"    — distribute requests across all enabled providers
# "cost_optimized" — prefer the currently cheapest provider
strategy = "manual"

# Provider used when strategy = "manual", or the primary for failover
# Accepted values: "openrouter" | "zhipu" | "anthropic" | "llamacpp" | "openai"
default_provider = "openrouter"


# ── Providers ─────────────────────────────────────────────────────────────────

# OpenRouter (routes to many models; recommended starting point)
# env: OPENROUTER_API_KEY / OPENROUTER_BASE_URL / OPENROUTER_DEFAULT_MODEL
[providers.openrouter]
api_key       = "sk-or-v1-..."
default_model = "openai/gpt-4o-mini"
# base_url  = "https://openrouter.ai/api/v1"  # rarely needs changing; useful for local proxies
# site_url  = "https://yourapp.com"            # HTTP Referer header for OpenRouter attribution (optional)
# site_name = "My App"                         # X-Title header (optional)

# Zhipu AI (z.ai international, OpenAI-compatible)
# env: ZHIPU_API_KEY / ZHIPU_BASE_URL / ZHIPU_DEFAULT_MODEL
[providers.zhipu]
api_key       = "your-zhipu-api-key"
default_model = "glm-4-plus"
# base_url = "https://api.z.ai/api/paas/v4"   # rarely needs changing
# title    = "My App"                          # X-Title header (required by coding plan)

# Anthropic (native Messages API, not OpenAI-compat)
# env: ANTHROPIC_API_KEY / ANTHROPIC_BASE_URL / ANTHROPIC_DEFAULT_MODEL
[providers.anthropic]
api_key       = "sk-ant-api03-..."
default_model = "claude-opus-4-5"
# base_url = "https://api.anthropic.com/v1"   # rarely needs changing

# OpenAI
# env: OPENAI_API_KEY
[providers.openai]
api_key       = "sk-..."
default_model = "gpt-4o"

# llama.cpp local inference server
# Start with: llama-server -m your-model.gguf --port 8080
# env: LLAMACPP_API_KEY / LLAMACPP_BASE_URL / LLAMACPP_DEFAULT_MODEL
[providers.llamacpp]
# api_key       = ""                           # usually not needed (some auth setups require it)
# base_url      = "http://127.0.0.1:8080/v1"  # rarely needs changing
# default_model = "default"                    # ignored by the llama.cpp server


# ── Sandbox ───────────────────────────────────────────────────────────────────
# Controls which local operations the agent is allowed to perform.
# All permissions default to off.

[sandbox]
# cwd       = "~/projects/my-app"  # agent's default working directory (defaults to launch cwd)
# fs_read   = "none"               # file read permission: "any" | "cwd" (restrict to workdir) | "none"
# fs_write  = "none"               # file write permission: "any" | "cwd" (restrict to workdir) | "none"
# shell     = "none"               # shell execution: "any" | "none" | "git,cargo,ls" (comma-separated allowlist)
# web_fetch = false                # allow the agent to make outbound HTTP requests
# browser   = false                # allow the agent to control a browser (computer-use)


# ── MCP Servers ───────────────────────────────────────────────────────────────
# On startup, each server receives a tools/list call; returned tools are
# registered automatically into the tool table.

[mcp]
# Master switch for Chicago MCP high-privilege capabilities (default false)
enable_chicago_mcp_feature_gate = false
# Allow high-privilege computer-use tools from MCP servers (default false)
allow_privileged_computer_use   = false

# Add one [[mcp.servers]] block per server; name and command are required.
# [[mcp.servers]]
# name    = "filesystem"
# command = "node"
# args    = ["./mcp-filesystem-server.js", "--stdio"]
# declared_capabilities = ["standard"]  # optional: "privileged_computer_use"
# enable_computer_use   = false         # enable computer-use for this server specifically


# ── Memory ────────────────────────────────────────────────────────────────────
# Agent memory / knowledge store

[memory]
# Storage backend
# "fts5"   — local full-text index, no embedding required (default)
# "vector" — semantic search; requires an embedding provider below
backend = "fts5"

# SQLite database path (default: ~/.ccode/memory.db)
# db_path = "~/.ccode/memory.db"

# Embedding config — required when backend = "vector"
# [memory.embedding]
# provider = "openai"   # "openai" | "llamacpp" | "zhipu"

# OpenAI embedding
# env: OPENAI_API_KEY / OPENAI_BASE_URL / OPENAI_EMBEDDING_MODEL
# [memory.embedding.openai]
# api_key  = "sk-..."
# model    = "text-embedding-3-small"       # or "text-embedding-3-large"
# base_url = "https://api.openai.com/v1"

# llama.cpp local embedding (server must be started with --embedding flag)
# Start with: llama-server -m nomic-embed-text.gguf --port 8080 --embedding
# env: LLAMACPP_BASE_URL / LLAMACPP_EMBEDDING_MODEL
# [memory.embedding.llamacpp]
# base_url = "http://127.0.0.1:8080/v1"
# model    = "default"

# Zhipu AI embedding
# env: ZHIPU_API_KEY / ZHIPU_EMBEDDING_MODEL
# [memory.embedding.zhipu]
# api_key = "your-zhipu-api-key"
# model   = "embedding-3"


# ── Context Compression ───────────────────────────────────────────────────────
# Context window management. Local estimate: 4 chars ≈ 1 token.

[context]
# Model max context tokens; used with compress_threshold_ratio to compute the trigger point.
# max_context_tokens = 200000

# Compression trigger ratio against max_context_tokens.
# e.g. 0.8 = compress when estimated context reaches 80% of max_context_tokens.
# compress_threshold_ratio = 0.8

# Alternatively, set an absolute character threshold (overrides ratio calculation).
# Rough estimate: 600000 chars ≈ 150k tokens.
# compress_chars_threshold = 600000

# Number of most-recent messages to keep verbatim after compression (default: 8).
# keep_recent_messages = 8

# Truncate a single tool result that exceeds this many characters (default: 40000).
# tool_result_max_chars = 40000

# Maximum agentic loop iterations — each tool call counts as one round (default: 50).
# max_agent_iterations = 50

# Default max_tokens sent to the LLM per request.
# If unset, the provider's built-in default applies (e.g. 4096 for Anthropic).
# Local models should be set to 8192 or higher to avoid truncated replies.
# default_max_tokens = 16384


# ── Remote Runtime ────────────────────────────────────────────────────────────
# CCR HTTP remote execution environment connection parameters

[remote_runtime.ccr_http]
# Request timeout in milliseconds (default: 10000)
timeout_ms     = 10000
# Retry count on timeout or transient failure (default: 2)
max_retries    = 2
# Delay between retries in milliseconds (default: 200)
retry_delay_ms = 200


# ── TUI ───────────────────────────────────────────────────────────────────────
# Terminal UI display settings

[tui]
# Color theme
# "default"       — standard 16-color palette (default)
# "high_contrast" — bold + bright colors, for low-contrast displays
# "no_color"      — text modifiers only (no color); equivalent to setting NO_COLOR env var
# Automatically overridden to "no_color" when the NO_COLOR env var is set or TERM=dumb.
# theme = "default"


# ── Gateway Daemon ────────────────────────────────────────────────────────────
# HTTP service settings for ccode-gateway

[gateway]
# port    = 7001          # listening port (default: 7001)
# workdir = "/your/work"  # agent working directory (overrides global sandbox.cwd)

[gateway.telegram]
bot_token      = "123456:ABC-DEF..."   # from @BotFather
mode           = "webhook"             # "webhook" (default) or "long_polling"
# webhook_secret = "my-secret"         # optional; validates X-Telegram-Bot-Api-Secret-Token header in webhook mode

[gateway.discord]
application_public_key = "abcdef..."   # from Discord Developer Portal → General Information
# bot_token = "Bot xxxx"               # optional; required only for sending follow-up messages
```

---

## 6. Gateway daemon

`ccode-gateway` is a long-running HTTP daemon that receives messages from messaging platforms (Telegram, Discord) and drives the agent.

> For full config field reference see [§5 Configuration reference](#5-configuration-reference) under the `[gateway]` block.

### Install

```sh
cargo install --path crates/gateway --force
```

### Run

```sh
# use config file values
ccode-gateway

# override port and workdir at runtime
ccode-gateway --port 8080 --workdir /tmp/agent-work

# adjust log level
RUST_LOG=debug ccode-gateway
```

### Endpoints

| Method | Path                | Description                               |
|--------|---------------------|-------------------------------------------|
| GET    | `/health`           | Liveness check, returns `ok`              |
| POST   | `/webhook/telegram` | Telegram webhook (webhook mode only)      |
| POST   | `/webhook/discord`  | Discord interactions endpoint             |

Endpoints for unconfigured platforms return `404`.
In `long_polling` mode, `/webhook/telegram` is not registered — the gateway polls Telegram directly.

### Telegram: webhook vs long polling

> **Important:** Telegram does not allow webhook and long polling on the same bot token simultaneously.
> If a webhook is registered, `getUpdates` (long polling) will not receive new messages.
> To switch from webhook to long polling, delete the webhook first:
> ```sh
> curl "https://api.telegram.org/bot<TOKEN>/deleteWebhook"
> ```

**Webhook mode** — requires a public HTTPS URL; Telegram pushes updates to your server:
1. Create a bot with [@BotFather](https://t.me/BotFather) and copy the token.
2. Set `mode = "webhook"` (or omit, it is the default).
3. Start the gateway and register the webhook:
   ```sh
   curl "https://api.telegram.org/bot<TOKEN>/setWebhook" \
     -d "url=https://your-domain/webhook/telegram" \
     -d "secret_token=my-secret"
   ```

**Long polling mode** — no public URL needed; the gateway polls Telegram continuously.
Suitable for local development or environments without inbound HTTPS:
1. Delete any existing webhook (see above).
2. Set `mode = "long_polling"` in config.
3. Start the gateway — it begins polling immediately, no extra setup required.

### Discord setup

1. Go to [Discord Developer Portal](https://discord.com/developers/applications) → create an application.
2. Under **General Information**, copy the **Public Key**.
3. Under **Interactions Endpoint URL**, set `https://your-domain/webhook/discord`.
4. Discord sends a PING (type 1) on save — the gateway replies with PONG automatically.
5. Create a slash command (e.g. `/ask`) with:
   - required `STRING` option named `prompt`
   - optional `ATTACHMENT` option named `image`
6. Users invoke `/ask prompt:your question` (or include `image`) — the gateway runs the agent and replies inline.

### Session continuity

- **Discord**: each channel maintains its own session (keyed by `channel_id`), so the agent remembers context across messages in the same channel.
- **Telegram**: each message starts a new session (stateless by default).
