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
