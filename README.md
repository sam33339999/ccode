

# 對單一模塊進行測試
```sh
cargo test -p llm-compat
```

⏺ Workspace compiles cleanly. Here's the full analysis:

  ---
  現狀總結

  你的 10 個 crates 其實已經是一個可運行的 agentic CLI，架構清晰，完成度大約 90%。以下是
  docs 規劃 vs 實際 crates 的對應：

  ┌────────────────────────┬───────────────┬───────────────────────────────────────────┐
  │       Docs 規劃        │  實際 Crate   │                   狀態                    │
  ├────────────────────────┼───────────────┼───────────────────────────────────────────┤
  │ api-types +            │ domain        │ ✅ 完成（實體、事件、值物件）             │
  │ core-domain            │               │                                           │
  ├────────────────────────┼───────────────┼───────────────────────────────────────────┤
  │ app-services           │ application   │ ✅ 完成（agent loop、context              │
  │                        │               │ compression）                             │
  ├────────────────────────┼───────────────┼───────────────────────────────────────────┤
  │ llm-compat             │ provider      │ ✅ 完成（4 個 provider adapter + router） │
  ├────────────────────────┼───────────────┼───────────────────────────────────────────┤
  │ config                 │ config        │ ✅ 完成（TOML + env fallback）            │
  ├────────────────────────┼───────────────┼───────────────────────────────────────────┤
  │ tool-runtime           │ tools         │ ✅ 完成（fs/shell/web/cron/spawn_agent）  │
  ├────────────────────────┼───────────────┼───────────────────────────────────────────┤
  │ state-store            │ session +     │ ✅ 完成（file-based + in-memory）         │
  │                        │ cron          │                                           │
  ├────────────────────────┼───────────────┼───────────────────────────────────────────┤
  │ 介面層（traits）       │ ports         │ ✅ 完成（hexagonal 邊界）                 │
  ├────────────────────────┼───────────────┼───────────────────────────────────────────┤
  │ 組裝層                 │ bootstrap     │ ✅ 完成（DI wiring）                      │
  ├────────────────────────┼───────────────┼───────────────────────────────────────────┤
  │ app-cli / ui-tui       │ cli           │ ⚠️  agent 指令完成，其他 partial           │
  ├────────────────────────┼───────────────┼───────────────────────────────────────────┤
  │ mcp-runtime            │ —             │ ❌ 未開始                                 │
  ├────────────────────────┼───────────────┼───────────────────────────────────────────┤
  │ remote-runtime         │ —             │ ❌ 未開始                                 │
  ├────────────────────────┼───────────────┼───────────────────────────────────────────┤
  │ plugin-runtime         │ —             │ ❌ 未開始                                 │
  ├────────────────────────┼───────────────┼───────────────────────────────────────────┤
  │ test-support           │ —             │ ❌ 未開始                                 │
  └────────────────────────┴───────────────┴───────────────────────────────────────────┘

  怎麼拼起來

  你的系統已經可以端到端跑了。拼裝的關鍵入口是 bootstrap crate：

  cli::main()
    → bootstrap::wire_from_config_with_cwd()
      → config::load()                    // 讀 ~/.ccode/config.toml
      → provider::factory::build_default() // 建 LLM provider (含 router)
      → session::FileSessionRepo::new()    // 檔案持久化
      → cron::FileCronRepo::new()          // cron 持久化
      → tools::ToolRegistry + 所有工具註冊  // fs/shell/web/cron/spawn
      → 回傳 AppState { 所有依賴 }

    → application::AgentRunCommand::new(AppState 的各部分)
      → .run(user_message)  // 進入 agent loop

  要跑起來你只需：

  1. 建設定檔
  mkdir -p ~/.ccode
  cp config.example.toml ~/.ccode/config.toml
  # 填入至少一個 provider 的 api_key
  2. 執行
  cargo run -p ccode-cli -- agent -m "hello"

  下一步建議（按優先序）

  1. 補齊 CLI 子指令 — repl、sessions、cron 指令讓日常使用完整
  2. Phase 4: MCP Runtime — 按 chicago-mcp-contract.md 實作，讓 agent 能接外部 MCP server
  3. Phase 5a: Bridge Mode — 按 bridge-mode-contract.md 加 remote-runtime，支援遠端 session
  4. Phase 5b: Multi-Agent — 按 coordinator-mode-contract.md +
  multi-agent-orchestration-contract.md

  核心迴路（user → LLM → tool → LLM → response）已經完整。現在是往外擴展 runtime
  能力的階段。

  要我從哪個部分開始幫你實作？



⏺ 已寫入 tasks/prd.json，共 28 個 user stories，分 16 個 priority level（對應 11 個 phase）。

  依賴鏈概覽：

  Phase 1 (Priority 1-3):  US-001~006  核心互動體驗
  Phase 2 (Priority 4-5):  US-007~009  LLM Compat 對齊
  Phase 3 (Priority 6-7):  US-010~012  MCP Runtime
  Phase 4 (Priority 8):    US-013~014  KAIROS 模式
  Phase 5 (Priority 9):    US-015      Coordinator Mode
  Phase 6 (Priority 10):   US-016      Agent Triggers (local)
  Phase 7 (Priority 10-11):US-018~019  Bridge Mode
  Phase 8 (Priority 11):   US-017      Triggers (remote)
  Phase 9 (Priority 12):   US-020~021  ULTRAPLAN + Platform
  Phase 10 (Priority 13):  US-022      TEAMMEM
  Phase 11 (Priority 14):  US-023~024  Multi-Agent
  Phase 12 (Priority 15-16):US-025~028 整合 + 硬化

  執行方式：
  ralph-tui run --prd ./tasks/prd.json
