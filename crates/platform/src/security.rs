use regex::Regex;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretFinding {
    pub rule_id: String,
    pub label: String,
}

pub trait SecretScanner: Send + Sync {
    fn scan(&self, content: &str) -> Vec<SecretFinding>;
}

struct SecretRule {
    rule_id: &'static str,
    label: &'static str,
    pattern: Regex,
}

pub struct DefaultSecretScanner {
    rules: Vec<SecretRule>,
}

impl DefaultSecretScanner {
    pub fn new() -> Self {
        let rules = vec![
            SecretRule {
                rule_id: "openai_api_key",
                label: "OpenAI API key",
                pattern: Regex::new(r"sk-(?:proj-)?[A-Za-z0-9_-]{20,}").expect("valid regex"),
            },
            SecretRule {
                rule_id: "github_pat",
                label: "GitHub personal access token",
                pattern: Regex::new(r"ghp_[A-Za-z0-9]{36}").expect("valid regex"),
            },
            SecretRule {
                rule_id: "aws_access_key_id",
                label: "AWS access key id",
                pattern: Regex::new(r"AKIA[0-9A-Z]{16}").expect("valid regex"),
            },
            SecretRule {
                rule_id: "private_key_block",
                label: "Private key block",
                pattern: Regex::new(r"-----BEGIN(?: [A-Z0-9]+)? PRIVATE KEY-----")
                    .expect("valid regex"),
            },
        ];
        Self { rules }
    }
}

impl Default for DefaultSecretScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretScanner for DefaultSecretScanner {
    fn scan(&self, content: &str) -> Vec<SecretFinding> {
        self.rules
            .iter()
            .filter(|rule| rule.pattern.is_match(content))
            .map(|rule| SecretFinding {
                rule_id: rule.rule_id.to_string(),
                label: rule.label.to_string(),
            })
            .collect()
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PathValidationError {
    #[error("path validation failed: parent traversal ('..') is denied")]
    ParentTraversalDenied,
    #[error("path validation failed: root path not found")]
    RootNotFound,
    #[error("path validation failed: path resolves outside root")]
    OutsideRoot,
    #[error("path validation failed: invalid target path")]
    InvalidTargetPath,
    #[error("path validation failed: io error")]
    Io,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PolicyError {
    #[error("{0}")]
    Path(#[from] PathValidationError),
    #[error("secret detected")]
    SecretDetected { findings: Vec<SecretFinding> },
}

pub struct ScanThenWritePolicy<S> {
    scanner: S,
}

impl<S> ScanThenWritePolicy<S>
where
    S: SecretScanner,
{
    pub fn new(scanner: S) -> Self {
        Self { scanner }
    }

    pub fn validate_then_scan(
        &self,
        root: &Path,
        target: &Path,
        content: &str,
    ) -> Result<PathBuf, PolicyError> {
        let validated = validate_canonical_path(root, target)?;
        let findings = self.scanner.scan(content);
        if findings.is_empty() {
            Ok(validated)
        } else {
            Err(PolicyError::SecretDetected { findings })
        }
    }

    pub fn scan_remote_payload(&self, content: &str) -> Result<(), PolicyError> {
        let findings = self.scanner.scan(content);
        if findings.is_empty() {
            Ok(())
        } else {
            Err(PolicyError::SecretDetected { findings })
        }
    }
}

pub fn validate_canonical_path(root: &Path, target: &Path) -> Result<PathBuf, PathValidationError> {
    if target
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(PathValidationError::ParentTraversalDenied);
    }

    let canonical_root = root
        .canonicalize()
        .map_err(|_| PathValidationError::RootNotFound)?;

    let joined = if target.is_absolute() {
        target.to_path_buf()
    } else {
        canonical_root.join(target)
    };

    let canonical_target = canonicalize_with_missing_segments(&joined)?;
    if canonical_target.starts_with(&canonical_root) {
        Ok(canonical_target)
    } else {
        Err(PathValidationError::OutsideRoot)
    }
}

fn canonicalize_with_missing_segments(path: &Path) -> Result<PathBuf, PathValidationError> {
    let mut suffix = Vec::<OsString>::new();
    let mut cursor = path.to_path_buf();

    loop {
        if cursor.exists() {
            let mut canonical = cursor.canonicalize().map_err(|_| PathValidationError::Io)?;
            for segment in suffix.iter().rev() {
                canonical.push(segment);
            }
            return Ok(canonical);
        }

        let Some(name) = cursor.file_name() else {
            return Err(PathValidationError::InvalidTargetPath);
        };
        suffix.push(name.to_os_string());

        let Some(parent) = cursor.parent() else {
            return Err(PathValidationError::InvalidTargetPath);
        };
        cursor = parent.to_path_buf();
    }
}
