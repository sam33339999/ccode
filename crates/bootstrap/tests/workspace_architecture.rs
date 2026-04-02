use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Package {
    id: String,
    name: String,
    dependencies: Vec<Dependency>,
}

#[derive(Debug, Deserialize)]
struct Dependency {
    name: String,
}

#[test]
fn workspace_structure_and_dependency_rules_hold() {
    let workspace_root = workspace_root();
    let metadata = load_metadata(&workspace_root);

    let member_ids: HashSet<&str> = metadata
        .workspace_members
        .iter()
        .map(String::as_str)
        .collect();

    let mut workspace_pkgs = HashMap::new();
    for pkg in &metadata.packages {
        if member_ids.contains(pkg.id.as_str()) {
            workspace_pkgs.insert(pkg.name.clone(), pkg);
        }
    }

    let expected_members: HashSet<&str> = [
        "ccode-domain",
        "ccode-ports",
        "ccode-config",
        "ccode-provider",
        "ccode-tools",
        "ccode-mcp-runtime",
        "ccode-remote-runtime",
        "ccode-platform",
        "ccode-application",
        "ccode-session",
        "ccode-cron",
        "ccode-cli",
        "ccode-state-store",
    ]
    .into_iter()
    .collect();

    for required in &expected_members {
        assert!(
            workspace_pkgs.contains_key(*required),
            "missing workspace crate: {required}"
        );
    }

    let workspace_names: HashSet<&str> = workspace_pkgs.keys().map(String::as_str).collect();
    let deps_of = |name: &str| -> HashSet<String> {
        let pkg = workspace_pkgs
            .get(name)
            .unwrap_or_else(|| panic!("workspace package not found: {name}"));

        pkg.dependencies
            .iter()
            .filter_map(|dep| {
                if workspace_names.contains(dep.name.as_str()) {
                    Some(dep.name.clone())
                } else {
                    None
                }
            })
            .collect()
    };

    assert!(
        deps_of("ccode-domain").is_empty(),
        "domain must have zero internal deps"
    );

    assert_eq!(
        deps_of("ccode-ports"),
        HashSet::from(["ccode-domain".to_string()]),
        "ports must depend only on domain"
    );

    assert_eq!(
        deps_of("ccode-application"),
        HashSet::from(["ccode-domain".to_string(), "ccode-ports".to_string(),]),
        "application must depend only on ports + domain (Bridge x Ultraplan remote session APIs must flow through app-services contracts)"
    );

    assert_eq!(
        deps_of("ccode-cli"),
        HashSet::from([
            "ccode-application".to_string(),
            "ccode-bootstrap".to_string(),
        ]),
        "cli must depend only on application + bootstrap"
    );

    let forbidden_from_domain: HashSet<&str> =
        ["ccode-ports", "ccode-provider", "ccode-tools", "ccode-cli"]
            .into_iter()
            .collect();
    let domain_deps = deps_of("ccode-domain");
    for forbidden in forbidden_from_domain {
        assert!(
            !domain_deps.contains(forbidden),
            "forbidden edge: ccode-domain -> {forbidden}"
        );
    }

    let forbidden_from_ports: HashSet<&str> = ["ccode-provider", "ccode-tools", "ccode-cli"]
        .into_iter()
        .collect();
    let ports_deps = deps_of("ccode-ports");
    for forbidden in forbidden_from_ports {
        assert!(
            !ports_deps.contains(forbidden),
            "forbidden edge: ccode-ports -> {forbidden}"
        );
    }
}

fn load_metadata(workspace_root: &Path) -> Metadata {
    let output = Command::new("cargo")
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--no-deps")
        .current_dir(workspace_root)
        .output()
        .expect("failed to run cargo metadata");

    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).expect("invalid cargo metadata JSON")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("failed to resolve workspace root")
}
