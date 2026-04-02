use crate::contracts::{RemoteSessionError, RemoteSessionState};

fn expected_transition_allowed(from: RemoteSessionState, to: RemoteSessionState) -> bool {
    match from {
        RemoteSessionState::Pending => matches!(
            to,
            RemoteSessionState::Running
                | RemoteSessionState::Idle
                | RemoteSessionState::RequiresAction
                | RemoteSessionState::Failed
        ),
        RemoteSessionState::Running => matches!(
            to,
            RemoteSessionState::Idle
                | RemoteSessionState::RequiresAction
                | RemoteSessionState::Archived
                | RemoteSessionState::Failed
                | RemoteSessionState::Expired
        ),
        RemoteSessionState::Idle => matches!(
            to,
            RemoteSessionState::Running
                | RemoteSessionState::Archived
                | RemoteSessionState::Expired
        ),
        RemoteSessionState::RequiresAction => matches!(
            to,
            RemoteSessionState::Running
                | RemoteSessionState::Archived
                | RemoteSessionState::Expired
        ),
        RemoteSessionState::Archived | RemoteSessionState::Expired | RemoteSessionState::Failed => {
            false
        }
    }
}

#[test]
fn allows_required_transitions() {
    let allowed = [
        (RemoteSessionState::Pending, RemoteSessionState::Running),
        (RemoteSessionState::Pending, RemoteSessionState::Idle),
        (
            RemoteSessionState::Pending,
            RemoteSessionState::RequiresAction,
        ),
        (RemoteSessionState::Pending, RemoteSessionState::Failed),
        (RemoteSessionState::Running, RemoteSessionState::Idle),
        (
            RemoteSessionState::Running,
            RemoteSessionState::RequiresAction,
        ),
        (RemoteSessionState::Running, RemoteSessionState::Archived),
        (RemoteSessionState::Running, RemoteSessionState::Failed),
        (RemoteSessionState::Running, RemoteSessionState::Expired),
        (RemoteSessionState::Idle, RemoteSessionState::Running),
        (RemoteSessionState::Idle, RemoteSessionState::Archived),
        (RemoteSessionState::Idle, RemoteSessionState::Expired),
        (
            RemoteSessionState::RequiresAction,
            RemoteSessionState::Running,
        ),
        (
            RemoteSessionState::RequiresAction,
            RemoteSessionState::Archived,
        ),
        (
            RemoteSessionState::RequiresAction,
            RemoteSessionState::Expired,
        ),
    ];

    for (from, to) in allowed {
        assert!(
            from.can_transition_to(to),
            "{from:?} -> {to:?} should be allowed"
        );
    }
}

#[test]
fn forbids_required_transitions() {
    assert!(!RemoteSessionState::Archived.can_transition_to(RemoteSessionState::Running));
    assert!(!RemoteSessionState::Expired.can_transition_to(RemoteSessionState::Running));
}

#[test]
fn terminal_states_have_no_outgoing_transitions() {
    let states = [
        RemoteSessionState::Pending,
        RemoteSessionState::Running,
        RemoteSessionState::Idle,
        RemoteSessionState::RequiresAction,
        RemoteSessionState::Archived,
        RemoteSessionState::Expired,
        RemoteSessionState::Failed,
    ];

    for state in [
        RemoteSessionState::Archived,
        RemoteSessionState::Expired,
        RemoteSessionState::Failed,
    ] {
        for target in states {
            assert!(
                !state.can_transition_to(target),
                "{state:?} -> {target:?} should be forbidden"
            );
        }
    }
}

#[test]
fn enforces_full_transition_matrix() {
    let states = [
        RemoteSessionState::Pending,
        RemoteSessionState::Running,
        RemoteSessionState::Idle,
        RemoteSessionState::RequiresAction,
        RemoteSessionState::Archived,
        RemoteSessionState::Expired,
        RemoteSessionState::Failed,
    ];

    for from in states {
        for to in states {
            assert_eq!(
                from.can_transition_to(to),
                expected_transition_allowed(from, to),
                "unexpected transition check for {from:?} -> {to:?}"
            );
        }
    }
}

#[test]
fn transition_to_returns_typed_error_for_forbidden_transition() {
    let result = RemoteSessionState::Expired.transition_to(RemoteSessionState::Running);

    assert_eq!(result, Err(RemoteSessionError::InvalidStateTransition));
}
