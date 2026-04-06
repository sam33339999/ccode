# US-005 Discord Interactions Handler Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在 gateway 新增 Discord interactions handler，含 Ed25519 驗證與 slash command 回覆。

**Architecture:** 在 `adapters/discord.rs` 直接處理原始 body bytes，先驗證 Discord 簽名再 JSON 反序列化，依 interaction type 回覆 PONG 或代理到 `agent_bridge`。在 `server` router 註冊 `/webhook/discord` 並從 config 讀取 `application_public_key`。

**Tech Stack:** Rust, axum, serde, ed25519-dalek, hex, tokio tests.

---

### Task 1: RED 測試

**Files:**
- Create: `crates/gateway/src/adapters/discord.rs`

1. 寫 `verify_signature` 單元測試（有效簽名通過、錯誤簽名失敗）。
2. 寫 `handle` 的 PING 測試（type=1 回 `{"type":1}`）。
3. 執行 `cargo test -p ccode-gateway discord`，確認先失敗。

### Task 2: GREEN 最小實作

**Files:**
- Modify: `crates/gateway/src/adapters/mod.rs`
- Modify: `crates/gateway/src/main.rs`
- Modify: `crates/gateway/src/adapters/discord.rs`

1. 新增 `DiscordInteraction` 與 `DiscordInteractionData` 型別。
2. 以 `Bytes` 取 body，讀 `X-Signature-Ed25519`、`X-Signature-Timestamp`。
3. 以 `VerifyingKey::from_bytes` 驗證 `timestamp + body`。
4. 實作 `PING` 回 `{"type":1}`。
5. 實作 `APPLICATION_COMMAND` 擷取文字並呼叫 `agent_bridge::run_agent`，回 `{"type":4,"data":{"content":"..."}}`。
6. 新增 router 路徑 `/webhook/discord`。

### Task 3: 驗證與提交

1. `cargo build -p ccode-gateway`
2. `cargo fmt --check`
3. `cargo clippy --workspace -- -D warnings`
4. `git commit -m "feat: US-005 - 實作 Discord interactions handler（含 Ed25519 驗證）"`
