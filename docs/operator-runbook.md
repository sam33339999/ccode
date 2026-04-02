# Operator Runbook

This runbook satisfies `acceptance-spec.md` section 6 observability and operations acceptance.

## Startup

1. Verify configuration exists at `~/.ccode/config.toml`.
2. Run `cargo run -p ccode-cli -- health` and confirm `status: ok`.
3. Run `cargo run -p ccode-cli -- sessions list --limit 1` to confirm storage is reachable.
4. If provider-backed workflows are required, confirm API credentials are configured.

## Shutdown

1. Stop active CLI/TUI sessions cleanly (Ctrl+C for interactive mode).
2. Confirm no in-progress destructive jobs are running.
3. Record shutdown time and reason in operator notes.

## Failure Triage

1. Capture failing command, timestamp, session ID, and provider name from stderr context.
2. Classify failure type:
- policy/auth
- transport/runtime
- state/validation
3. Re-run with minimal reproduction command.
4. Escalate with captured context and command output.

## Rollback

1. Revert to previous release tag/commit.
2. Re-run baseline checks:
- `cargo run -p ccode-cli -- health`
- `cargo test -p ccode-bootstrap --test workspace_architecture`
- `cargo test -p ccode-bootstrap --test acceptance_parity_gate`
3. Confirm critical command paths are restored (`health`, `sessions`, `agent`, `cron`).

## Incident Playbooks

Top 5 on-call scenarios:

1. Provider authentication failures:
- Symptoms: repeated auth errors from `agent` or `cron run`.
- Action: validate key env vars/config and retry with `health` + one command invocation.

2. Tool permission denials:
- Symptoms: tool calls rejected (`PermissionDenied`).
- Action: inspect sandbox policy config and rerun with expected policy.

3. Session storage corruption or unreadable files:
- Symptoms: `sessions list/show/delete` failures.
- Action: back up session store, isolate bad files, restore from known good state.

4. MCP server disconnect loop:
- Symptoms: MCP tools disappear repeatedly.
- Action: verify MCP server process/transport, restart server, confirm tool rediscovery.

5. Cron repository failures:
- Symptoms: `cron list/create/delete/run` errors.
- Action: validate cron storage path permissions and rerun CLI checks.

## Dry-Run Safety Procedure

Use dry-run mode before destructive operations:

1. `ccode sessions delete <id> --dry-run`
2. `ccode sessions clear --dry-run`
3. `ccode cron delete <id> --dry-run`

Dry-run output must be reviewed and approved before running the same command without `--dry-run`.
