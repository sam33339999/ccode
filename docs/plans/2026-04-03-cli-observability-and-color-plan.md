# CLI Observability And Color Control Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove `tok/s` progress output from agent/chat flows, add configurable tool/agent color mapping, and make subagent execution progress visible instead of black-box.

**Architecture:** Keep existing event pipeline and extend it with lightweight metadata: display style policy from config, worker stream events from subagents, and richer TUI rendering. Preserve current default behavior when config keys are absent so rollout is backward compatible.

**Tech Stack:** Rust, clap CLI, ratatui, serde, tokio broadcast channels.

---

### Task 1: Remove `tok/s` output in agent + repl modes

**Files:**
- Modify: `crates/cli/src/cmd/agent.rs`
- Modify: `crates/cli/src/cmd/repl.rs`
- Modify: `crates/cli/src/cmd/output.rs`

**Step 1: Write failing/updated tests**
- Update `crates/cli/src/cmd/output.rs` tests:
  - Replace `stream_progress_renders_token_stats` with assertion that stream progress renderer is unused/removed.
  - If `StreamProgress` is deleted, delete related unit tests and keep compile-level proof.

**Step 2: Remove stream progress runtime hooks**
- In `agent.rs`:
  - Remove `StreamProgress` creation.
  - Remove periodic `eprint!("\r...tok/s")` in on-delta callback.
  - Remove final summary progress line output.
- In `repl.rs`:
  - Remove per-turn progress tracker setup.
  - Remove periodic/final `tok/s` prints.

**Step 3: Clean output module**
- In `output.rs`:
  - Remove `StreamProgress` type and related imports (`Duration`, `Instant`, token estimator dependency) if no longer used.
  - Keep tool and error formatting untouched.

**Step 4: Verify**
- Run: `cargo test -p ccode-cli cmd::output`
- Run: `cargo test -p ccode-cli cmd::agent cmd::repl`

**Step 5: Commit**
- `git add crates/cli/src/cmd/agent.rs crates/cli/src/cmd/repl.rs crates/cli/src/cmd/output.rs`
- `git commit -m "feat(cli): remove tok/s progress lines from agent and repl"`

### Task 2: Add configurable colors for tool/agent events with risk-aware defaults

**Files:**
- Modify: `crates/config/src/schema.rs`
- Modify: `crates/bootstrap/src/lib.rs`
- Modify: `config.example.toml`
- Modify: `crates/cli/src/cmd/tui.rs`
- Modify: `crates/cli/src/cmd/output.rs` (ANSI color for non-TUI if desired)
- Test: `crates/cli/src/cmd/tui.rs` tests

**Step 1: Extend config schema**
- Add optional config block under `[tui]` such as:
  - `tool_color_low`
  - `tool_color_medium`
  - `tool_color_high`
  - `worker_color_running`
  - `worker_color_completed`
  - `worker_color_failed`
- Keep string enum values constrained to known color names (e.g. `red`, `yellow`, `green`, `cyan`, `magenta`, `white`, `gray`).
- Add sensible defaults when unset:
  - risk low = cyan
  - risk medium = yellow
  - risk high = red

**Step 2: Bootstrap accessors**
- Add helper(s) in `crates/bootstrap/src/lib.rs` to expose parsed TUI display config (parallel to existing `tui_theme()`).
- Ensure invalid color strings gracefully fall back to defaults.

**Step 3: TUI rendering**
- In `tui.rs`, expand `Theme` to include styles for:
  - tool start by risk level
  - tool done success/fail
  - worker line per status
- In `push_tool_start`, include risk label and store/render with risk style.
- In tool approval modal, style `Risk: high/medium/low` line by risk class.

**Step 4: Optional non-TUI color parity**
- For `agent`/`repl` stderr tool logs, optionally apply ANSI color wrappers by risk.
- Respect `NO_COLOR` and `TERM=dumb`.

**Step 5: Verify**
- Run: `cargo test -p ccode-cli cmd::tui`
- Run: `cargo test -p ccode-config`
- Manual check: run `ccode tui` and trigger `fs_read` / `web_fetch` / `fs_write` to confirm color differentiation.

**Step 6: Commit**
- `git add crates/config/src/schema.rs crates/bootstrap/src/lib.rs config.example.toml crates/cli/src/cmd/tui.rs crates/cli/src/cmd/output.rs`
- `git commit -m "feat(tui): configurable risk and worker colors for tool/agent events"`

### Task 3: Make subagent execution visible (de-blackbox)

**Files:**
- Modify: `crates/tools/src/worker_monitor.rs`
- Modify: `crates/tools/src/spawn_agent.rs`
- Modify: `crates/tools/src/agent.rs`
- Modify: `crates/cli/src/cmd/tui.rs`
- Test: `crates/cli/src/cmd/tui.rs` tests

**Step 1: Expand worker monitor event contract**
- Add optional fields to `WorkerMonitorEvent`:
  - `source` (`coordinator` | `spawn_agent`)
  - `event_type` (`status` | `delta` | `tool_start` | `tool_done`)
  - `detail` (short free text for tool name/summary)
- Keep existing fields for backward compatibility.

**Step 2: Emit streaming progress from subagents**
- In `spawn_agent.rs` and `agent.rs` worker branches:
  - During `on_delta`, publish throttled `delta` events (e.g. every 200-500ms or every N chars).
  - On subagent tool execution, publish `tool_start` / `tool_done` event snapshots.
  - Continue publishing `Running/Completed/Failed` status events as today.

**Step 3: TUI presentation model**
- Extend `WorkerTaskEntry` to keep a short rolling `last_activity` and recent log lines ring buffer.
- In Worker Details pane show:
  - current status
  - session id (if known)
  - last delta snippet
  - last tool event
- Keep existing summary line, but stop truncating everything to first line only for subagent response preview.

**Step 4: Guardrails**
- Cap per-worker buffered detail (e.g. 20 lines, each max 160 chars) to avoid memory/visual flooding.
- Sanitize control chars and keep line wrapping safe.

**Step 5: Verify**
- Add tests for:
  - worker event merge logic when delta/status interleave
  - worker details pane rendering for delta + tool updates
  - no panic when worker sends high-frequency updates
- Run: `cargo test -p ccode-cli cmd::tui`
- Run: `cargo test -p ccode-tools spawn_agent agent`

**Step 6: Commit**
- `git add crates/tools/src/worker_monitor.rs crates/tools/src/spawn_agent.rs crates/tools/src/agent.rs crates/cli/src/cmd/tui.rs`
- `git commit -m "feat(worker): stream subagent progress and activity into tui"`

### Task 4: Documentation and migration note

**Files:**
- Modify: `config.example.toml`
- Modify: `README.md`
- Optional: `docs/README.md`

**Step 1: Document behavior changes**
- Explicitly state:
  - `tok/s` progress line removed from `agent` and `repl`.
  - Tool/worker color mapping is configurable.
  - Worker panel now shows live subagent activity, not only terminal summary.

**Step 2: Add config snippet**
- Provide a copy-paste `[tui]` example with risk color keys and defaults.

**Step 3: Verify docs consistency**
- Run: `rg -n "tok/s|tool_color|worker panel|subagent" README.md config.example.toml docs`

**Step 4: Commit**
- `git add README.md config.example.toml docs/README.md`
- `git commit -m "docs(cli): document new observability and color configuration"`
