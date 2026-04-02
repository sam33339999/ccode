# KAIROS Contract Spec (Rust)

## 1. Objective

Define Rust rewrite contract for KAIROS family (`KAIROS`, `KAIROS_BRIEF`, `KAIROS_CHANNELS`) with stable prompt/routing/telemetry boundaries.

## 2. Current Code Evidence

1. `main.tsx:80`, `81` (assistant module + gate import)
2. `main.tsx:2206` (assistant mode prompt path)
3. `main.tsx:1728` (tool visibility with KAIROS/BRIEF)
4. `utils/processUserInput/processSlashCommand.tsx:102` (slash routing behavior)
5. `commands.ts:63`, `67`, `70` (feature-gated command registration)
6. `services/analytics/metadata.ts:735` (`kairosActive` emission)

## 3. Rust Boundary Mapping

1. `api-types`: mode flags, routing decisions, analytics tag schema.
2. `app-services`: activation policy and routing orchestration.
3. `tool-runtime`: mode-aware tool visibility policy.
4. `ui-tui`: prompt composition and slash entry behavior.
5. `config`: feature gate keys and default mode settings.

## 4. Proposed Contracts

```rust
pub trait AssistantModeService: Send + Sync {
    fn resolve_mode(&self, ctx: AssistantModeContext) -> AssistantModeDecision;
    fn build_prompt(&self, ctx: PromptComposeContext) -> PromptComposeResult;
    fn route_input(&self, ctx: RouteInputContext) -> RouteDecision;
}
```

Policy requirements:

1. Prompt precedence order is deterministic and documented.
2. Tool visibility is a pure function of mode + capability policy.
3. Telemetry tag emission uses schema-only fields (no payload leakage).

## 5. Error Taxonomy

```rust
pub enum KairosError {
    DisabledByPolicy,
    InvalidModeState,
    PromptComposeFailed(String),
    RouteConflict,
}
```

## 6. Constants Classification

1. `api-types`: mode labels and telemetry field keys.
2. `config`: feature gate keys and mode defaults.
3. `core-domain`: route and precedence invariants.
4. `ui-tui`: display copy and helper text.

## 7. Acceptance Matrix

1. Contract tests: mode resolution and route precedence are deterministic.
2. Integration tests: KAIROS/BRIEF/CHANNELS combinations produce expected tool visibility.
3. CLI behavior tests: slash routing and assistant-mode entry behavior match current semantics.

