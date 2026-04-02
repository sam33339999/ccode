# Implementation Phases

## Guiding Principle

Build bottom-up from the LLM adapter layer. Each phase produces a working, testable vertical slice.
No phase depends on features from a later phase. Each phase has explicit exit criteria.

---

## Phase 0: LLM Foundation (llm-compat + api-types)

**Goal:** A working dual-provider LLM client that any Rust code can call with a single trait.

### Tasks

1. **Define Canonical types in `api-types`**
   - `ContentBlock`, `Message`, `Role`, `ToolDefinition`, `LlmRequest`, `LlmResponse`, `StopReason`, `TokenUsage`
   - `StreamEvent`, `DeltaBlock` for streaming
   - No provider-specific types here

2. **Refactor `llm-compat` to triple conversion**
   - Current: `anthropic_request_to_openai_request()` (direct)
   - Target: `anthropic_to_canonical()`, `canonical_to_anthropic()`, `openai_to_canonical()`, `canonical_to_openai()`
   - Keep existing tests passing, add Canonical round-trip tests

3. **Add `LlmClient` trait**
   - `complete(LlmRequest) -> Result<LlmResponse, LlmError>`
   - `stream(LlmRequest) -> Result<Stream<StreamEvent>, LlmError>`

4. **Implement `AnthropicAdapter`**
   - HTTP client (reqwest) for Messages API
   - SSE parser for streaming
   - Error mapping (HTTP status -> `LlmError`)

5. **Implement `OpenAiAdapter`**
   - HTTP client for Chat Completions API
   - SSE parser for streaming
   - Error mapping

6. **Add tool_use conversion**
   - `ToolDefinition` <-> provider tool schemas
   - `ContentBlock::ToolUse` / `ToolResult` conversion for both providers
   - `StopReason::ToolUse` mapping

### Exit Criteria

- `cargo test -p api-types -p llm-compat` passes
- Both adapters can `complete()` against real API (manual/integration test)
- Both adapters can `stream()` with correct event sequence
- Tool use round-trip: request with tools -> tool_use response -> tool_result follow-up
- No provider types visible from `api-types` public API

### Crates Touched

`api-types` (new), `llm-compat` (refactor + extend), `config` (provider/key config)

---

## Phase 1: Agent Loop (app-services)

**Goal:** A minimal agent that can converse and execute tools in a loop.

### Tasks

1. **Define agent loop in `app-services`**
   - Accept `LlmClient` + tool registry
   - Loop: send messages -> check stop_reason -> if tool_use, execute tool -> append result -> repeat
   - Terminate on `EndTurn` or `MaxTokens`

2. **Define `Tool` trait in `tool-runtime`**
   - `name() -> &str`
   - `description() -> &str`
   - `input_schema() -> serde_json::Value`
   - `execute(input: serde_json::Value) -> Result<String, ToolError>`

3. **Implement core tools**
   - `ReadFileTool` — read file contents
   - `WriteFileTool` — write/create files
   - `EditFileTool` — search-and-replace edits
   - `BashTool` — execute shell commands
   - `GlobTool` — file pattern search
   - `GrepTool` — content search

4. **System prompt composition**
   - Inject tool definitions into `LlmRequest.tools`
   - Configurable system prompt

5. **Conversation state**
   - Message history management
   - Token budget tracking via `TokenUsage`

### Exit Criteria

- Agent can hold multi-turn conversation with tool use
- Agent can read a file, edit it, and verify the edit — end-to-end
- Works with both Anthropic and OpenAI backends (swap via config)
- Streaming output displays incrementally

### Crates Touched

`app-services` (new), `tool-runtime` (new), `config` (extend)

---

## Phase 2: CLI Interface (app-cli + ui-tui)

**Goal:** Interactive terminal experience.

### Tasks

1. **CLI entrypoint with `clap`**
   - `ccode` command with subcommands and flags
   - `--provider`, `--model`, `--system-prompt` flags
   - Stdin pipe support for non-interactive use

2. **Terminal rendering**
   - Markdown rendering for assistant responses
   - Streaming token display
   - Tool execution status indicators
   - Input handling (multi-line, history)

3. **IME support**
   - Grapheme-cluster-aware editing
   - CJK display-width cursor positioning
   - Preedit/commit state separation (per acceptance-spec.md 5.4)

4. **Permission system**
   - Tool execution approval prompts
   - Auto-allow / ask / deny modes

### Exit Criteria

- `ccode` launches interactive session
- Streaming responses render in real-time
- Chinese/Japanese input works correctly
- Tool use shows execution and result inline
- Non-interactive pipe mode works (`echo "question" | ccode`)

### Crates Touched

`bins/app-cli` (new), `ui-tui` (new)

---

## Phase 3: State & Memory (state-store + config)

**Goal:** Persistent conversation and project context.

### Tasks

1. **Session persistence**
   - Save/resume conversation history
   - Session listing and selection

2. **Project context**
   - CLAUDE.md / project-level instructions loading
   - Git context awareness (branch, status, recent commits)

3. **Configuration layering**
   - Global config (`~/.config/ccode/`)
   - Project config (`.ccode/`)
   - Environment variables
   - CLI flag overrides

### Exit Criteria

- Conversations persist and can be resumed
- Project instructions automatically loaded
- Config precedence: CLI > env > project > global

### Crates Touched

`state-store` (new), `config` (extend), `platform` (new)

---

## Phase 4: MCP Integration (mcp-runtime)

**Goal:** Model Context Protocol server support.

### Tasks

1. **MCP client implementation**
   - stdio and SSE transport
   - Tool/resource/prompt discovery
   - Server lifecycle management

2. **MCP tool bridge**
   - MCP tools register as native tools in `tool-runtime`
   - Permission/approval integration

3. **Server configuration**
   - Per-project MCP server config
   - Built-in vs user servers

### Exit Criteria

- External MCP servers can be launched and their tools used
- MCP tools appear alongside native tools in agent loop
- Server crash/timeout handled gracefully

### Crates Touched

`mcp-runtime` (new), `tool-runtime` (extend), `config` (extend)

---

## Phase 5: Advanced Features

**Goal:** Multi-agent, remote, and plugin capabilities.

### Sub-phases (can be parallelized)

#### 5a: Multi-Agent (coordinator mode)
- `AgentTool` for spawning sub-agents
- Coordinator mode with fan-out/synthesis
- Task notification system
- Per `multi-agent-orchestration-contract.md`

#### 5b: Remote Sessions (bridge mode)
- Remote session lifecycle
- OAuth/org-scoped auth
- Archive semantics
- Per `bridge-mode-contract.md`

#### 5c: Plugin System (plugin-runtime)
- Plugin loading and capability model
- Skill/slash-command registry
- Per `agent-triggers-contract.md`

### Exit Criteria

- Coordinator can spawn parallel workers and synthesize results
- Remote sessions create/resume/archive correctly
- Plugin system loads and executes external skills

### Crates Touched

All remaining crates from workspace layout

---

## Phase 6: Hardening & Release

**Goal:** Production readiness.

### Tasks

1. **Security review**
   - Secret scanning
   - Permission boundary tests
   - Dependency audit (`cargo audit`, `cargo deny`)

2. **Performance**
   - Cold startup benchmark
   - Streaming latency benchmark
   - Memory profiling

3. **CI/CD**
   - All quality gates from `acceptance-spec.md` section 7
   - Release packaging (homebrew, cargo install, binaries)

4. **Documentation**
   - Crate-level READMEs
   - Operator runbook
   - ADR directory

### Exit Criteria

- All acceptance criteria from `acceptance-spec.md` pass
- All codename contracts have passing test suites
- Release candidate tagged

### Crates Touched

`xtask` (new), `test-support` (new), all crates (hardening)

---

## Dependency Graph (Phase Order)

```
Phase 0 (llm-compat)
    ↓
Phase 1 (agent loop)
    ↓
Phase 2 (CLI)     Phase 3 (state)     ← can parallelize
    ↓                  ↓
Phase 4 (MCP)
    ↓
Phase 5a/5b/5c    ← can parallelize
    ↓
Phase 6 (hardening)
```

## Priority and Scope Notes

- Phase 0-2 is the **minimum viable agent** — a working CLI that can call LLMs and execute tools.
- Phase 3 adds **daily-driver quality** — persistence and project awareness.
- Phase 4-5 adds **parity with reference architecture** — MCP, multi-agent, remote.
- Phase 6 is **release gating** — must pass before any public distribution.

Each phase is independently shippable for internal testing.
