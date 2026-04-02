# Rewrite Acceptance Playbook (Spec-Only Phase)

## Goal

Provide a repeatable way to validate spec quality before implementation begins.

Covered specs:

1. `bridge-mode-contract.md`
2. `agent-triggers-contract.md`
3. `chicago-mcp-contract.md`
4. `teammem-contract.md`
5. `ultraplan-contract.md`
6. IME acceptance requirements in `acceptance-spec.md` section 5.4

## 1. Spec QA Checklist

For each contract doc, verify:

1. At least 6 code evidence anchors with file references.
2. Crate boundary section exists and is consistent with `README.md`.
3. Service trait section exists with explicit method signatures.
4. Error taxonomy section exists with policy/runtime distinction where applicable.
5. Constants classification section exists.
6. Acceptance test matrix includes contract + integration + CLI/security tests.
7. If user input is involved, IME scenarios (preedit/commit/delete/cursor width) are explicitly tested.

## 2. Cross-Spec Consistency Checks

Run document review checks:

1. Error naming style is stable (`*Error` enums, deterministic category names).
2. Gate and entitlement wording is fail-closed by default.
3. State machines define terminal states and forbidden transitions.
4. Remote side-effect features include orphan/session cleanup rules.
5. No policy logic is placed in runtime adapter sections.
6. Text input behavior does not assume byte-length editing for CJK input.

## 3. Readiness Gates Before Coding

All must pass:

1. `acceptance-spec.md` reflects all active contract docs.
2. `spec-iterations.md` status matches actual document completion.
3. Each contract has explicit P0 acceptance cases.
4. Open-risk list is recorded and owner assigned.
5. IME mandatory cases are listed with platform coverage and expected results.

## 4. Output of Spec Review Round

At the end of each review round, produce:

1. Passed checks count.
2. Failed checks with owner and due date.
3. Waivers with expiry date.

This ensures implementation starts from auditable, testable specifications.
