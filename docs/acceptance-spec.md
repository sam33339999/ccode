# Rust Rewrite Acceptance Specification

## 1. Scope

This specification defines measurable acceptance criteria for full migration from TypeScript/Bun to Rust workspace architecture.

In scope:

1. Workspace architecture (`crates/`, `bins/`, `xtask/`).
2. Service boundary migration and ownership.
3. Command/tool/runtime parity.
4. Security, observability, and operability readiness.

Out of scope:

1. New product features not present in legacy behavior.
2. Cosmetic UX redesign unrelated to parity or maintainability.

## 2. Architecture Acceptance

### 2.1 Workspace Structure

Must exist:

1. Root `Cargo.toml` with workspace members for all runtime and service crates.
2. `crates/` with at least the core crates declared in the rewrite plan.
3. `bins/app-cli` binary crate as primary CLI entrypoint.
4. `xtask` crate for release/build/check automation.

Acceptance command:

```bash
cargo metadata --format-version 1 > /tmp/rust-workspace-metadata.json
```

Pass condition:

1. Command exits `0`.
2. Metadata includes expected members and no duplicate package names.

### 2.2 Dependency Direction Rules

Must hold:

1. `core-domain` does not depend on runtime or UI crates.
2. `api-types` does not depend on runtime crates.
3. `bins/*` contain composition only, with no domain logic implementation.

Acceptance mechanism:

1. Static dependency check script in `xtask`.
2. PR check that fails on forbidden edges.

Pass condition:

1. All forbidden edges report zero findings.

## 3. Constants Governance Acceptance

Constants must be classified into exactly one category:

1. Contract constants in `api-types`.
2. Configurable defaults in `config`.
3. Domain invariants in `core-domain`.
4. Runtime-local constants in owning runtime crate.
5. UI-only constants in `ui-tui`.

Acceptance mechanism:

1. `docs/rust-rewrite/constants-policy.md` exists and is referenced by crate READMEs.
2. Lint/check script blocks new cross-domain `constants` dumping patterns.

Pass condition:

1. No unresolved constants classification exceptions in release branch.

## 4. Functional Parity Acceptance

### 4.1 Command Parity

For each supported CLI command group:

1. Success path parity test.
2. Validation error parity test.
3. Permission or auth failure parity test (where applicable).

Pass condition:

1. `P0` command set reaches 100% parity tests passing.
2. `P1` command set reaches at least 95% parity tests passing.

### 4.2 Tool Runtime Parity

For each migrated tool category:

1. Deterministic output parity test.
2. Timeout/cancellation behavior parity test.
3. Structured error mapping parity test.

Pass condition:

1. No open `P0` parity gaps.
2. Documented waiver for any accepted `P1` differences.

## 5. Non-Functional Acceptance

### 5.1 Reliability

Pass condition:

1. CLI smoke suite passes for 30 consecutive CI runs on main branch.
2. No crash-loop defects in soak test window.

### 5.2 Performance

Benchmarks must be captured for representative flows:

1. Cold startup time.
2. Median command latency.
3. Memory footprint under sustained interaction.

Pass condition:

1. No metric regresses more than agreed budget (default: 10%) without approved waiver.

### 5.3 Security

Pass condition:

1. Dependency checks pass (`cargo audit` and policy checks).
2. Secret scanning passes.
3. Permission boundary tests pass for runtime-sensitive operations.

### 5.4 Input Method Editor (IME) Correctness

Scope:

1. Interactive CLI/TUI text input behavior under Chinese IME composition and candidate selection.

Required behavior:

1. Composition text (preedit) and committed text are managed as separate states.
2. Backspace/delete operates on grapheme clusters, not bytes.
3. Cursor movement and rendering use display width rules, preventing full-width/half-width drift.

Pass condition:

1. No visual corruption after candidate selection followed by delete/backspace.
2. Cursor position remains aligned with rendered text during and after composition.
3. Same test cases pass on macOS input methods used by the team.

## 6. Observability and Operations Acceptance

Must exist:

1. Structured logging with request/session correlation IDs.
2. Error taxonomy documented and mapped to user-facing exit behavior.
3. Operational runbook for startup, shutdown, failure triage, and rollback.

Pass condition:

1. On-call dry run can complete top 5 incident playbooks without undocumented steps.

## 7. CI/CD Acceptance Gates

Required pipeline checks:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
```

Recommended additional checks:

```bash
cargo audit
cargo deny check
```

Pass condition:

1. All required checks are green on release candidate tag.
2. IME regression suite is green on release candidate tag.

## 8. Documentation Acceptance

Required artifacts:

1. `docs/rust-rewrite/README.md` (architecture + migration strategy).
2. `docs/rust-rewrite/constants-policy.md` (classification policy).
3. `docs/rust-rewrite/adr/*` for boundary decisions.
4. Operator runbook and release checklist.
5. `docs/rust-rewrite/multi-agent-orchestration-contract.md` for coordinator fan-out/synthesis behavior.

Pass condition:

1. Every runtime crate has a README with purpose, dependencies, and owner.

## 9. Final Sign-Off Checklist

Release sign-off requires all below:

1. Architecture acceptance passed.
2. Constants governance acceptance passed.
3. Functional parity acceptance passed.
4. Non-functional acceptance passed.
5. CI/CD gates passed.
6. Documentation acceptance passed.
7. Stakeholder sign-off from Engineering, SRE/Platform, and Security.

Any exception must be recorded as a dated waiver with owner and expiry date.
