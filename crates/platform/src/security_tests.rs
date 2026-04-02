#[cfg(test)]
mod tests {
    use crate::security::{
        DefaultSecretScanner, ScanThenWritePolicy, SecretScanner, validate_canonical_path,
    };
    use std::path::Path;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("ccode-us021-{label}-{ts}"));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn rejects_parent_directory_traversal() {
        let root = unique_temp_dir("traversal");
        let err =
            validate_canonical_path(&root, Path::new("../secrets.txt")).expect_err("must reject");
        assert_eq!(
            err.to_string(),
            "path validation failed: parent traversal ('..') is denied"
        );
    }

    #[test]
    fn rejects_paths_outside_root() {
        let root = unique_temp_dir("outside-root");
        let outside = unique_temp_dir("outside-target").join("note.txt");

        let err = validate_canonical_path(&root, &outside).expect_err("outside root should fail");
        assert_eq!(
            err.to_string(),
            "path validation failed: path resolves outside root"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        use std::os::unix::fs as unix_fs;

        let root = unique_temp_dir("symlink-root");
        let outside = unique_temp_dir("symlink-outside");
        let link = root.join("link");
        unix_fs::symlink(&outside, &link).expect("symlink");

        let err = validate_canonical_path(&root, Path::new("link/stolen.txt"))
            .expect_err("symlink escape should fail");
        assert_eq!(
            err.to_string(),
            "path validation failed: path resolves outside root"
        );
    }

    #[test]
    fn detects_secret_patterns_with_audit_labels() {
        let scanner = DefaultSecretScanner::new();
        let findings = scanner.scan("token=ghp_123456789012345678901234567890123456");

        assert!(!findings.is_empty());
        assert!(findings.iter().any(|f| f.rule_id == "github_pat"));
        assert!(findings.iter().any(|f| !f.label.is_empty()));
    }

    #[test]
    fn detects_openai_project_style_keys() {
        let scanner = DefaultSecretScanner::new();
        let findings = scanner.scan("OPENAI_API_KEY=sk-proj-1234567890abcdef1234567890abcdef");

        assert!(findings.iter().any(|f| f.rule_id == "openai_api_key"));
    }

    struct CountingScanner {
        calls: Arc<AtomicUsize>,
    }

    impl SecretScanner for CountingScanner {
        fn scan(&self, _content: &str) -> Vec<crate::security::SecretFinding> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Vec::new()
        }
    }

    #[test]
    fn enforces_validate_then_scan_order_before_write() {
        let root = unique_temp_dir("order");
        std::fs::create_dir_all(root.join("safe")).expect("safe dir");
        let calls = Arc::new(AtomicUsize::new(0));
        let policy = ScanThenWritePolicy::new(CountingScanner {
            calls: Arc::clone(&calls),
        });

        let validated = policy
            .validate_then_scan(&root, Path::new("safe/file.txt"), "hello")
            .expect("policy should pass");

        let canonical_root = root.canonicalize().expect("canonical root");
        assert!(validated.starts_with(&canonical_root));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
