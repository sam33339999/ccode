# ccode Workspace

這個 repository 是 `ccode` 的 Rust Cargo workspace。

---

## 0. 安裝

```sh
# 安裝 CLI
cargo install --path crates/cli --force

# 安裝 Gateway daemon
cargo install --path crates/gateway --force
```

### macOS：安裝後 binary 被立即終止

重新安裝後，macOS Gatekeeper 可能會無聲地阻擋 binary：

```
[1]  12345 killed  ccode repl
```

**診斷：**

```sh
spctl --assess --verbose /usr/local/bin/ccode
# → /usr/local/bin/ccode: rejected
```

**修復：**

```sh
sudo spctl --add /usr/local/bin/ccode
sudo spctl --add /usr/local/bin/ccode-gateway
```

或：**系統設定 → 隱私權與安全性 → 向下捲動 → 「仍然允許」**

---

## 1. 建置與測試

```sh
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

---

## 2. Gateway Daemon

`ccode-gateway` 是一個長時間運行的 HTTP daemon，接收來自通訊平台（Telegram、Discord）的訊息並驅動 agent 執行。

### 設定

在 `~/.ccode/config.toml` 新增 `[gateway]` section：

```toml
[gateway]
port    = 7001           # 可選，預設 7001
workdir = "/your/work"   # 可選，agent 的工作目錄

[gateway.telegram]
bot_token      = "123456:ABC-DEF..."   # 從 @BotFather 取得
mode           = "webhook"             # "webhook"（預設）或 "long_polling"
webhook_secret = "my-secret"           # 可選，僅 webhook 模式使用

[gateway.discord]
application_public_key = "abcdef..."   # Discord Developer Portal → General Information
bot_token              = "Bot xxxx"    # 可選，需要發送後續訊息時使用
```

### 啟動

```sh
# 使用設定檔的值
ccode-gateway

# 啟動時覆蓋 port 和 workdir
ccode-gateway --port 8080 --workdir /tmp/agent-work

# 調整 log 等級
RUST_LOG=debug ccode-gateway
```

### API 端點

| 方法   | 路徑                  | 說明                                        |
|--------|-----------------------|---------------------------------------------|
| GET    | `/health`             | 存活檢查，回傳 `ok`                         |
| POST   | `/webhook/telegram`   | 接收 Telegram Bot webhook（webhook 模式）   |
| POST   | `/webhook/discord`    | Discord interactions 端點                   |

未設定的平台端點回傳 `404`。  
`long_polling` 模式下，`/webhook/telegram` 不會掛載，gateway 改為主動向 Telegram 輪詢。

### Telegram：Webhook vs Long Polling

> **重要限制：** Telegram 不允許同一個 bot token 同時使用 webhook 和 long polling。  
> 若已設定 webhook，`getUpdates`（long polling）就不會收到新訊息。  
> 要從 webhook 切換到 long polling，請先刪除 webhook：
> ```sh
> curl "https://api.telegram.org/bot<TOKEN>/deleteWebhook"
> ```

**Webhook 模式**：需要公開的 HTTPS URL，由 Telegram 主動推送訊息：
1. 用 [@BotFather](https://t.me/BotFather) 建立 bot，取得 token。
2. 設定 `mode = "webhook"`（或省略，這是預設值）。
3. 啟動 gateway，然後向 Telegram 註冊 webhook：
   ```sh
   curl "https://api.telegram.org/bot<TOKEN>/setWebhook" \
     -d "url=https://your-domain/webhook/telegram" \
     -d "secret_token=my-secret"
   ```

**Long Polling 模式**：不需要公開 URL，由 gateway 主動向 Telegram 持續輪詢。  
適合本機開發或沒有對外 HTTPS 的環境：
1. 先刪除已有的 webhook（見上方指令）。
2. 設定 `mode = "long_polling"`。
3. 啟動 gateway — 立即開始輪詢，不需要額外設定。

### Discord 設定步驟

1. 前往 [Discord Developer Portal](https://discord.com/developers/applications)，建立一個 Application。
2. 在 **General Information** 頁面複製 **Public Key**。
3. 在 **Interactions Endpoint URL** 填入 `https://your-domain/webhook/discord`，儲存。
   - Discord 會發送 PING（type 1）驗證端點，gateway 自動回應 PONG。
4. 在 **Slash Commands** 建立指令，例如 `/ask`，加上一個 `String` 類型的必填參數 `prompt`。
5. 使用者輸入 `/ask prompt:你的問題` → gateway 執行 agent → 直接在頻道回覆。

### 對話記憶（Session）

- **Discord**：以 `channel_id` 為 key，同一個頻道的對話共用 session，agent 記得上下文。
- **Telegram**：每則訊息都是獨立的新 session（無狀態）。

---

## 3. 架構概覽

```
domain → ports → application → (provider / tools / ...) → bootstrap
                                                               ↘ cli
                                                               ↘ gateway
```

`gateway` 和 `cli` 並列，都透過 `bootstrap` 接線，共用相同的 provider、tool registry 和 session 基礎設施。

### 架構檢查

```sh
cargo test -p ccode-bootstrap --test workspace_architecture
```
