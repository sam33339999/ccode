# TEAMMEM Contract Spec (Rust)

## 1. Objective

Define Rust contracts for team memory sync with mandatory security checks (path validation + secret scanning) and deterministic conflict handling.

## 2. Evidence from Current Code

1. `setup.ts:365` startup gate for TEAMMEM watcher.
2. `services/teamMemorySync/watcher.ts:253` watcher start conditions.
3. `services/teamMemorySync/index.ts:177` Bearer auth usage.
4. `services/teamMemorySync/index.ts:496` conflict handling (412).
5. `services/teamMemorySync/index.ts:741` mkdir + write path.
6. `services/teamMemorySync/secretScanner.ts` client-side secret scanner.
7. `services/teamMemorySync/teamMemSecretGuard.ts` write-time secret guard.
8. `memdir/teamMemPaths.ts` path validation and traversal protection.

## 3. Rust Boundary Mapping

1. `crates/api-types`: sync requests/results, conflict and secret-scan outcomes.
2. `crates/app-services`: sync orchestration and retry policy.
3. `crates/state-store`: local read/merge/write abstraction.
4. `crates/remote-runtime`: sync endpoint client and auth integration.
5. `crates/platform`: secure path validation and filesystem adapters.
6. `crates/config`: retry cap, max-size/max-entry defaults.

## 4. Core Contracts (Rust Sketch)

```rust
#[derive(Debug, Clone)]
pub struct TeamMemSyncOptions {
    pub max_retries: u8,
    pub max_file_size_bytes: usize,
    pub max_entries: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct SecretScanFinding { pub rule_id: String, pub label: String }

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub files_pulled: usize,
    pub files_pushed: usize,
    pub skipped_secrets: Vec<String>,
}

#[async_trait::async_trait]
pub trait TeamMemorySyncService {
    async fn pull(&self) -> Result<SyncResult, TeamMemError>;
    async fn push(&self) -> Result<SyncResult, TeamMemError>;
    async fn sync(&self) -> Result<SyncResult, TeamMemError>;
}
```

## 5. Error Taxonomy

1. `AuthUnavailable`
2. `Unauthorized`
3. `ConflictExhausted`
4. `PathValidationFailed`
5. `SecretDetected`
6. `FileTooLarge`
7. `EntryLimitExceeded`
8. `StorageWriteError`
9. `UpstreamTimeout`

## 6. Policy Rules

1. All remote writes pass secret scan first.
2. All local writes pass canonical path validation first.
3. 412 conflict uses bounded retry with deterministic terminal error.
4. Files over max-size are skipped with explicit classification.
5. Max-entry overflow has deterministic truncation/skip policy.

## 7. Constants Classification

1. `api-types`: sync outcome enums and error class identifiers.
2. `config`: retry caps, limits, endpoint keys.
3. `core-domain`: invariant ordering (validate -> scan -> write/push).
4. `state-store`/`platform`: local lock/key/file internals.
5. `ui-tui`: user-facing summaries only.

## 8. Acceptance Matrix

### Contract tests

1. Path traversal/symlink escape returns `PathValidationFailed`.
2. Secret findings skip file and never return raw secret content.
3. 412 conflicts stop at configured max retries.

### Integration tests

1. Pull -> merge -> push happy path with ETag handling.
2. Unauthorized token maps to stable auth error class.
3. Mixed payload with oversized + secret + valid entries behaves deterministically.

### Security tests

1. Secret scanner redaction path verified.
2. Write outside team root prevented.
3. Audit events emitted for skipped secrets and conflicts.

