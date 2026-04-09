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

> Gateway 的完整設定欄位說明見 [§3 設定參考](#3-設定參考) 中的 `[gateway]` 段落。

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
4. 在 **Slash Commands** 建立指令，例如 `/ask`，加入：
   - `String` 類型必填參數 `prompt`
   - `Attachment` 類型選填參數 `image`
5. 使用者輸入 `/ask prompt:你的問題`（可另外附上 `image`）→ gateway 執行 agent → 直接在頻道回覆。

### 對話記憶（Session）

- **Discord**：以 `channel_id` 為 key，同一個頻道的對話共用 session，agent 記得上下文。
- **Telegram**：每則訊息都是獨立的新 session（無狀態）。

---

## 3. 設定參考

設定檔路徑：`~/.ccode/config.toml`（或 `~/.clawler/config.toml`）。  
所有 `api_key` / `base_url` / `default_model` 欄位均可省略，改用環境變數替代（各欄位旁有標注）。

```toml
# ── Image ─────────────────────────────────────────────────────────────────────
# 影像前處理策略（供 pipeline 判斷是否 resize/quantize）

[image]
# strategy: "resize" | "quantize" | "none"
strategy = "resize"
# 圖片長邊最大尺寸（像素）
max_dimension = 2048

# ── Routing ───────────────────────────────────────────────────────────────────
# 控制 LLM 請求要路由到哪個服務商

[routing]
# 路由策略
# "manual"        — 固定使用 default_provider（預設）
# "failover"      — 主服務商失敗時依序嘗試其他服務商
# "round_robin"   — 輪流分配到所有已啟用的服務商
# "cost_optimized"— 優先選擇目前費率最低的服務商
strategy = "manual"

# strategy = "manual" 時使用的服務商，或 failover 的主服務商
# 可選值: "openrouter" | "zhipu" | "anthropic" | "llamacpp" | "openai" | "gemini"
default_provider = "openrouter"


# ── Providers ─────────────────────────────────────────────────────────────────

# OpenRouter（可路由到多種模型，建議優先使用）
# env: OPENROUTER_API_KEY / OPENROUTER_BASE_URL / OPENROUTER_DEFAULT_MODEL
[providers.openrouter]
api_key       = "sk-or-v1-..."
default_model = "openai/gpt-4o-mini"
# base_url  = "https://openrouter.ai/api/v1"  # 通常不需改；可改為本地代理
# site_url  = "https://yourapp.com"            # HTTP Referer header，用於 OpenRouter 流量歸因（選填）
# site_name = "My App"                         # X-Title header（選填）
# vision    = true                              # provider 是否支援圖片輸入（選填）
# context_window = 200000                       # 模型 context window（token，選填）

# 智谱 AI（z.ai 國際站，OpenAI-compatible）
# env: ZHIPU_API_KEY / ZHIPU_BASE_URL / ZHIPU_DEFAULT_MODEL
[providers.zhipu]
api_key       = "your-zhipu-api-key"
default_model = "glm-4-plus"
# base_url = "https://api.z.ai/api/paas/v4"   # 通常不需改
# title    = "My App"                          # X-Title header（coding 方案必填）
# vision   = true                               # provider 是否支援圖片輸入（選填）
# context_window = 128000                       # 模型 context window（token，選填）

# Anthropic（原生 Messages API，非 OpenAI-compat）
# env: ANTHROPIC_API_KEY / ANTHROPIC_BASE_URL / ANTHROPIC_DEFAULT_MODEL
[providers.anthropic]
api_key       = "sk-ant-api03-..."
default_model = "claude-opus-4-5"
# base_url = "https://api.anthropic.com/v1"    # 通常不需改
# vision   = true                                # provider 是否支援圖片輸入（選填）
# context_window = 200000                        # 模型 context window（token，選填）

# OpenAI
# env: OPENAI_API_KEY
[providers.openai]
api_key       = "sk-..."
default_model = "gpt-4o"
# vision       = true                            # provider 是否支援圖片輸入（選填）

# Gemini（Google AI Studio OpenAI-compatible endpoint）
# env: GEMINI_API_KEY / GEMINI_BASE_URL / GEMINI_DEFAULT_MODEL
[providers.gemini]
api_key       = "AIza..."
default_model = "gemini-2.5-flash"
# base_url     = "https://generativelanguage.googleapis.com/v1beta/openai"
# vision       = true                            # provider 是否支援圖片輸入（選填）
# context_window = 128000                        # 模型 context window（token，選填）

# llama.cpp 本地推理伺服器
# 啟動方式: llama-server -m your-model.gguf --port 8080
# env: LLAMACPP_API_KEY / LLAMACPP_BASE_URL / LLAMACPP_DEFAULT_MODEL
[providers.llamacpp]
# api_key      = ""                            # 通常不需要（部分 auth 設定除外）
# base_url     = "http://127.0.0.1:8080/v1"   # 通常不需改
# default_model = "default"                    # llama.cpp 伺服器端忽略此欄位
# vision       = false                          # provider 是否支援圖片輸入（選填）
# context_window = 8192                         # 模型 context window（token，選填）


# ── Sandbox ───────────────────────────────────────────────────────────────────
# 控制 agent 可執行哪些本地操作；所有權限預設關閉

[sandbox]
# cwd      = "~/projects/my-app"  # agent 預設工作目錄（未設定則沿用啟動時的 cwd）
# fs_read  = "none"               # 檔案讀取權限: "any" | "cwd"（限工作目錄）| "none"
# fs_write = "none"               # 檔案寫入權限: "any" | "cwd"（限工作目錄）| "none"
# shell    = "none"               # Shell 執行權限: "any" | "none" | "git,cargo,ls"（逗號分隔的 allowlist）
# web_fetch = false               # 允許 agent 發出 HTTP 請求
# browser  = false                # 允許 agent 控制瀏覽器（computer-use）


# ── MCP Servers ───────────────────────────────────────────────────────────────
# 啟動後對每個 server 呼叫 tools/list，並把回傳工具自動註冊到工具表

[mcp]
# Chicago MCP 高權限能力總開關（預設 false）
enable_chicago_mcp_feature_gate = false
# Policy 是否允許高權限 computer-use 工具（預設 false）
allow_privileged_computer_use   = false

# 可新增多個 server，每個 server 需有 name 與 command
# [[mcp.servers]]
# name    = "filesystem"
# command = "node"
# args    = ["./mcp-filesystem-server.js", "--stdio"]
# declared_capabilities = ["standard"]  # 可選: "privileged_computer_use"
# enable_computer_use   = false         # 是否對此 server 啟用 computer-use


# ── Memory ────────────────────────────────────────────────────────────────────
# Agent 記憶系統

[memory]
# 儲存後端
# "fts5"   — 純本地全文索引，不需 embedding（預設）
# "vector" — 語意搜尋，需設定下方 embedding provider
backend = "fts5"

# SQLite 資料庫路徑（預設 ~/.ccode/memory.db）
# db_path = "~/.ccode/memory.db"

# Embedding 設定（backend = "vector" 時必填）
# [memory.embedding]
# provider = "openai"   # "openai" | "llamacpp" | "zhipu"

# OpenAI embedding
# env: OPENAI_API_KEY / OPENAI_BASE_URL / OPENAI_EMBEDDING_MODEL
# [memory.embedding.openai]
# api_key  = "sk-..."
# model    = "text-embedding-3-small"        # 或 "text-embedding-3-large"
# base_url = "https://api.openai.com/v1"

# llama.cpp 本地 embedding（需以 --embedding 旗標啟動）
# 啟動方式: llama-server -m nomic-embed-text.gguf --port 8080 --embedding
# env: LLAMACPP_BASE_URL / LLAMACPP_EMBEDDING_MODEL
# [memory.embedding.llamacpp]
# base_url = "http://127.0.0.1:8080/v1"
# model    = "default"

# 智谱 AI embedding
# env: ZHIPU_API_KEY / ZHIPU_EMBEDDING_MODEL
# [memory.embedding.zhipu]
# api_key = "your-zhipu-api-key"
# model   = "embedding-3"


# ── Context Compression ───────────────────────────────────────────────────────
# 上下文視窗管理；使用本地粗估：4 chars ≈ 1 token

[context]
# 模型最大上下文 token 數，搭配 compress_threshold_ratio 計算壓縮觸發點
# max_context_tokens = 200000

# 壓縮觸發比例，例如 0.8 = 達到 max_context_tokens 的 80% 時觸發
# compress_threshold_ratio = 0.8

# 直接以字元數設定壓縮觸發點（此欄位會覆蓋 ratio 計算結果）
# 粗估: 600000 chars ≈ 150k tokens
# compress_chars_threshold = 600000

# 壓縮後保留最近幾則訊息的完整內容（預設 8）
# keep_recent_messages = 8

# 單則 tool result 超過此字元數時截斷（預設 40000）
# tool_result_max_chars = 40000

# Agent 迴圈最大輪數（每次 tool call 算一輪，預設 50）
# max_agent_iterations = 50

# 每次 LLM 請求的 max_tokens；未設定時沿用 provider 預設值（Anthropic = 4096）
# 本地模型建議設為 8192 或更高，避免回覆被截斷
# default_max_tokens = 16384


# ── Remote Runtime ────────────────────────────────────────────────────────────
# CCR HTTP 遠端執行環境的連線參數

[remote_runtime.ccr_http]
# 請求 timeout（毫秒，預設 10000）
timeout_ms    = 10000
# 超時或暫時性失敗的重試次數（預設 2）
max_retries   = 2
# 每次重試的等待間隔（毫秒，預設 200）
retry_delay_ms = 200


# ── TUI ───────────────────────────────────────────────────────────────────────
# 終端介面顯示設定

[tui]
# 顏色主題
# "default"       — 16 色標準配色（預設）
# "high_contrast" — 粗體 + 高亮色，適合低對比度螢幕
# "no_color"      — 僅使用文字修飾（無顏色），等同設定 NO_COLOR 環境變數
# 當偵測到 NO_COLOR 環境變數或 TERM=dumb 時，此設定會被自動覆蓋為 no_color
# theme = "default"


# ── Gateway Daemon ────────────────────────────────────────────────────────────
# ccode-gateway 的 HTTP 服務設定

[gateway]
# port    = 7001           # 監聽埠（預設 7001）
# workdir = "/your/work"   # agent 的工作目錄（覆蓋全域 sandbox.cwd）

[gateway.telegram]
bot_token      = "123456:ABC-DEF..."   # 從 @BotFather 取得
mode           = "webhook"             # "webhook"（預設）或 "long_polling"
# webhook_secret = "my-secret"         # 選填；webhook 模式下驗證 X-Telegram-Bot-Api-Secret-Token header

[gateway.discord]
application_public_key = "abcdef..."   # Discord Developer Portal → General Information
# bot_token = "Bot xxxx"               # 選填；需要發送後續 follow-up 訊息時才需要
```

---

## 4. 架構概覽

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
