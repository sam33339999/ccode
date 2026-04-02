#![allow(dead_code)]
// Bridge mode CLI rendering helpers and behaviour tests (US-037 §8.3).
//
// Pure rendering functions — no I/O, no async — so they are fully testable
// without any mock infrastructure.
//
// The rendering helpers are not yet wired to a CLI subcommand; the
// dead_code lint is suppressed until the bridge command is plumbed in.

/// Returns the actionable message shown when the bridge-mode gate is disabled.
pub fn render_bridge_disabled_message() -> String {
    "Remote control is not available: bridge mode is disabled by policy. \
     To enable it, set the bridge feature gate in your configuration and restart."
        .to_string()
}

/// Formats the session URL shown at the start of a remote-control session.
pub fn render_session_url(base_url: &str, session_id: &str) -> String {
    format!("{}/sessions/{}", base_url.trim_end_matches('/'), session_id)
}

/// Formats a lifecycle status update line for an active remote session.
pub fn render_lifecycle_update(session_id: &str, state: &str) -> String {
    format!("[remote-session:{session_id}] state → {state}")
}

/// Formats the non-fatal warning emitted when an archive call fails on exit.
///
/// The message is intentionally non-fatal: the CLI must still exit cleanly
/// (exit code 0) after printing it.
pub fn render_archive_warning(session_id: &str, reason: &str) -> String {
    format!(
        "[warning] archive did not complete for {session_id} ({reason}); \
         the session may be cleaned up automatically by the remote service."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── §8.3 CLI behaviour tests ───────────────────────────────────────────

    /// AC: disabled gate path prints actionable message.
    ///
    /// When bridge mode is disabled the rendered string must tell the user
    /// what is wrong and what to do about it — not a bare error code.
    #[test]
    fn cli_disabled_gate_path_prints_actionable_message() {
        let msg = render_bridge_disabled_message();

        assert!(
            msg.contains("disabled by policy"),
            "message must name the reason; got: {msg}"
        );
        assert!(
            msg.contains("configuration") || msg.contains("feature gate"),
            "message must tell the user how to fix it; got: {msg}"
        );
    }

    /// AC: successful remote-control path yields session URL and lifecycle updates.
    ///
    /// The rendered session URL must contain the session ID so the user can
    /// navigate directly to the session. Lifecycle lines must identify the
    /// session and the new state.
    #[test]
    fn cli_successful_path_yields_session_url_and_lifecycle_updates() {
        let base = "https://claude.ai/code";
        let session_id = "session_abc123";

        let url = render_session_url(base, session_id);
        assert!(
            url.contains(session_id),
            "session URL must contain the session ID; got: {url}"
        );
        assert!(
            url.starts_with(base),
            "session URL must start with the configured base; got: {url}"
        );

        let running = render_lifecycle_update(session_id, "Running");
        assert!(
            running.contains(session_id),
            "lifecycle line must identify the session; got: {running}"
        );
        assert!(
            running.contains("Running"),
            "lifecycle line must include the new state; got: {running}"
        );

        let archived = render_lifecycle_update(session_id, "Archived");
        assert!(
            archived.contains("Archived"),
            "lifecycle line must reflect state transitions; got: {archived}"
        );
    }

    /// AC: archive failure surfaces non-fatal warning, exit remains graceful.
    ///
    /// The warning string must NOT contain "fatal" — callers use this to
    /// verify they should not propagate the failure as a hard error.
    #[test]
    fn cli_archive_failure_surfaces_non_fatal_warning() {
        let session_id = "session_xyz";
        let reason = "timeout after 5s";

        let warning = render_archive_warning(session_id, reason);

        assert!(
            warning.contains(session_id),
            "warning must identify the session; got: {warning}"
        );
        assert!(
            warning.to_ascii_lowercase().contains("warning"),
            "message must be labelled as a warning, not an error; got: {warning}"
        );
        assert!(
            !warning.to_ascii_lowercase().contains("fatal"),
            "warning must not claim the failure is fatal; got: {warning}"
        );
        assert!(
            warning.contains(reason),
            "warning should include the reason so the user can diagnose it; got: {warning}"
        );
    }

    // ── rendering invariants ───────────────────────────────────────────────

    #[test]
    fn session_url_trims_trailing_slash_from_base() {
        let url = render_session_url("https://claude.ai/code/", "session_1");
        assert_eq!(url, "https://claude.ai/code/sessions/session_1");
    }

    #[test]
    fn lifecycle_update_format_is_stable() {
        let line = render_lifecycle_update("session_1", "Idle");
        assert_eq!(line, "[remote-session:session_1] state → Idle");
    }

    #[test]
    fn archive_warning_format_is_stable() {
        let line = render_archive_warning("session_1", "upstream error");
        assert!(line.starts_with("[warning]"));
        assert!(line.contains("session_1"));
        assert!(line.contains("upstream error"));
    }
}
