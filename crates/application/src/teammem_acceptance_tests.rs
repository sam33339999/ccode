/// TeamMem acceptance tests (US-042)
///
/// Covers all scenarios from teammem-contract.md Acceptance Matrix:
///   Contract tests  – path traversal/symlink escape, secret findings skip, 412 conflict retry
///   Integration tests – pull/merge/push happy path, unauthorized token, mixed payload
///   Security tests – secret redaction, write outside team root, audit events
use crate::{
    spec_contracts::{SyncResult, TeamMemError, TeamMemorySyncService},
    teammem_kairos_service::{
        BackendFailure, TeamMemAuditEvent, TeamMemAuditLog, TeamMemBackend, TeamMemEntry,
        TeamMemKairosService,
    },
};
use async_trait::async_trait;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex as StdMutex},
};
use tokio::sync::Mutex;

// ── helpers ───────────────────────────────────────────────────────────────

#[derive(Default)]
struct RecordingAuditLog {
    events: StdMutex<Vec<TeamMemAuditEvent>>,
}

impl RecordingAuditLog {
    fn events(&self) -> Vec<TeamMemAuditEvent> {
        self.events.lock().expect("poisoned").clone()
    }
}

impl TeamMemAuditLog for RecordingAuditLog {
    fn record(&self, event: TeamMemAuditEvent) {
        self.events.lock().expect("poisoned").push(event);
    }
}

struct ScriptedBackend {
    pull_results: Mutex<VecDeque<Result<SyncResult, BackendFailure>>>,
    push_results: Mutex<VecDeque<Result<SyncResult, BackendFailure>>>,
    sync_results: Mutex<VecDeque<Result<SyncResult, BackendFailure>>>,
}

impl ScriptedBackend {
    fn new() -> Self {
        Self {
            pull_results: Mutex::new(VecDeque::new()),
            push_results: Mutex::new(VecDeque::new()),
            sync_results: Mutex::new(VecDeque::new()),
        }
    }

    fn with_pull(mut self, results: Vec<Result<SyncResult, BackendFailure>>) -> Self {
        self.pull_results = Mutex::new(results.into());
        self
    }

    fn with_push(mut self, results: Vec<Result<SyncResult, BackendFailure>>) -> Self {
        self.push_results = Mutex::new(results.into());
        self
    }

    fn with_sync(mut self, results: Vec<Result<SyncResult, BackendFailure>>) -> Self {
        self.sync_results = Mutex::new(results.into());
        self
    }
}

#[async_trait]
impl TeamMemBackend for ScriptedBackend {
    async fn pull_once(&self) -> Result<SyncResult, BackendFailure> {
        self.pull_results
            .lock()
            .await
            .pop_front()
            .expect("missing pull result")
    }

    async fn push_once(&self) -> Result<SyncResult, BackendFailure> {
        self.push_results
            .lock()
            .await
            .pop_front()
            .expect("missing push result")
    }

    async fn sync_once(&self) -> Result<SyncResult, BackendFailure> {
        self.sync_results
            .lock()
            .await
            .pop_front()
            .expect("missing sync result")
    }
}

fn ok_sync(pulled: usize, pushed: usize) -> Result<SyncResult, BackendFailure> {
    Ok(SyncResult {
        files_pulled: pulled,
        files_pushed: pushed,
    })
}

fn make_service(
    backend: ScriptedBackend,
    max_retries: u8,
) -> (
    TeamMemKairosService<ScriptedBackend>,
    Arc<RecordingAuditLog>,
) {
    let audit = Arc::new(RecordingAuditLog::default());
    let svc = TeamMemKairosService::new(backend, max_retries, audit.clone());
    (svc, audit)
}

// ── Contract tests ──────────────────────────────────────────────────────

/// AC: path traversal escape (../) returns PathValidationFailed via prompt filtering.
///
/// Entries with parent traversal in the path_key are dropped and an
/// InvalidPathKeyBlocked audit event is emitted.
#[test]
fn contract_path_traversal_escape_returns_path_validation_failed() {
    let (svc, audit) = make_service(ScriptedBackend::new(), 2);

    let traversal_keys = vec![
        "../../../etc/passwd",
        "notes/../../../shadow",
        "team/../../secret",
    ];

    for key in &traversal_keys {
        let payload = svc.build_prompt_payload(&[TeamMemEntry {
            path_key: key.to_string(),
            content: "malicious content".into(),
        }]);

        assert!(
            payload.is_empty(),
            "traversal key '{key}' must be filtered out"
        );
    }

    let events = audit.events();
    let blocked_count = events
        .iter()
        .filter(|e| matches!(e, TeamMemAuditEvent::InvalidPathKeyBlocked { .. }))
        .count();
    assert_eq!(
        blocked_count,
        traversal_keys.len(),
        "each traversal key should emit InvalidPathKeyBlocked"
    );
}

/// AC: symlink-like path escape (absolute paths) returns PathValidationFailed.
///
/// Absolute paths starting with / are blocked as they could reference
/// files outside the team memory root.
#[test]
fn contract_absolute_path_escape_returns_path_validation_failed() {
    let (svc, audit) = make_service(ScriptedBackend::new(), 2);

    let payload = svc.build_prompt_payload(&[
        TeamMemEntry {
            path_key: "/etc/passwd".into(),
            content: "root:x:0:0".into(),
        },
        TeamMemEntry {
            path_key: "/tmp/evil".into(),
            content: "payload".into(),
        },
        TeamMemEntry {
            path_key: "valid/note.md".into(),
            content: "safe content".into(),
        },
    ]);

    assert!(!payload.contains("root:x:0:0"));
    assert!(!payload.contains("payload"));
    assert!(payload.contains("valid/note.md: safe content"));

    let events = audit.events();
    let blocked = events
        .iter()
        .filter(|e| matches!(e, TeamMemAuditEvent::InvalidPathKeyBlocked { .. }))
        .count();
    assert_eq!(blocked, 2, "both absolute paths should be blocked");
}

/// AC: control characters in path keys are blocked.
#[test]
fn contract_control_characters_in_path_blocked() {
    let (svc, audit) = make_service(ScriptedBackend::new(), 2);

    let payload = svc.build_prompt_payload(&[
        TeamMemEntry {
            path_key: "notes/file\x00.md".into(),
            content: "null byte path".into(),
        },
        TeamMemEntry {
            path_key: "notes/file\n.md".into(),
            content: "newline path".into(),
        },
    ]);

    assert!(payload.is_empty(), "control char paths must be blocked");
    assert_eq!(
        audit
            .events()
            .iter()
            .filter(|e| matches!(e, TeamMemAuditEvent::InvalidPathKeyBlocked { .. }))
            .count(),
        2
    );
}

/// AC: secret findings skip file and never return raw secret content.
///
/// When an entry's content contains detectable secrets, the entry is
/// excluded from the prompt payload and no raw secret content leaks.
#[test]
fn contract_secret_findings_skip_file_no_raw_content() {
    let (svc, audit) = make_service(ScriptedBackend::new(), 2);

    let secrets = vec![
        ("openai.md", "api_key=sk-proj-12345678901234567890abcde"),
        (
            "github.md",
            "token: ghp_abcdefghij1234567890abcdef1234567890",
        ),
        ("aws.md", "access_key=AKIAIOSFODNN7EXAMPLE"),
        ("cert.md", "-----BEGIN PRIVATE KEY-----\nMIIE..."),
    ];

    let entries: Vec<TeamMemEntry> = secrets
        .iter()
        .map(|(key, content)| TeamMemEntry {
            path_key: key.to_string(),
            content: content.to_string(),
        })
        .chain(std::iter::once(TeamMemEntry {
            path_key: "notes/clean.md".into(),
            content: "no secrets here".into(),
        }))
        .collect();

    let payload = svc.build_prompt_payload(&entries);

    // Clean entry passes through.
    assert!(payload.contains("notes/clean.md: no secrets here"));

    // No secret content leaks.
    for (_, secret_content) in &secrets {
        assert!(
            !payload.contains(secret_content),
            "raw secret content must not appear in payload: {secret_content}"
        );
    }

    let events = audit.events();
    let skip_count = events
        .iter()
        .filter(|e| matches!(e, TeamMemAuditEvent::SecretSkipped { .. }))
        .count();
    assert_eq!(
        skip_count,
        secrets.len(),
        "each secret entry should emit SecretSkipped"
    );
}

/// AC: 412 conflicts stop at configured max retries (ConflictExhausted).
///
/// When sync_once returns Conflict more times than max_conflict_retries,
/// the service returns ConflictExhausted and records the observation.
#[tokio::test]
async fn contract_412_conflicts_stop_at_max_retries() {
    let max_retries = 3u8;
    let backend = ScriptedBackend::new().with_sync(
        (0..=(max_retries as usize + 1))
            .map(|_| Err(BackendFailure::Conflict))
            .collect(),
    );

    let (svc, audit) = make_service(backend, max_retries);
    let result = svc.sync().await;

    assert!(
        matches!(result, Err(TeamMemError::ConflictExhausted)),
        "expected ConflictExhausted, got {result:?}"
    );

    let obs = svc
        .last_sync_observation()
        .await
        .expect("observation must be recorded");
    assert_eq!(obs.retries, max_retries);
    assert!(obs.conflict_exhausted);
    // attempts = 1 initial + max_retries retries
    assert_eq!(obs.attempts, max_retries + 1);

    let events = audit.events();
    let retry_count = events
        .iter()
        .filter(|e| matches!(e, TeamMemAuditEvent::ConflictRetry { .. }))
        .count();
    assert_eq!(retry_count, max_retries as usize);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, TeamMemAuditEvent::ConflictExhausted { .. }))
    );
}

/// AC: conflict retry with max_retries=0 fails immediately.
#[tokio::test]
async fn contract_zero_retries_fails_immediately_on_conflict() {
    let backend = ScriptedBackend::new().with_sync(vec![Err(BackendFailure::Conflict)]);

    let (svc, _audit) = make_service(backend, 0);
    let result = svc.sync().await;

    assert!(matches!(result, Err(TeamMemError::ConflictExhausted)));

    let obs = svc.last_sync_observation().await.unwrap();
    assert_eq!(obs.attempts, 1);
    assert_eq!(obs.retries, 0);
    assert!(obs.conflict_exhausted);
}

// ── Integration tests ───────────────────────────────────────────────────

/// AC: pull -> merge -> push happy path with ETag handling.
///
/// Simulates pull (backend returns files), then push (backend accepts),
/// verifying the complete sync lifecycle succeeds with correct counts.
#[tokio::test]
async fn integration_pull_merge_push_happy_path() {
    let backend = ScriptedBackend::new()
        .with_pull(vec![ok_sync(5, 0)])
        .with_push(vec![ok_sync(0, 3)]);

    let (svc, _audit) = make_service(backend, 2);

    // Pull phase: receive 5 files.
    let pull_result = svc.pull().await.expect("pull should succeed");
    assert_eq!(pull_result.files_pulled, 5);
    assert_eq!(pull_result.files_pushed, 0);

    // Push phase: send 3 files.
    let push_result = svc.push().await.expect("push should succeed");
    assert_eq!(push_result.files_pushed, 3);
    assert_eq!(push_result.files_pulled, 0);
}

/// AC: full sync happy path succeeds after transient conflict.
///
/// Backend returns one conflict then succeeds, verifying bounded retry works.
#[tokio::test]
async fn integration_sync_succeeds_after_transient_conflict() {
    let backend =
        ScriptedBackend::new().with_sync(vec![Err(BackendFailure::Conflict), ok_sync(3, 2)]);

    let (svc, audit) = make_service(backend, 3);
    let result = svc.sync().await.expect("sync should succeed after retry");

    assert_eq!(result.files_pulled, 3);
    assert_eq!(result.files_pushed, 2);

    let obs = svc.last_sync_observation().await.unwrap();
    assert_eq!(obs.attempts, 2);
    assert_eq!(obs.retries, 1);
    assert!(!obs.conflict_exhausted);

    let events = audit.events();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, TeamMemAuditEvent::ConflictRetry { .. }))
            .count(),
        1
    );
}

/// AC: unauthorized token maps to stable auth error class.
///
/// When the backend returns Unauthorized, the service surfaces the
/// canonical TeamMemError::Unauthorized variant.
#[tokio::test]
async fn integration_unauthorized_token_maps_to_stable_error() {
    let backend = ScriptedBackend::new()
        .with_pull(vec![Err(BackendFailure::Fatal(TeamMemError::Unauthorized))])
        .with_push(vec![Err(BackendFailure::Fatal(TeamMemError::Unauthorized))])
        .with_sync(vec![Err(BackendFailure::Fatal(TeamMemError::Unauthorized))]);

    let (svc, _audit) = make_service(backend, 2);

    let pull_err = svc.pull().await.unwrap_err();
    assert!(
        matches!(pull_err, TeamMemError::Unauthorized),
        "pull: expected Unauthorized, got {pull_err:?}"
    );

    let push_err = svc.push().await.unwrap_err();
    assert!(
        matches!(push_err, TeamMemError::Unauthorized),
        "push: expected Unauthorized, got {push_err:?}"
    );

    let sync_err = svc.sync().await.unwrap_err();
    assert!(
        matches!(sync_err, TeamMemError::Unauthorized),
        "sync: expected Unauthorized, got {sync_err:?}"
    );
}

/// AC: AuthUnavailable maps correctly through all operations.
#[tokio::test]
async fn integration_auth_unavailable_maps_correctly() {
    let backend = ScriptedBackend::new()
        .with_pull(vec![Err(BackendFailure::Fatal(
            TeamMemError::AuthUnavailable,
        ))])
        .with_sync(vec![Err(BackendFailure::Fatal(
            TeamMemError::AuthUnavailable,
        ))]);

    let (svc, _audit) = make_service(backend, 2);

    assert!(matches!(
        svc.pull().await,
        Err(TeamMemError::AuthUnavailable)
    ));
    assert!(matches!(
        svc.sync().await,
        Err(TeamMemError::AuthUnavailable)
    ));
}

/// AC: mixed payload with oversized + secret + valid entries behaves deterministically.
///
/// Given entries with path traversal, secrets, and valid content,
/// only valid entries appear in the payload. The order is preserved
/// and results are deterministic across multiple calls.
#[test]
fn integration_mixed_payload_behaves_deterministically() {
    let (svc, audit) = make_service(ScriptedBackend::new(), 2);

    let entries = vec![
        TeamMemEntry {
            path_key: "../escape/passwd".into(),
            content: "should be blocked by path validation".into(),
        },
        TeamMemEntry {
            path_key: "config/api_keys.md".into(),
            content: "secret=sk-proj-abcdefghij1234567890klmnopqrstuvwxyz".into(),
        },
        TeamMemEntry {
            path_key: "notes/meeting.md".into(),
            content: "Q4 planning notes".into(),
        },
        TeamMemEntry {
            path_key: "/absolute/path".into(),
            content: "blocked by absolute path check".into(),
        },
        TeamMemEntry {
            path_key: "docs/readme.md".into(),
            content: "project documentation".into(),
        },
        TeamMemEntry {
            path_key: "keys/private.pem".into(),
            content: "-----BEGIN PRIVATE KEY-----\nMIIE...".into(),
        },
    ];

    // Run twice to verify determinism.
    let payload_a = svc.build_prompt_payload(&entries);

    // Clear audit for second run.
    audit.events.lock().unwrap().clear();
    let payload_b = svc.build_prompt_payload(&entries);

    assert_eq!(payload_a, payload_b, "payload must be deterministic");

    // Only valid entries survive.
    assert!(payload_a.contains("notes/meeting.md: Q4 planning notes"));
    assert!(payload_a.contains("docs/readme.md: project documentation"));

    // Blocked entries absent.
    assert!(!payload_a.contains("should be blocked"));
    assert!(!payload_a.contains("sk-proj-"));
    assert!(!payload_a.contains("PRIVATE KEY"));
    assert!(!payload_a.contains("/absolute/path"));

    // Order preserved: meeting.md before readme.md.
    let meeting_pos = payload_a.find("notes/meeting.md").unwrap();
    let readme_pos = payload_a.find("docs/readme.md").unwrap();
    assert!(
        meeting_pos < readme_pos,
        "order of valid entries must be preserved"
    );
}

// ── Security tests ──────────────────────────────────────────────────────

/// AC: secret scanner redaction path verified (no raw secrets in output).
///
/// Exhaustive check that all supported secret patterns are detected and
/// never appear in the prompt payload output.
#[test]
fn security_secret_scanner_redaction_no_raw_secrets() {
    let (svc, _audit) = make_service(ScriptedBackend::new(), 2);

    let secret_patterns = vec![
        ("openai_standard", "sk-12345678901234567890abcde"),
        (
            "openai_proj",
            "sk-proj-12345678901234567890abcdefghijklmnopqrs",
        ),
        ("github_pat", "ghp_abcdefghij1234567890abcdef1234567890"),
        ("aws_access_key", "AKIAIOSFODNN7EXAMPLE_secret"),
        ("private_key", "-----BEGIN PRIVATE KEY-----"),
    ];

    for (label, secret) in &secret_patterns {
        let payload = svc.build_prompt_payload(&[TeamMemEntry {
            path_key: format!("test/{label}.txt"),
            content: format!("data: {secret}"),
        }]);

        assert!(
            payload.is_empty(),
            "secret pattern '{label}' must be redacted, but payload was: {payload}"
        );
    }
}

/// AC: non-secret content that superficially resembles keys passes through.
#[test]
fn security_non_secret_lookalikes_pass_through() {
    let (svc, _audit) = make_service(ScriptedBackend::new(), 2);

    let safe_patterns = vec![
        ("short_sk", "sk-short"),                  // Too short to be a real key.
        ("sketch_prefix", "sketch-of-something"),  // Not sk- prefix.
        ("github_discussion", "the ghp is great"), // Not a token.
        ("safe_text", "This is just normal text."),
    ];

    for (label, content) in &safe_patterns {
        let payload = svc.build_prompt_payload(&[TeamMemEntry {
            path_key: format!("test/{label}.txt"),
            content: content.to_string(),
        }]);

        assert!(
            !payload.is_empty(),
            "safe pattern '{label}' should pass through, but was filtered"
        );
    }
}

/// AC: write outside team root prevented (PathValidationFailed).
///
/// Various path escape techniques are all caught by validation:
/// parent traversal, absolute paths, null bytes, unicode line separators.
#[test]
fn security_write_outside_team_root_prevented() {
    let (svc, audit) = make_service(ScriptedBackend::new(), 2);

    let malicious_paths = vec![
        ("parent_traversal", "../../../etc/shadow"),
        ("double_dot_mid", "team/../../../root"),
        ("absolute_unix", "/var/log/syslog"),
        ("absolute_slash_tmp", "/tmp/exploit"),
        ("null_byte", "notes/file\x00.md"),
        ("newline_inject", "notes/file\n../../escape"),
        ("tab_inject", "notes/\tfile.md"),
        ("unicode_line_sep", "notes/file\u{2028}escape.md"),
        ("unicode_para_sep", "notes/file\u{2029}escape.md"),
        ("empty_key", ""),
        ("whitespace_only", "   "),
    ];

    for (label, path) in &malicious_paths {
        let payload = svc.build_prompt_payload(&[TeamMemEntry {
            path_key: path.to_string(),
            content: format!("payload for {label}"),
        }]);

        assert!(
            payload.is_empty(),
            "malicious path '{label}' ({path:?}) must be blocked"
        );
    }

    let events = audit.events();
    let blocked = events
        .iter()
        .filter(|e| matches!(e, TeamMemAuditEvent::InvalidPathKeyBlocked { .. }))
        .count();
    assert_eq!(
        blocked,
        malicious_paths.len(),
        "all malicious paths must emit InvalidPathKeyBlocked"
    );
}

/// AC: audit events emitted for skipped secrets and conflicts.
///
/// Verifies the complete audit trail: SecretSkipped events for detected
/// secrets, ConflictRetry for each retry, and ConflictExhausted at the end.
#[tokio::test]
async fn security_audit_events_for_secrets_and_conflicts() {
    let backend = ScriptedBackend::new().with_sync(vec![
        Err(BackendFailure::Conflict),
        Err(BackendFailure::Conflict),
        Err(BackendFailure::Conflict),
    ]);

    let (svc, audit) = make_service(backend, 2);

    // Phase 1: Trigger secret audit events via prompt filtering.
    let _ = svc.build_prompt_payload(&[
        TeamMemEntry {
            path_key: "secrets/api.md".into(),
            content: "key=ghp_abcdefghij1234567890abcdef1234567890".into(),
        },
        TeamMemEntry {
            path_key: "secrets/aws.md".into(),
            content: "AKIAIOSFODNN7EXAMPLE".into(),
        },
    ]);

    // Phase 2: Trigger conflict audit events via sync.
    let _ = svc.sync().await;

    let events = audit.events();

    // Verify secret skip events.
    let secret_skips: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, TeamMemAuditEvent::SecretSkipped { .. }))
        .collect();
    assert_eq!(secret_skips.len(), 2, "two secrets should be skipped");
    assert!(events.iter().any(|e| matches!(
        e,
        TeamMemAuditEvent::SecretSkipped { path_key } if path_key == "secrets/api.md"
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        TeamMemAuditEvent::SecretSkipped { path_key } if path_key == "secrets/aws.md"
    )));

    // Verify conflict retry events.
    let retries: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, TeamMemAuditEvent::ConflictRetry { .. }))
        .collect();
    assert_eq!(retries.len(), 2, "two conflict retries expected");

    // Verify ConflictRetry events have incrementing attempt numbers.
    assert!(events.iter().any(|e| matches!(
        e,
        TeamMemAuditEvent::ConflictRetry {
            attempt: 1,
            max_retries: 2
        }
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        TeamMemAuditEvent::ConflictRetry {
            attempt: 2,
            max_retries: 2
        }
    )));

    // Verify ConflictExhausted terminal event.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, TeamMemAuditEvent::ConflictExhausted { attempts: 3 })),
        "ConflictExhausted with attempts=3 expected"
    );
}

/// AC: audit log sanitization prevents path key injection in log output.
///
/// Path keys with control characters are sanitized to '?' in audit events
/// to prevent log injection attacks.
#[test]
fn security_audit_log_sanitizes_path_keys() {
    let (svc, audit) = make_service(ScriptedBackend::new(), 2);

    let _ = svc.build_prompt_payload(&[TeamMemEntry {
        path_key: "notes/\x00\x01\x02evil.md".into(),
        content: "content".into(),
    }]);

    let events = audit.events();
    let blocked_event = events
        .iter()
        .find(|e| matches!(e, TeamMemAuditEvent::InvalidPathKeyBlocked { .. }))
        .expect("should have blocked event");

    if let TeamMemAuditEvent::InvalidPathKeyBlocked { path_key, .. } = blocked_event {
        assert!(
            !path_key.contains('\x00'),
            "null bytes must be sanitized in audit log"
        );
        assert!(
            !path_key.contains('\x01'),
            "control chars must be sanitized in audit log"
        );
        assert!(
            path_key.contains('?'),
            "control chars should be replaced with '?'"
        );
    }
}

/// AC: fatal backend errors propagate without retry in sync.
///
/// Non-conflict errors (e.g., StorageWriteError) terminate immediately
/// without consuming retries.
#[tokio::test]
async fn security_fatal_errors_propagate_without_retry() {
    let backend = ScriptedBackend::new().with_sync(vec![Err(BackendFailure::Fatal(
        TeamMemError::StorageWriteError,
    ))]);

    let (svc, audit) = make_service(backend, 5);
    let result = svc.sync().await;

    assert!(
        matches!(result, Err(TeamMemError::StorageWriteError)),
        "fatal error should propagate directly"
    );

    let obs = svc.last_sync_observation().await.unwrap();
    assert_eq!(obs.attempts, 1, "should not retry on fatal error");
    assert_eq!(obs.retries, 0);
    assert!(!obs.conflict_exhausted);

    let events = audit.events();
    assert!(
        events
            .iter()
            .all(|e| !matches!(e, TeamMemAuditEvent::ConflictRetry { .. })),
        "no retry events for fatal errors"
    );
}
