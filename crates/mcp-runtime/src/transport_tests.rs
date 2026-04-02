use serde_json::json;

use crate::transport::{
    ConnectionFailure, StdioTransport, TransportError, decode_jsonrpc_line, encode_jsonrpc_line,
};

#[test]
fn encode_jsonrpc_line_appends_newline() {
    let msg = json!({"jsonrpc": "2.0", "id": 1_u64, "method": "initialize"});

    let encoded = encode_jsonrpc_line(&msg).expect("encoding should succeed");

    assert_eq!(
        String::from_utf8(encoded).expect("encoded json is utf8"),
        "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"initialize\"}\n"
    );
}

#[test]
fn decode_jsonrpc_line_parses_message() {
    let decoded = decode_jsonrpc_line("{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}")
        .expect("decode should succeed");

    assert_eq!(decoded["jsonrpc"], "2.0");
    assert_eq!(decoded["id"], 1);
}

#[test]
fn decode_jsonrpc_line_rejects_empty_line() {
    let err = decode_jsonrpc_line("  ").expect_err("empty line should fail");

    assert!(matches!(err, TransportError::Protocol(_)));
}

#[test]
fn decode_jsonrpc_line_rejects_invalid_json() {
    let err = decode_jsonrpc_line("not-json").expect_err("invalid json should fail");

    assert!(matches!(err, TransportError::Deserialize(_)));
}

#[tokio::test]
async fn spawn_nonexistent_binary_is_connection_failure() {
    let err = match StdioTransport::spawn("/definitely/missing/mcp-binary", &[]).await {
        Ok(_) => panic!("spawn should fail"),
        Err(err) => err,
    };

    assert!(matches!(
        err,
        TransportError::Connection(ConnectionFailure::Spawn { .. })
    ));
}
