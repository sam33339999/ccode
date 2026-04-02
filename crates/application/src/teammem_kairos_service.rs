use crate::spec_contracts::{SyncResult, TeamMemError, TeamMemorySyncService};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamMemEntry {
    pub path_key: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamMemAuditEvent {
    SecretSkipped {
        path_key: String,
    },
    InvalidPathKeyBlocked {
        path_key: String,
        reason: &'static str,
    },
    ConflictRetry {
        attempt: u8,
        max_retries: u8,
    },
    ConflictExhausted {
        attempts: u8,
    },
}

pub trait TeamMemAuditLog: Send + Sync {
    fn record(&self, event: TeamMemAuditEvent);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncObservation {
    pub attempts: u8,
    pub retries: u8,
    pub conflict_exhausted: bool,
}

#[derive(Debug)]
pub enum BackendFailure {
    Conflict,
    Fatal(TeamMemError),
}

#[async_trait]
pub trait TeamMemBackend: Send + Sync {
    async fn pull_once(&self) -> Result<SyncResult, BackendFailure>;
    async fn push_once(&self) -> Result<SyncResult, BackendFailure>;
    async fn sync_once(&self) -> Result<SyncResult, BackendFailure>;
}

pub struct TeamMemKairosService<B> {
    backend: B,
    max_conflict_retries: u8,
    audit_log: Arc<dyn TeamMemAuditLog>,
    last_sync_observation: Mutex<Option<SyncObservation>>,
}

impl<B> TeamMemKairosService<B> {
    pub fn new(backend: B, max_conflict_retries: u8, audit_log: Arc<dyn TeamMemAuditLog>) -> Self {
        Self {
            backend,
            max_conflict_retries,
            audit_log,
            last_sync_observation: Mutex::new(None),
        }
    }

    pub async fn last_sync_observation(&self) -> Option<SyncObservation> {
        *self.last_sync_observation.lock().await
    }

    pub fn build_prompt_payload(&self, entries: &[TeamMemEntry]) -> String {
        entries
            .iter()
            .filter_map(|entry| {
                if let Err(reason) = validate_path_key(&entry.path_key) {
                    self.audit_log
                        .record(TeamMemAuditEvent::InvalidPathKeyBlocked {
                            path_key: sanitize_for_log(&entry.path_key),
                            reason,
                        });
                    return None;
                }

                if detect_secret(&entry.content) {
                    self.audit_log.record(TeamMemAuditEvent::SecretSkipped {
                        path_key: sanitize_for_log(&entry.path_key),
                    });
                    return None;
                }

                Some(format!("{}: {}", entry.path_key, entry.content))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[async_trait]
impl<B> TeamMemorySyncService for TeamMemKairosService<B>
where
    B: TeamMemBackend,
{
    async fn pull(&self) -> Result<SyncResult, TeamMemError> {
        match self.backend.pull_once().await {
            Ok(result) => Ok(result),
            Err(BackendFailure::Conflict) => Err(TeamMemError::ConflictExhausted),
            Err(BackendFailure::Fatal(err)) => Err(err),
        }
    }

    async fn push(&self) -> Result<SyncResult, TeamMemError> {
        match self.backend.push_once().await {
            Ok(result) => Ok(result),
            Err(BackendFailure::Conflict) => Err(TeamMemError::ConflictExhausted),
            Err(BackendFailure::Fatal(err)) => Err(err),
        }
    }

    async fn sync(&self) -> Result<SyncResult, TeamMemError> {
        let mut attempts = 0u8;
        let mut retries = 0u8;

        loop {
            attempts = attempts.saturating_add(1);
            match self.backend.sync_once().await {
                Ok(result) => {
                    *self.last_sync_observation.lock().await = Some(SyncObservation {
                        attempts,
                        retries,
                        conflict_exhausted: false,
                    });
                    return Ok(result);
                }
                Err(BackendFailure::Conflict) => {
                    if retries >= self.max_conflict_retries {
                        self.audit_log
                            .record(TeamMemAuditEvent::ConflictExhausted { attempts });
                        *self.last_sync_observation.lock().await = Some(SyncObservation {
                            attempts,
                            retries,
                            conflict_exhausted: true,
                        });
                        return Err(TeamMemError::ConflictExhausted);
                    }

                    retries = retries.saturating_add(1);
                    self.audit_log.record(TeamMemAuditEvent::ConflictRetry {
                        attempt: retries,
                        max_retries: self.max_conflict_retries,
                    });
                }
                Err(BackendFailure::Fatal(err)) => {
                    *self.last_sync_observation.lock().await = Some(SyncObservation {
                        attempts,
                        retries,
                        conflict_exhausted: false,
                    });
                    return Err(err);
                }
            }
        }
    }
}

fn validate_path_key(key: &str) -> Result<(), &'static str> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("empty path key");
    }
    if trimmed.starts_with('/') {
        return Err("absolute path is not allowed");
    }
    if trimmed.contains("..") {
        return Err("parent traversal is not allowed");
    }
    if trimmed
        .chars()
        .any(|ch| ch.is_ascii_control() || ch == '\u{2028}' || ch == '\u{2029}')
    {
        return Err("control characters are not allowed");
    }
    Ok(())
}

fn sanitize_for_log(input: &str) -> String {
    const MAX_LOG_CHARS: usize = 48;
    let mut out = String::new();
    for ch in input.chars().take(MAX_LOG_CHARS) {
        if ch.is_ascii_control() || ch == '\u{2028}' || ch == '\u{2029}' {
            out.push('?');
        } else {
            out.push(ch);
        }
    }
    out
}

fn detect_secret(content: &str) -> bool {
    looks_like_openai_key(content)
        || content.contains("ghp_")
        || content.contains("AKIA")
        || content.contains("PRIVATE KEY-----")
}

fn looks_like_openai_key(content: &str) -> bool {
    content.split_whitespace().any(|token| {
        if let Some(rest) = token.strip_prefix("sk-") {
            rest.chars().all(|ch| ch.is_ascii_alphanumeric()) && rest.len() >= 20
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex as StdMutex};

    #[derive(Default)]
    struct RecordingAuditLog {
        events: StdMutex<Vec<TeamMemAuditEvent>>,
    }

    impl TeamMemAuditLog for RecordingAuditLog {
        fn record(&self, event: TeamMemAuditEvent) {
            self.events.lock().expect("poisoned").push(event);
        }
    }

    struct MockTeamMemBackend {
        sync_results: Mutex<VecDeque<Result<SyncResult, BackendFailure>>>,
    }

    #[async_trait]
    impl TeamMemBackend for MockTeamMemBackend {
        async fn pull_once(&self) -> Result<SyncResult, BackendFailure> {
            Ok(SyncResult {
                files_pulled: 0,
                files_pushed: 0,
            })
        }

        async fn push_once(&self) -> Result<SyncResult, BackendFailure> {
            Ok(SyncResult {
                files_pulled: 0,
                files_pushed: 0,
            })
        }

        async fn sync_once(&self) -> Result<SyncResult, BackendFailure> {
            self.sync_results
                .lock()
                .await
                .pop_front()
                .expect("missing sync result")
        }
    }

    #[test]
    fn secret_detected_entries_are_never_emitted_in_prompt_payload() {
        let audit = Arc::new(RecordingAuditLog::default());
        let service = TeamMemKairosService::new(
            MockTeamMemBackend {
                sync_results: Mutex::new(VecDeque::new()),
            },
            2,
            audit.clone(),
        );

        let payload = service.build_prompt_payload(&[
            TeamMemEntry {
                path_key: "notes/safe.md".to_string(),
                content: "hello world".to_string(),
            },
            TeamMemEntry {
                path_key: "notes/secret.md".to_string(),
                content: "token sk-12345678901234567890".to_string(),
            },
        ]);

        assert!(payload.contains("notes/safe.md: hello world"));
        assert!(!payload.contains("sk-12345678901234567890"));
        let events = audit.events.lock().expect("poisoned");
        assert!(events.iter().any(|event| matches!(
            event,
            TeamMemAuditEvent::SecretSkipped { path_key } if path_key == "notes/secret.md"
        )));
    }

    #[test]
    fn invalid_path_keys_are_blocked_and_logged_safely() {
        let audit = Arc::new(RecordingAuditLog::default());
        let service = TeamMemKairosService::new(
            MockTeamMemBackend {
                sync_results: Mutex::new(VecDeque::new()),
            },
            1,
            audit.clone(),
        );

        let payload = service.build_prompt_payload(&[
            TeamMemEntry {
                path_key: "../private\nkey".to_string(),
                content: "should not pass".to_string(),
            },
            TeamMemEntry {
                path_key: "notes/ok.md".to_string(),
                content: "kept".to_string(),
            },
        ]);

        assert!(!payload.contains("should not pass"));
        assert!(payload.contains("notes/ok.md: kept"));

        let events = audit.events.lock().expect("poisoned");
        assert!(events.iter().any(|event| matches!(
            event,
            TeamMemAuditEvent::InvalidPathKeyBlocked { path_key, .. }
                if path_key == "../private?key"
        )));
    }

    #[tokio::test]
    async fn conflict_retries_are_bounded_and_observable() {
        let audit = Arc::new(RecordingAuditLog::default());
        let service = TeamMemKairosService::new(
            MockTeamMemBackend {
                sync_results: Mutex::new(
                    vec![
                        Err(BackendFailure::Conflict),
                        Err(BackendFailure::Conflict),
                        Err(BackendFailure::Conflict),
                    ]
                    .into(),
                ),
            },
            2,
            audit.clone(),
        );

        let result = service.sync().await;
        assert!(matches!(result, Err(TeamMemError::ConflictExhausted)));

        let observation = service
            .last_sync_observation()
            .await
            .expect("observation should be stored");
        assert_eq!(observation.attempts, 3);
        assert_eq!(observation.retries, 2);
        assert!(observation.conflict_exhausted);

        let events = audit.events.lock().expect("poisoned");
        let retry_count = events
            .iter()
            .filter(|event| matches!(event, TeamMemAuditEvent::ConflictRetry { .. }))
            .count();
        assert_eq!(retry_count, 2);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, TeamMemAuditEvent::ConflictExhausted { attempts: 3 }))
        );
    }
}
