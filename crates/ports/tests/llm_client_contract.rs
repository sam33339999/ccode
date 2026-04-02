use ccode_ports::provider::LlmError;
use std::time::Duration;

#[test]
fn llm_error_variants_match_contract_shapes() {
    let _ = LlmError::AuthError("bad key".to_string());
    let _ = LlmError::RateLimited {
        retry_after_ms: Some(1500),
    };
    let _ = LlmError::ModelNotAvailable("unknown".to_string());
    let _ = LlmError::RequestTooLarge("too many tokens".to_string());
    let _ = LlmError::InvalidResponse("missing field".to_string());
    let _ = LlmError::StreamInterrupted("socket closed".to_string());
    let _ = LlmError::Network("dns error".to_string());
    let _ = LlmError::Timeout(Duration::from_secs(10));
    let _ = LlmError::ProviderError {
        status: 502,
        message: "upstream bad gateway".to_string(),
    };
}
