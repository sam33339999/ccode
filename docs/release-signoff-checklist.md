# Release Sign-Off Checklist

This checklist encodes `acceptance-spec.md` section 9 as a CI-auditable artifact.

## Section 9 Gates

- [ ] Architecture acceptance passed
- [ ] Constants governance acceptance passed
- [ ] Functional parity acceptance passed
- [ ] Non-functional acceptance passed
- [ ] CI/CD gates passed
- [ ] Documentation acceptance passed
- [ ] Stakeholder sign-off from Engineering, SRE/Platform, and Security

## Stakeholder Sign-Off Record

| Role | Owner | Date | Status | Notes |
| --- | --- | --- | --- | --- |
| Engineering |  |  | Pending |  |
| SRE/Platform |  |  | Pending |  |
| Security |  |  | Pending |  |

## CI Gate Mapping

- `crates/bootstrap/tests/workspace_architecture.rs`: architecture and dependency direction.
- `crates/bootstrap/tests/spec_governance_gate.rs`: constants/spec governance.
- `crates/bootstrap/tests/acceptance_parity_gate.rs`: functional parity inventory, dry-run enforcement, runbook presence, and section-9 checklist presence.
