//! Naming is a closed public command; semantic name validation remains with persistence.
use milkdrift_control_protocol::{
    DaemonMetadataDto, ErrorCode, RenameDaemonReceipt, RenameDaemonRequest, decode_json,
    encode_json,
};

#[test]
fn rename_documents_reject_duplicate_and_unknown_fields() {
    for invalid in [
        r#"{"request_id":"one","expected_generation":1,"name":"Studio","actor":"forged"}"#,
        r#"{"request_id":"one","expected_generation":1,"name":"Studio","name":"Other"}"#,
        r#"{"request_id":"one","name":"Studio"}"#,
    ] {
        assert!(decode_json::<RenameDaemonRequest>(invalid.as_bytes()).is_err());
    }
}

#[test]
fn naming_documents_preserve_exact_request_bytes_and_stable_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let request = RenameDaemonRequest {
        request_id: "rename-one".into(),
        expected_generation: 7,
        name: "  Studio Δ  ".into(),
    };
    assert_eq!(
        String::from_utf8(encode_json(&request)?)?,
        r#"{"expected_generation":7,"name":"  Studio Δ  ","request_id":"rename-one"}"#,
    );
    assert_eq!(
        decode_json::<RenameDaemonRequest>(&encode_json(&request)?)?,
        request
    );
    let metadata = DaemonMetadataDto {
        daemon_id: "host:stable".into(),
        name: "Studio Δ".into(),
        generation: 8,
        can_rename: true,
    };
    assert_eq!(
        decode_json::<DaemonMetadataDto>(&encode_json(&metadata)?)?,
        metadata
    );
    let receipt = RenameDaemonReceipt {
        request_id: request.request_id,
        daemon_id: metadata.daemon_id,
        name: metadata.name,
        generation: metadata.generation,
    };
    assert_eq!(
        decode_json::<RenameDaemonReceipt>(&encode_json(&receipt)?)?,
        receipt
    );
    assert_eq!(ErrorCode::DaemonIdentityMismatch.http_status_code(), 409);
    assert_eq!(
        serde_json::to_string(&ErrorCode::DaemonIdentityMismatch)?,
        "\"daemon_identity_mismatch\""
    );
    Ok(())
}
