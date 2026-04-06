# Gateway Webhook Daemon — Telegram & Discord

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 新增 `crates/gateway/` binary crate，作為 long-running HTTP daemon，接收來自 Telegram 和 Discord 的 webhook/interaction 訊息，驅動 `AgentRunCommand` 執行並回覆。

**Architecture:** `gateway` 與 `cli` 並列，同樣依賴 `ccode-bootstrap`。Axum 作為 HTTP server（已是 workspace 依賴）。設定透過 `~/.ccode/config.toml` 的 `[gateway]` section 管理，CLI flag 可覆蓋 port 和 workdir。Discord 需要 Ed25519 簽名驗證（新增 `ed25519-dalek` 依賴）。

**Config 範例：**
```toml
[gateway]
port = 8080
workdir = "/Users/yaxin/workspace"

[gateway.telegram]
bot_token = "xxxx:yyyy"
webhook_secret = "my-secret"   # 可選，驗證 X-Telegram-Bot-Api-Secret-Token

[gateway.discord]
application_public_key = "abcdef..."  # Discord Developer Portal 的 Ed25519 公鑰
bot_token = "Bot xxxx"               # 可選，用於 follow-up 訊息
```

**Tech Stack:** Rust (`axum 0.7`, `tokio`, `ed25519-dalek 2`, `reqwest`)，現有 `AgentRunCommand`，現有 `wire_from_config_with_cwd`。

---

### Task 1: 擴展 Config schema 支援 gateway 設定

**Files:**
- Modify: `crates/config/src/schema.rs`

1. 在 `Config` struct 新增 `pub gateway: Option<GatewayConfig>` 欄位。
2. 新增 `GatewayConfig` struct（`port: Option<u16>`, `workdir: Option<String>`, `telegram: Option<TelegramConfig>`, `discord: Option<DiscordConfig>`）。
3. 新增 `TelegramConfig` struct（`bot_token: String`, `webhook_secret: Option<String>`）。
4. 新增 `DiscordConfig` struct（`application_public_key: String`, `bot_token: Option<String>`）。
5. 執行 `cargo test -p ccode-config` 確認現有測試通過。

### Task 2: 建立 gateway crate 骨架與 main.rs

**Files:**
- Create: `crates/gateway/Cargo.toml`
- Create: `crates/gateway/src/main.rs`
- Modify: `Cargo.toml` (workspace root) — 加入 `"crates/gateway"` 至 `members`

1. `Cargo.toml` 依賴：`ccode-bootstrap`, `ccode-config`, `axum`, `tokio`（`rt-multi-thread`+`macros`+`signal`）, `serde`, `serde_json`, `tracing`, `tracing-subscriber`, `thiserror`, `anyhow`, `reqwest`, `clap`, `ed25519-dalek = "2"`, `hex = "0.4"`。
2. `main.rs`：用 `clap` 解析 `--port: Option<u16>` 和 `--workdir: Option<String>` flag。
3. 讀取 config：`let cfg = ccode_config::load()?`，從 `cfg.gateway` 取出設定；CLI flag 覆蓋 config 的 port/workdir。
4. 呼叫 `wire_from_config_with_cwd(workdir_override)` 初始化 `AppState`。
5. 呼叫 `server::start(state, port, cfg.gateway).await`。
6. 執行 `cargo build -p ccode-gateway` 確認骨架可編譯。

### Task 3: 實作 agent_bridge — 將訊息橋接到 AgentRunCommand

**Files:**
- Create: `crates/gateway/src/agent_bridge.rs`

1. 定義 `pub async fn run_agent(state: &AppState, text: String, session_id: Option<String>) -> anyhow::Result<String>`。
2. 參考 `crates/cli/src/cmd/agent.rs` 的模式：建立 `AgentRunCommand::new(state.session_repo.clone(), provider).with_context(state.context_policy.clone())`。
3. `on_delta`：收集 delta 字串到 `Arc<Mutex<String>>` buffer，不直接 print。
4. `execute_tool`：無 confirmation prompt，直接 `registry.execute(&name, args, &tool_ctx).await`。
5. 呼叫 `cmd.run_with_metrics(session_id, None, text, tool_definitions, &on_delta, &execute_tool).await`。
6. 回傳 buffer 中收集到的完整 response 字串。

### Task 4: 實作 adapters/telegram — Webhook handler

**Files:**
- Create: `crates/gateway/src/adapters/mod.rs`
- Create: `crates/gateway/src/adapters/telegram.rs`

1. 定義最小必要的 Telegram 型別：`TelegramUpdate`, `TelegramMessage`, `TelegramChat`（用 `serde_json::Value` 處理非關鍵欄位）。
2. `handle` 函數：`axum::extract::{State<Arc<AppState>>, Json<TelegramUpdate>, TypedHeader<...>}`。
3. 若設定了 `webhook_secret`，驗證 `X-Telegram-Bot-Api-Secret-Token` header；不符合回 `401`。
4. 取出 `update.message.text` 和 `chat.id`；非文字訊息直接回 `200 OK`（忽略）。
5. 呼叫 `agent_bridge::run_agent(&state, text, None).await`。
6. 用 `reqwest` 呼叫 `https://api.telegram.org/bot{token}/sendMessage`，payload：`{"chat_id": ..., "text": ...}`。
7. 回傳 `200 OK`。

### Task 5: 實作 adapters/discord — Interactions handler

**Files:**
- Create: `crates/gateway/src/adapters/discord.rs`

1. 定義必要型別：`DiscordInteraction`（`type: u8`, `data: Option<DiscordInteractionData>`, `token: String`, `channel_id: Option<String>`）。
2. **Ed25519 簽名驗證**（Discord 強制要求）：
   - 從 header 取 `X-Signature-Ed25519`（hex 解碼）和 `X-Signature-Timestamp`。
   - 用 `ed25519_dalek::VerifyingKey::from_bytes` 載入 `application_public_key`。
   - 驗證 `timestamp + body` 的簽名；失敗回 `401`。
   - 注意：需要在 axum handler 中取得原始 body bytes（用 `axum::body::Bytes` extractor，不能先用 `Json`）。
3. `type == 1`（PING）：直接回 `{"type": 1}`（PONG）。
4. `type == 2`（APPLICATION_COMMAND）：取出 `data.options[0].value` 或用 command name 作為 text。
5. 呼叫 `agent_bridge::run_agent` 取得回應。
6. 回覆 `{"type": 4, "data": {"content": "..."}}` (`CHANNEL_MESSAGE_WITH_SOURCE`)。

### Task 6: 實作 server.rs — Axum router 與 graceful shutdown

**Files:**
- Create: `crates/gateway/src/server.rs`

1. `pub async fn start(state: AppState, port: u16, gateway_cfg: Option<GatewayConfig>) -> anyhow::Result<()>`。
2. 建立 `Arc<AppState>` 作為 Axum shared state。
3. Router：
   - `GET  /health` → `200 OK` + `"ok"`
   - `POST /webhook/telegram` → `adapters::telegram::handle`（條件：`gateway_cfg.telegram.is_some()`，否則回 `404`）
   - `POST /webhook/discord`  → `adapters::discord::handle`（條件：`gateway_cfg.discord.is_some()`，否則回 `404`）
4. Graceful shutdown：`tokio::select!` on `ctrl_c()` 和 UNIX `SIGTERM`（`#[cfg(unix)]`）。
5. `tracing::info!("gateway listening on :{}", port)` 啟動日誌。

### Task 7: 驗證與收尾

**Files:**
- Verify only

1. `cargo fmt --check`
2. `cargo clippy --workspace -- -D warnings`
3. `cargo test -p ccode-bootstrap --test workspace_architecture`（確認架構 enforcement 無違規）
4. `cargo build -p ccode-gateway`
5. 手動測試：
   ```bash
   ccode-gateway --port 8080
   curl http://localhost:8080/health
   curl -X POST http://localhost:8080/webhook/telegram \
     -H "Content-Type: application/json" \
     -d '{"update_id":1,"message":{"chat":{"id":123},"text":"hello"}}'
   ```
