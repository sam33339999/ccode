# Functional Parity Matrix

This matrix is the acceptance artifact for `acceptance-spec.md` section 4.

## Coverage Thresholds

- P0 command parity threshold: 100%
- P1 command parity threshold: 95%
- P0 tool parity threshold: 100%
- P1 tool parity threshold: 95%

## Command Parity

| Priority | Command group | Status | Evidence |
| --- | --- | --- | --- |
| P0 | `ccode health` | Implemented | `crates/cli/src/cmd/health.rs` |
| P0 | `ccode sessions` | Implemented | `crates/cli/src/cmd/sessions.rs` |
| P0 | `ccode agent` | Implemented | `crates/cli/src/cmd/agent.rs` |
| P0 | `ccode repl` | Implemented | `crates/cli/src/cmd/repl.rs` |
| P0 | `ccode tui` | Implemented | `crates/cli/src/cmd/tui.rs` |
| P0 | `ccode cron` | Implemented | `crates/cli/src/cmd/cron.rs` |
| P1 | `ccode bridge` rendering behavior | Implemented (module-level) | `crates/cli/src/cmd/bridge.rs` |
| P1 | `ccode chicago` rendering behavior | Implemented (module-level) | `crates/cli/src/cmd/chicago.rs` |

## Tool Parity

| Priority | Tool | Status | Evidence |
| --- | --- | --- | --- |
| P0 | `fs_read` | Registered | `crates/bootstrap/src/lib.rs` |
| P0 | `fs_write` | Registered | `crates/bootstrap/src/lib.rs` |
| P0 | `fs_edit` | Registered | `crates/bootstrap/src/lib.rs` |
| P0 | `fs_list` | Registered | `crates/bootstrap/src/lib.rs` |
| P0 | `fs_grep` | Registered | `crates/bootstrap/src/lib.rs` |
| P0 | `fs_glob` | Registered | `crates/bootstrap/src/lib.rs` |
| P0 | `shell` | Registered | `crates/bootstrap/src/lib.rs` |
| P0 | `web_fetch` | Registered | `crates/bootstrap/src/lib.rs` |
| P0 | `browser` | Registered | `crates/bootstrap/src/lib.rs` |
| P1 | `cron_create` | Registered when provider + cron repo are available | `crates/bootstrap/src/lib.rs` |
| P1 | `spawn_agent` | Registered when provider is available | `crates/bootstrap/src/lib.rs` |
| P1 | `agent` | Registered when provider is available | `crates/bootstrap/src/lib.rs` |
| P1 | `task_stop` | Registered when provider is available | `crates/bootstrap/src/lib.rs` |

## Waiver Rule

Any accepted P1 gap requires a dated waiver with owner and expiry, per `acceptance-spec.md` section 4.2 and section 9.
