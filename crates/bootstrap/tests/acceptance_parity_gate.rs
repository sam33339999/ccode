use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use ccode_bootstrap::build_tool_registry;

#[test]
fn acceptance_parity_gate_artifacts_and_controls_exist() {
    let root = workspace_root();

    let parity = read_text(&root.join("docs/parity-matrix.md"));
    assert!(
        parity.contains("P0 command parity threshold: 100%"),
        "parity matrix must define P0 command threshold"
    );
    assert!(
        parity.contains("P1 command parity threshold: 95%"),
        "parity matrix must define P1 command threshold"
    );
    assert!(
        parity.contains("P0 tool parity threshold: 100%"),
        "parity matrix must define P0 tool threshold"
    );
    assert!(
        parity.contains("P1 tool parity threshold: 95%"),
        "parity matrix must define P1 tool threshold"
    );

    for command in [
        "ccode health",
        "ccode sessions",
        "ccode agent",
        "ccode repl",
        "ccode tui",
        "ccode cron",
    ] {
        assert!(
            parity.contains(command),
            "parity matrix missing command inventory item: {command}"
        );
    }

    for tool in [
        "fs_read",
        "fs_write",
        "fs_edit",
        "fs_list",
        "fs_grep",
        "fs_glob",
        "shell",
        "web_fetch",
        "browser",
    ] {
        assert!(
            parity.contains(tool),
            "parity matrix missing tool inventory item: {tool}"
        );
    }

    let cmd_mod = read_text(&root.join("crates/cli/src/cmd/mod.rs"));
    for variant in ["Health", "Sessions", "Agent", "Repl", "Tui", "Cron"] {
        assert!(
            cmd_mod.contains(variant),
            "CLI command enum missing expected variant: {variant}"
        );
    }

    let registry = build_tool_registry(PathBuf::from("."), None, None, Vec::new());
    let registered: HashSet<String> = registry.definitions().into_iter().map(|d| d.name).collect();
    for required in [
        "fs_read",
        "fs_write",
        "fs_edit",
        "fs_list",
        "fs_grep",
        "fs_glob",
        "shell",
        "web_fetch",
        "browser",
    ] {
        assert!(
            registered.contains(required),
            "tool registry missing required tool: {required}"
        );
    }

    let sessions_cmd = read_text(&root.join("crates/cli/src/cmd/sessions.rs"));
    assert!(
        sessions_cmd.contains("dry_run"),
        "sessions command must support --dry-run for destructive operations"
    );

    let cron_cmd = read_text(&root.join("crates/cli/src/cmd/cron.rs"));
    assert!(
        cron_cmd.contains("dry_run"),
        "cron command must support --dry-run for destructive operations"
    );

    let runbook = read_text(&root.join("docs/operator-runbook.md"));
    for section in [
        "Startup",
        "Shutdown",
        "Failure Triage",
        "Rollback",
        "Incident Playbooks",
    ] {
        assert!(
            runbook.contains(section),
            "operator runbook missing section: {section}"
        );
    }

    let signoff = read_text(&root.join("docs/release-signoff-checklist.md"));
    for item in [
        "Architecture acceptance passed",
        "Constants governance acceptance passed",
        "Functional parity acceptance passed",
        "Non-functional acceptance passed",
        "CI/CD gates passed",
        "Documentation acceptance passed",
        "Stakeholder sign-off from Engineering, SRE/Platform, and Security",
    ] {
        assert!(
            signoff.contains(item),
            "sign-off checklist missing section 9 item: {item}"
        );
    }
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("failed to resolve workspace root")
}
