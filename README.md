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

## 5. Gateway daemon

`ccode-gateway` is a long-running HTTP daemon that receives messages from messaging platforms (Telegram, Discord) and drives the agent.

### Install

```sh
cargo install --path crates/gateway --force
```

### Configure

Add a `[gateway]` section to `~/.ccode/config.toml`:

```toml
[gateway]
port    = 7001          # optional, default 7001
workdir = "/your/work"  # optional, directory the agent operates in

[gateway.telegram]
bot_token      = "123456:ABC-DEF..."   # from @BotFather
mode           = "webhook"             # "webhook" (default) or "long_polling"
webhook_secret = "my-secret"           # optional, webhook mode only

[gateway.discord]
application_public_key = "abcdef..."   # from Discord Developer Portal → General Information
bot_token              = "Bot xxxx"    # optional, needed only for follow-up messages
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
5. Create a slash command (e.g. `/ask`) with a required string option named `prompt`.
6. Users invoke `/ask prompt:your question` — the gateway runs the agent and replies inline.

### Session continuity

- **Discord**: each channel maintains its own session (keyed by `channel_id`), so the agent remembers context across messages in the same channel.
- **Telegram**: each message starts a new session (stateless by default).
