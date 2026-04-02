use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
struct ContractRule {
    codename: &'static str,
    doc: &'static str,
    required_traits: &'static [&'static str],
    required_errors: &'static [&'static str],
    allowed_trait_crates: &'static [&'static str],
    requires_cli_or_security_matrix: bool,
}

const LOCKED_CORE_CONTRACTS: &[&str] = &[
    "bridge-mode-contract.md",
    "agent-triggers-contract.md",
    "chicago-mcp-contract.md",
    "teammem-contract.md",
    "ultraplan-contract.md",
    "kairos-contract.md",
    "coordinator-mode-contract.md",
    "llm-compat-contract.md",
];

const CONTRACT_RULES: &[ContractRule] = &[
    ContractRule {
        codename: "BRIDGE_MODE",
        doc: "bridge-mode-contract.md",
        required_traits: &["RemoteSessionService", "CcrClient"],
        required_errors: &["RemoteSessionError", "CcrClientError"],
        allowed_trait_crates: &["crates/application/", "crates/remote-runtime/"],
        requires_cli_or_security_matrix: true,
    },
    ContractRule {
        codename: "AGENT_TRIGGERS",
        doc: "agent-triggers-contract.md",
        required_traits: &["TriggerSchedulerService", "RemoteTriggerDispatchService"],
        required_errors: &["TriggerError"],
        allowed_trait_crates: &["crates/application/"],
        requires_cli_or_security_matrix: true,
    },
    ContractRule {
        codename: "CHICAGO_MCP",
        doc: "chicago-mcp-contract.md",
        required_traits: &["McpCapabilityPolicy", "ComputerUseLifecycle"],
        required_errors: &["McpPolicyError", "McpRuntimeError"],
        allowed_trait_crates: &["crates/mcp-runtime/"],
        requires_cli_or_security_matrix: true,
    },
    ContractRule {
        codename: "TEAMMEM",
        doc: "teammem-contract.md",
        required_traits: &["TeamMemorySyncService"],
        required_errors: &["TeamMemError"],
        allowed_trait_crates: &["crates/application/"],
        requires_cli_or_security_matrix: true,
    },
    ContractRule {
        codename: "ULTRAPLAN",
        doc: "ultraplan-contract.md",
        required_traits: &["UltraplanService"],
        required_errors: &["UltraplanError"],
        allowed_trait_crates: &["crates/application/"],
        requires_cli_or_security_matrix: true,
    },
    ContractRule {
        codename: "KAIROS",
        doc: "kairos-contract.md",
        required_traits: &["AssistantModeService"],
        required_errors: &["KairosError"],
        allowed_trait_crates: &["crates/application/"],
        requires_cli_or_security_matrix: false,
    },
    ContractRule {
        codename: "COORDINATOR_MODE",
        doc: "coordinator-mode-contract.md",
        required_traits: &["ModeCoordinatorService"],
        required_errors: &["CoordinatorModeError"],
        allowed_trait_crates: &["crates/application/"],
        requires_cli_or_security_matrix: false,
    },
    ContractRule {
        codename: "LLM_COMPAT",
        doc: "llm-compat-contract.md",
        required_traits: &["LlmClient"],
        required_errors: &["LlmError"],
        allowed_trait_crates: &["crates/ports/"],
        requires_cli_or_security_matrix: false,
    },
    // Cross-spec orchestration contract is also enforced for consistency checks.
    ContractRule {
        codename: "MULTI_AGENT_ORCHESTRATION",
        doc: "multi-agent-orchestration-contract.md",
        required_traits: &["MultiAgentOrchestrator"],
        required_errors: &["OrchestrationError"],
        allowed_trait_crates: &["crates/application/"],
        requires_cli_or_security_matrix: false,
    },
];

#[test]
fn spec_governance_gate_validation_passes() {
    let workspace_root = workspace_root();
    let readme = read_text(&workspace_root.join("docs/README.md"));
    let acceptance_playbook = read_text(&workspace_root.join("docs/acceptance-playbook.md"));
    let acceptance_spec = read_text(&workspace_root.join("docs/acceptance-spec.md"));
    let spec_lock = read_text(&workspace_root.join("docs/spec-lock-v1.md"));
    let codename_mapping = read_text(&workspace_root.join("docs/codename-mapping.md"));
    let integration_contracts = read_text(&workspace_root.join("docs/integration-contracts.md"));

    assert!(
        acceptance_playbook.contains("## 1. Spec QA Checklist"),
        "acceptance-playbook must define the spec QA checklist"
    );

    for rule in CONTRACT_RULES {
        let doc_path = workspace_root.join("docs").join(rule.doc);
        let doc = read_text(&doc_path);

        // Spec QA checklist from acceptance-playbook.md
        assert!(
            count_evidence_anchors(&doc) >= 6,
            "{} must contain at least 6 code evidence anchors",
            rule.doc
        );
        assert!(
            has_crate_boundary_section(&doc),
            "{} must contain a crate boundary/ownership section",
            rule.doc
        );
        assert!(
            has_constants_classification(&doc),
            "{} must contain constants classification",
            rule.doc
        );
        assert!(
            has_error_taxonomy_section(&doc),
            "{} must contain an error taxonomy section",
            rule.doc
        );
        assert!(
            has_contract_and_integration_matrix(&doc),
            "{} must include contract + integration tests in acceptance matrix",
            rule.doc
        );
        if rule.requires_cli_or_security_matrix {
            assert!(
                doc.contains("CLI behavior tests") || doc.contains("Security tests"),
                "{} must include CLI behavior tests or security tests",
                rule.doc
            );
        }

        for trait_name in rule.required_traits {
            assert!(
                has_trait_with_methods(&doc, trait_name),
                "{} must include explicit method signatures for trait {}",
                rule.doc,
                trait_name
            );
        }

        for err in rule.required_errors {
            assert!(
                err.ends_with("Error"),
                "error type naming must be *Error: {err}"
            );
        }

        // Cross-spec consistency checks
        assert!(
            has_fail_closed_policy_text(&doc),
            "{} must encode fail-closed policy wording",
            rule.doc
        );
    }

    // acceptance-playbook IME requirement encoded in acceptance-spec
    let ime_required_terms = [
        "Composition text (preedit)",
        "committed text",
        "delete",
        "display width",
    ];
    for term in ime_required_terms {
        assert!(
            acceptance_spec.contains(term),
            "acceptance-spec.md IME section missing term: {term}"
        );
    }

    // state-machine terminal state policy is explicit where state machines are defined
    for doc in [
        "bridge-mode-contract.md",
        "ultraplan-contract.md",
        "multi-agent-orchestration-contract.md",
    ] {
        let text = read_text(&workspace_root.join("docs").join(doc));
        assert!(
            text.to_lowercase().contains("terminal"),
            "{doc} must define terminal state semantics"
        );
    }

    // remote side-effects must include recoverable/idempotent semantics
    for text in [
        &read_text(&workspace_root.join("docs/bridge-mode-contract.md")),
        &read_text(&workspace_root.join("docs/ultraplan-contract.md")),
        &integration_contracts,
    ] {
        let lower = text.to_lowercase();
        assert!(
            lower.contains("idempotent") || lower.contains("cleanup") || lower.contains("orphan"),
            "remote side-effect contracts must encode idempotent/cleanup/orphan behavior"
        );
    }

    // spec-lock-v1 release gate checks
    assert!(
        spec_lock.contains("acceptance-spec.md"),
        "spec-lock-v1 must lock acceptance-spec.md"
    );
    for contract in LOCKED_CORE_CONTRACTS {
        assert!(
            spec_lock.contains(contract),
            "spec-lock-v1 missing locked contract: {contract}"
        );
    }
    assert!(
        spec_lock.contains("No waiver without expiry is allowed."),
        "spec-lock-v1 must fail-closed on waiver expiry"
    );
    for waiver_field in ["Scope", "Owner", "Risk", "Expiry"] {
        assert!(
            spec_lock.contains(waiver_field),
            "spec-lock-v1 waiver policy missing required field: {waiver_field}"
        );
    }
    assert!(
        spec_lock.contains("Breaking spec changes require `spec-lock-v2.md`"),
        "breaking locked-spec changes must require explicit next lock version"
    );

    // crate-boundary and code-anchor checks: service traits and error taxonomies must exist in code.
    let rust_sources = collect_rust_sources(&workspace_root.join("crates"));

    for rule in CONTRACT_RULES {
        for trait_name in rule.required_traits {
            let hits = find_token_hits(&rust_sources, &format!("trait {trait_name}"));
            assert!(
                !hits.is_empty(),
                "missing code trait anchor for {} ({})",
                trait_name,
                rule.codename
            );
            assert!(
                hits.iter().any(|p| {
                    rule.allowed_trait_crates
                        .iter()
                        .any(|allowed| p.contains(allowed))
                }),
                "trait {} for {} must be declared in one of {:?}, found in {:?}",
                trait_name,
                rule.codename,
                rule.allowed_trait_crates,
                hits
            );
        }

        for err_name in rule.required_errors {
            let hits = find_token_hits(&rust_sources, &format!("enum {err_name}"));
            assert!(
                !hits.is_empty(),
                "missing code error taxonomy anchor for {} ({})",
                err_name,
                rule.codename
            );
        }
    }

    // codename-mapping and README consistency at crate-name vocabulary level.
    for crate_name in [
        "app-services",
        "remote-runtime",
        "mcp-runtime",
        "state-store",
        "config",
    ] {
        assert!(
            codename_mapping.contains(crate_name),
            "codename-mapping.md must mention crate boundary: {crate_name}"
        );
        assert!(
            readme.contains(crate_name),
            "docs/README.md must mention crate boundary: {crate_name}"
        );
    }
}

fn has_trait_with_methods(doc: &str, trait_name: &str) -> bool {
    let marker = format!("trait {trait_name}");
    let Some(start) = doc.find(&marker) else {
        return false;
    };

    let tail = &doc[start..doc.len().min(start + 900)];
    tail.contains("fn ") || tail.contains("async fn ")
}

fn has_crate_boundary_section(doc: &str) -> bool {
    doc.contains("Boundary Mapping")
        || doc.contains("Ownership Boundaries")
        || doc.contains("Crate mapping")
        || doc.contains("Crate mapping")
}

fn has_constants_classification(doc: &str) -> bool {
    doc.contains("Constants Classification") || doc.contains("constants classification")
}

fn has_error_taxonomy_section(doc: &str) -> bool {
    doc.contains("Error Taxonomy")
}

fn has_contract_and_integration_matrix(doc: &str) -> bool {
    doc.contains("Contract tests") && doc.contains("Integration tests")
}

fn has_fail_closed_policy_text(doc: &str) -> bool {
    let lower = doc.to_lowercase();
    let explicit_guardrails = [
        "fail-close",
        "fail-closed",
        "disabled by policy",
        "gate off",
        "hard-rejected",
        "requires gate",
        "denied",
        "requires both",
        "disabledbypolicy",
        "gatedisabled",
        "never sees raw",
        "no provider wire types may appear",
        "blocks orchestration calls",
        "policyviolation",
    ]
    .iter()
    .any(|needle| lower.contains(needle));

    if explicit_guardrails {
        return true;
    }

    // TEAMMEM-style deny-by-default language: mandatory guard ordering.
    lower.contains("policy rules")
        && lower.contains("all remote writes pass")
        && lower.contains("all local writes pass")
        && lower.contains("first")
}

fn count_evidence_anchors(doc: &str) -> usize {
    doc.split('`')
        .filter(|segment| {
            let has_path = segment.contains(".ts")
                || segment.contains(".tsx")
                || segment.contains(".rs")
                || segment.contains(".md");
            has_path && (segment.contains('/') || segment.contains(':'))
        })
        .count()
}

fn collect_rust_sources(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    collect_rust_sources_recursive(root, &mut out);
    out
}

fn collect_rust_sources_recursive(root: &Path, out: &mut Vec<(String, String)>) {
    let entries =
        fs::read_dir(root).unwrap_or_else(|e| panic!("failed to read {}: {e}", root.display()));
    for entry in entries {
        let entry = entry.expect("invalid dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources_recursive(&path, out);
            continue;
        }
        if path.extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        let text = read_text(&path);
        out.push((path.to_string_lossy().to_string(), text));
    }
}

fn find_token_hits(sources: &[(String, String)], token: &str) -> Vec<String> {
    sources
        .iter()
        .filter_map(|(path, text)| {
            if text.contains(token) {
                Some(path.clone())
            } else {
                None
            }
        })
        .collect()
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
