#![allow(dead_code)]
// Chicago MCP CLI rendering helpers and behaviour tests (US-038 §8.3).
//
// Pure rendering functions — no I/O, no async — so they are fully testable
// without any mock infrastructure.
//
// The rendering helpers are not yet wired to a CLI subcommand; the
// dead_code lint is suppressed until the command is plumbed in.

/// Routes `--computer-use-mcp` to the privileged adapter path.
///
/// Returns `true` when the flag is present and the request should be
/// dispatched to the privileged computer-use MCP adapter rather than
/// the generic MCP pipeline.
pub fn should_use_privileged_adapter(computer_use_mcp_flag: bool) -> bool {
    computer_use_mcp_flag
}

/// Renders a user-facing error message from an MCP policy error class.
///
/// Exposes the error class (e.g. "reserved server name") but never
/// surfaces native stack traces, internal memory addresses, or other
/// sensitive implementation details.
pub fn render_mcp_policy_error(error_class: &str) -> String {
    match error_class {
        "ReservedServerName" => "Server registration rejected: the requested name is reserved. \
             Choose a different server name."
            .to_string(),
        "FeatureGateDisabled" => render_gate_disabled_reason(),
        "PrivilegedCapabilityDenied" => "Privileged computer-use capability was denied by policy. \
             Check your configuration allows privileged MCP access."
            .to_string(),
        "TransportError" => {
            "MCP transport error occurred. Check server connectivity and retry.".to_string()
        }
        "InvalidToolPayload" => {
            "Invalid tool payload: the request or response did not match the expected schema."
                .to_string()
        }
        "CleanupFailed" => {
            "Cleanup did not complete successfully. The session may require manual cleanup."
                .to_string()
        }
        other => format!("MCP policy error: {other}"),
    }
}

/// Returns the deterministic reason string for a disabled gate path.
pub fn render_gate_disabled_reason() -> String {
    "Computer-use MCP is not available: the chicago_mcp feature gate is disabled. \
     Enable the gate in your configuration to use this feature."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── §8.3 CLI behaviour tests ───────────────────────────────────────────

    /// AC: --computer-use-mcp routes to privileged adapter path.
    ///
    /// When the flag is true the router must select the privileged adapter;
    /// when false it must select the standard path.
    #[test]
    fn cli_computer_use_mcp_routes_to_privileged_adapter() {
        assert!(
            should_use_privileged_adapter(true),
            "--computer-use-mcp must route to privileged adapter"
        );
        assert!(
            !should_use_privileged_adapter(false),
            "absence of --computer-use-mcp must route to standard path"
        );
    }

    /// AC: Error output exposes class, not sensitive native details.
    ///
    /// Each policy error variant must produce a user-readable message that
    /// includes the error class but never leaks memory addresses, stack
    /// traces, or internal type names.
    #[test]
    fn cli_error_output_exposes_class_not_native_details() {
        let cases = vec![
            ("ReservedServerName", "reserved"),
            ("FeatureGateDisabled", "feature gate"),
            ("PrivilegedCapabilityDenied", "denied"),
            ("TransportError", "transport"),
            ("InvalidToolPayload", "payload"),
            ("CleanupFailed", "cleanup"),
        ];

        for (error_class, expected_class_hint) in cases {
            let rendered = render_mcp_policy_error(error_class);

            // Must contain the class-level hint
            assert!(
                rendered.to_ascii_lowercase().contains(expected_class_hint),
                "rendered message for {error_class} must contain '{expected_class_hint}', \
                 got: {rendered}"
            );

            // Must NOT contain sensitive native details
            assert!(
                !rendered.contains("0x"),
                "rendered message must not contain memory addresses: {rendered}"
            );
            assert!(
                !rendered.contains("at src/"),
                "rendered message must not contain source paths: {rendered}"
            );
            assert!(
                !rendered.contains("thread '"),
                "rendered message must not contain thread names: {rendered}"
            );
            assert!(
                !rendered.contains("panicked"),
                "rendered message must not contain panic traces: {rendered}"
            );
        }
    }

    /// AC: Disabled gate path gives deterministic reason.
    ///
    /// The message must be stable across invocations and contain actionable
    /// guidance for the user.
    #[test]
    fn cli_disabled_gate_path_gives_deterministic_reason() {
        let msg1 = render_gate_disabled_reason();
        let msg2 = render_gate_disabled_reason();

        assert_eq!(
            msg1, msg2,
            "gate-disabled message must be deterministic across calls"
        );

        assert!(
            msg1.contains("disabled"),
            "message must indicate the gate is disabled: {msg1}"
        );
        assert!(
            msg1.contains("Enable"),
            "message must contain actionable guidance: {msg1}"
        );
    }

    /// Companion: FeatureGateDisabled error and render_gate_disabled_reason
    /// produce the same message — they are the same user-facing path.
    #[test]
    fn cli_feature_gate_error_matches_disabled_reason() {
        let from_error = render_mcp_policy_error("FeatureGateDisabled");
        let from_reason = render_gate_disabled_reason();

        assert_eq!(
            from_error, from_reason,
            "FeatureGateDisabled rendering must match render_gate_disabled_reason"
        );
    }
}
