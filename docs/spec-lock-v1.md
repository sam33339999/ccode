# Rust Rewrite Spec Lock v1

## 1. Lock Objective

Freeze a reviewable and testable spec baseline before implementation starts.

## 2. Locked Artifacts

1. `README.md`
2. `acceptance-spec.md`
3. `codename-mapping.md`
4. `bridge-mode-contract.md`
5. `agent-triggers-contract.md`
6. `chicago-mcp-contract.md`
7. `teammem-contract.md`
8. `ultraplan-contract.md`
9. `kairos-contract.md`
10. `coordinator-mode-contract.md`
11. `integration-contracts.md`
12. `spec-iterations.md`
13. `multi-agent-orchestration-contract.md`
14. `llm-compat-contract.md`
15. `implementation-phases.md`

## 3. Release-Gate Criteria

All must be true:

1. Every `P0` codename has contract + acceptance criteria.
2. Cross-feature integration contracts are defined.
3. Constants classification exists for each codename contract.
4. No unresolved `TBD` in policy-critical sections.
5. `llm-compat-contract.md` defines Canonical types and provider adapter contracts.
6. `implementation-phases.md` defines phased delivery plan with exit criteria.

## 4. Waiver Process

Any exception must include:

1. Scope and impacted codename.
2. Owner and review approver.
3. Risk statement and mitigation.
4. Expiry date.

No waiver without expiry is allowed.

## 5. Implementation Entry Criteria

Before writing Rust code:

1. Engineering sign-off on architecture and boundaries.
2. Security sign-off on high-risk paths (Bridge, Remote Trigger, TeamMem, Chicago MCP).
3. SRE/Platform sign-off on observability and runbook requirements.

## 6. Acceptance Commands (when implementation exists)

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
```

## 7. Versioning Rule

1. Breaking spec changes require `spec-lock-v2.md`.
2. Non-breaking clarifications can be appended with dated notes in this file.
