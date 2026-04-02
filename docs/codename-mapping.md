# Codename-to-Rust Mapping and Design Review

## Purpose

This document maps high-impact feature codenames to the Rust rewrite architecture and reviews whether current planned boundaries are reasonable for enterprise maintainability.

Primary references come from current code paths and gates in:

1. `main.tsx`, `commands.ts`, `tools.ts`, `query.ts`
2. `bridge/*`, `services/mcp/*`, `services/teamMemorySync/*`
3. `commands/ultraplan.tsx`, `utils/processUserInput/*`, `coordinator/*`

## A. Codename Mapping (Confidence Ordered)

| Codename | Evidence (sample) | Rust Crates | Notes |
|---|---|---|---|
| KAIROS / KAIROS_BRIEF / KAIROS_CHANNELS | `main.tsx:80`, `main.tsx:2206`, `utils/processUserInput/processSlashCommand.tsx:102`, `services/analytics/metadata.ts:735` | `ui-tui`, `app-services`, `tool-runtime`, `api-types` | Assistant-mode family affecting boot path, prompt composition, slash routing, telemetry tags. |
| BRIDGE_MODE / CCR Remote Control (`tengu_ccr_bridge`) | `entrypoints/cli.tsx:112`, `bridge/bridgeEnabled.ts:34`, `bridge/createSession.ts:134`, `bridge/createSession.ts:263` | `remote-runtime`, `app-services`, `config`, `api-types` | Remote session lifecycle and OAuth/org-scoped headers, with archive semantics and compat paths. |
| CHICAGO_MCP (computer-use MCP) | `entrypoints/cli.tsx:86`, `services/mcp/client.ts:241`, `services/mcp/config.ts:641`, `query.ts:1033` | `mcp-runtime`, `tool-runtime`, `platform`, `config` | Native-capability MCP path with reserved-name policy and turn-end cleanup hooks. |
| TEAMMEM (team memory sync) | `setup.ts:365`, `services/teamMemorySync/watcher.ts:253`, `services/teamMemorySync/index.ts:177`, `services/teamMemorySync/index.ts:742` | `state-store`, `remote-runtime`, `config`, `platform` | Bi-directional sync touching OAuth/network + filesystem writes + secret scanning. |
| AGENT_TRIGGERS / AGENT_TRIGGERS_REMOTE | `tools.ts:29`, `tools.ts:36`, `skills/bundled/index.ts:56`, `tools/RemoteTriggerTool/RemoteTriggerTool.ts:59` | `tool-runtime`, `remote-runtime`, `state-store`, `config` | Local durable cron and remote-trigger API paths with separate gates. |
| ULTRAPLAN | `commands.ts:104`, `utils/processUserInput/processUserInput.ts:468`, `commands/ultraplan.tsx:330`, `commands/ultraplan.tsx:395` | `ui-tui`, `app-services`, `remote-runtime`, `config` | Keyword/command launch path into long-lived remote session + archive cleanup. |
| COORDINATOR_MODE | `utils/systemPrompt.ts:63`, `coordinator/coordinatorMode.ts:37`, `coordinator/coordinatorMode.ts:66`, `tools.ts:120` | `app-services`, `tool-runtime`, `ui-tui`, `config` | Mode switch and system-prompt/tool-visibility branch driven by env + gate behavior. |
| LLM_COMPAT | `crates/llm-compat/src/request.rs`, `crates/llm-compat/src/response.rs`, `crates/llm-compat/src/convert.rs`, `crates/llm-compat/tests/conversion.rs` | `api-types`, `llm-compat`, `config` | Provider-agnostic LLM adapter. Canonical types in `api-types`, triple conversion (Anthropic ↔ Canonical ↔ OpenAI), HTTP client trait, streaming, tool_use mapping. Foundation crate for dual-API agent. |

## B. Service Boundary Fit Review (Is Current Cut Reasonable?)

### 1) BRIDGE_MODE / CCR

Verdict: **Reasonable with one adjustment**.

Why current cut works:

1. Session lifecycle belongs to `remote-runtime`.
2. Command entry and UX messaging belong to `ui-tui`/`app-cli`.
3. Shared session contracts belong to `api-types`.

Required adjustment:

1. Introduce explicit `RemoteSessionPolicy` in `app-services` for fail-open/fail-close behavior, archive-on-exit guarantees, and resume semantics.  
Without this, policy drifts into runtime adapters.

### 2) AGENT_TRIGGERS(_REMOTE)

Verdict: **Mostly reasonable**.

Why current cut works:

1. Scheduling and tool registration naturally fit `tool-runtime`.
2. Durable local task persistence fits `state-store`.
3. Remote trigger dispatch/auth belongs in `remote-runtime`.

Risk:

1. Cron parsing and storage are easy to over-centralize. Keep scheduler core in `tool-runtime`, disk format adapter in `state-store`.

### 3) CHICAGO_MCP

Verdict: **Reasonable but needs strict isolation**.

Why current cut works:

1. MCP transport/protocol concerns fit `mcp-runtime`.
2. Tool invocation lifecycle integrates with `tool-runtime`.
3. Native wrapper/process controls fit `platform`.

Risk:

1. High-privilege local actions can leak into general MCP code.  
Mitigate by splitting `mcp-runtime` into protocol core + computer-use adapter module with explicit capability checks.

### 4) TEAMMEM

Verdict: **Reasonable with contract hardening**.

Why current cut works:

1. Sync state + merge/persistence belongs to `state-store`.
2. Network sync endpoints and token auth fit `remote-runtime`.
3. Path validation and secure write wrappers fit `platform`.

Risk:

1. Secret scanning and path validation can be bypassed if write APIs are inconsistent.  
Mitigate by exposing only one `ValidatedTeamMemWriter` facade from `state-store`.

### 5) ULTRAPLAN

Verdict: **Reasonable**.

Why current cut works:

1. Keyword/command routing is UI concern.
2. Remote launch/poll/archive lifecycle is remote concern.
3. Plan-mode approval policy belongs to `app-services`.

Risk:

1. Orphan session cleanup logic duplicated across UI and remote modules.  
Mitigate with single `UltraplanSessionManager` service interface.

### 6) KAIROS

Verdict: **Reasonable but broad**.

Why current cut works:

1. Prompt composition and routing span UI + services + tools.
2. Telemetry marker and capability shaping require shared contracts.

Risk:

1. KAIROS becomes a cross-cutting "god mode".  
Mitigate by decomposing into:
1. `assistant-activation` policy (`app-services`)
2. `assistant-prompt` composer (`ui-tui`)
3. `assistant-analytics` emitter (`app-services` with `api-types` tags)

### 7) COORDINATOR_MODE

Verdict: **Reasonable**.

Why current cut works:

1. Mode state and behavior policy fit `app-services`.
2. Tool visibility filtering belongs to `tool-runtime`.
3. Prompt branch in UI/prompt composer remains in `ui-tui`.

Risk:

1. Env-variable mode source can conflict with persisted session mode.  
Mitigate using one `ModeStateStore` contract with deterministic precedence.

### 8) LLM_COMPAT

Verdict: **Reasonable and already partially implemented**.

Why current cut works:

1. Canonical types naturally belong in `api-types` (shared across all crates).
2. Provider-specific wire formats and HTTP transport are isolated in `llm-compat`.
3. `config` controls provider selection and credentials.

Risk:

1. Provider-specific quirks (e.g. OpenAI function_call vs tool_calls, Anthropic thinking blocks) may require adapter-level workarounds that bloat `llm-compat`.
Mitigate by keeping conversion functions per-provider in submodules (`providers/anthropic.rs`, `providers/openai.rs`).

## C. Recommended Interface Contracts (Must Have)

Define these early in `api-types` + `app-services` traits:

1. `RemoteSessionService`  
Create, fetch, archive, resume, poll; explicit result states and timeout taxonomy.

2. `TriggerSchedulerService`  
Create/list/delete task; durable vs session-only semantics; ownership checks.

3. `McpCapabilityService`  
Server registration policy, reserved-name policy, high-privilege capability gates.

4. `TeamMemorySyncService`  
Pull/push/sync with conflict model, secret-scan outcome, path-validation outcome.

5. `ModeCoordinatorService`  
Resolve effective mode from env/session/runtime inputs and expose immutable snapshot.

6. `UltraplanService`  
Launch/poll/stop/archive with orphan prevention semantics and idempotent cleanup.

7. `LlmClient` (trait in `llm-compat`)  
`complete()` and `stream()` operating on Canonical types. Provider adapters implement this trait. Agent loop in `app-services` depends on this trait only.

## D. Constants Classification for These Codenames

Apply constants policy from `README.md` with codename examples:

1. `api-types`  
Event names: ultraplan lifecycle events, bridge session states, trigger result states.

2. `config`  
Gate keys (`tengu_*`), timeout defaults, retry limits, env var names.

3. `core-domain`  
Invariants like allowed state transitions (`running -> archived`, not `archived -> running`).

4. Runtime-local crates  
Transport headers composition details, cache key internals, lock file names.

5. `ui-tui`  
Prompt/usage text and display-only labels.

## E. Design Validity Conclusion

Current rewrite architecture is **structurally valid** for these codenames, but production-grade success depends on three boundary guardrails:

1. Policy interfaces live in `app-services`; runtimes should implement, not define, policy.
2. High-risk paths (remote session, remote trigger, computer-use MCP, team sync writes) must expose one canonical facade each.
3. Constants must remain classified by semantics, never by convenience.

Without those guardrails, the workspace will compile but degrade into cross-crate coupling similar to the current monolith.

## F. Next Deep-Dive Order (Security and Operability First)

1. BRIDGE_MODE / CCR
2. AGENT_TRIGGERS_REMOTE
3. CHICAGO_MCP
4. TEAMMEM
5. ULTRAPLAN
6. KAIROS
7. COORDINATOR_MODE
8. LLM_COMPAT (foundation — implement first despite lowest risk, as all other codenames depend on it)

This order matches risk concentration: remote execution and auth boundaries first, broad behavior/prompt orchestration later.
Exception: LLM_COMPAT has lowest security risk but highest dependency — it is the foundation crate that all upper layers consume.

## G. Implemented Deep-Dive Specs

1. BRIDGE_MODE / CCR: `docs/bridge-mode-contract.md`
2. AGENT_TRIGGERS: `docs/agent-triggers-contract.md`
3. CHICAGO_MCP: `docs/chicago-mcp-contract.md`
4. TEAMMEM: `docs/teammem-contract.md`
5. ULTRAPLAN: `docs/ultraplan-contract.md`
6. Cross-codename integration: `docs/integration-contracts.md`
7. LLM_COMPAT: `docs/llm-compat-contract.md`
