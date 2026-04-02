# Integration Contracts (Cross-Codename)

## Objective

Define cross-feature contracts so independently migrated crates compose safely.

## 1. BRIDGE_MODE x ULTRAPLAN

Contract:

1. Ultraplan launch uses remote session APIs through `app-services` only.
2. Orphan archive is idempotent and non-fatal if session already archived.
3. Concurrent remote-control and ultraplan operations must preserve session ownership boundaries.

Acceptance checks:

1. Launch failure after session creation always issues archive attempt.
2. Archived session cannot re-enter running state.
3. Concurrent operations do not corrupt active session pointer state.

## 2. CHICAGO_MCP x Tool Runtime

Contract:

1. Privileged adapter lifecycle is controlled by tool runtime hooks.
2. Turn-end and interrupt cleanup are invoked once per turn scope.
3. Reserved-name and capability checks happen before runtime connection.

Acceptance checks:

1. Cleanup hooks run on normal completion and interruption.
2. Privileged capability cannot be activated via generic MCP registration.
3. Policy failure prevents side effects.

## 3. TEAMMEM x KAIROS Memory Flow

Contract:

1. Team memory sync operations must not bypass secret/path safety guards.
2. Memory prompt injection consumes validated state only.
3. Sync retries and watcher behavior must not block user interaction loops.

Acceptance checks:

1. Secret-detected content is never emitted in prompt payload.
2. Invalid path keys are blocked and logged safely.
3. Sync conflict retries remain bounded and observable.

## 4. AGENT_TRIGGERS x Remote Runtime

Contract:

1. Local scheduling remains functional when remote dispatch is disabled.
2. Remote dispatch errors do not delete local scheduled state.
3. Ownership and durability constraints are enforced before dispatch.

Acceptance checks:

1. Remote gate-off path keeps local tasks intact.
2. Durable/session-only semantics survive restart semantics.
3. Ownership violations return deterministic policy errors.

## 5. Shared Error Envelope

All cross-feature paths expose:

1. Stable error category (`policy`, `auth`, `transport`, `state`, `validation`)
2. Correlation ID for remote/network operations
3. User-visible message mapped from stable error category

Acceptance checks:

1. Each contract doc maps local errors into this shared envelope.
2. Logs and telemetry include correlation IDs for remote calls.

## 6. COORDINATOR_MODE x Multi-Agent Orchestration

Contract:

1. Coordinator mode owns orchestration policy decisions (fan-out, blocking vs sidecar).
2. Worker task results must be ingested as structured task notifications.
3. Coordinator must synthesize a merged summary, not forward raw notifications only.

Acceptance checks:

1. Parallel fan-out occurs only when write-scope conflicts are absent.
2. Mixed worker outcomes (completed/failed/cancelled) produce deterministic aggregate status.
3. Final summary contains findings and actionable next steps with task traceability.
