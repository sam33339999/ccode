use crate::ccr_client::{
    TransportErrorKind, classify_status_error, map_transport_error,
    normalize_session_id_for_response, normalize_session_id_for_transport,
};
use crate::contracts::CcrClientError;

#[test]
fn prefix_translation_round_trips_session_id() {
    let local = "session_abc123";
    let transport = normalize_session_id_for_transport(local);
    let back = normalize_session_id_for_response(&transport);

    assert_eq!(transport, "cse_abc123");
    assert_eq!(back, local);
}

#[test]
fn prefix_translation_round_trips_cse_id() {
    let transport = "cse_abc123";
    let local = normalize_session_id_for_response(transport);
    let back = normalize_session_id_for_transport(&local);

    assert_eq!(local, "session_abc123");
    assert_eq!(back, transport);
}

#[test]
fn classifies_http_error_variant() {
    let err = classify_status_error(500, "server exploded".to_string());
    match err {
        CcrClientError::Http(message) => {
            assert!(message.contains("status 500"));
            assert!(!message.contains("server exploded"));
        }
        other => panic!("expected http error, got {other:?}"),
    }
}

#[test]
fn classifies_timeout_error_variant() {
    let err = map_transport_error(TransportErrorKind::Timeout, "deadline".to_string());
    assert!(matches!(err, CcrClientError::Timeout));
}

#[test]
fn classifies_unauthorized_error_variant() {
    let err = classify_status_error(401, "unauthorized".to_string());
    assert!(matches!(err, CcrClientError::Unauthorized));
}

#[test]
fn classifies_forbidden_error_variant() {
    let err = classify_status_error(403, "forbidden".to_string());
    assert!(matches!(err, CcrClientError::Forbidden));
}

#[test]
fn status_error_mapping_does_not_leak_internal_body_for_client_errors() {
    let err = classify_status_error(400, "trace_id=abc internal stack details".to_string());
    match err {
        CcrClientError::Http(message) => {
            assert!(message.contains("status 400"));
            assert!(!message.contains("trace_id=abc"));
            assert!(!message.contains("stack details"));
        }
        other => panic!("expected http error, got {other:?}"),
    }
}

#[test]
fn classifies_invalid_payload_error_variant() {
    let err = map_transport_error(TransportErrorKind::InvalidPayload, "bad json".to_string());
    assert!(matches!(err, CcrClientError::InvalidPayload));
}
