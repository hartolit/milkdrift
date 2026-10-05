use super::artifacts::parse_range;
use super::*;
use axum::http::HeaderValue;
use axum::response::IntoResponse as _;

#[tokio::test]
async fn every_public_error_has_the_same_server_client_and_protocol_status()
-> Result<(), Box<dyn std::error::Error>> {
    use milkdrift_control_client::{ClientError, status_class};
    use milkdrift_control_protocol::{ErrorCode, ErrorEnvelope};
    for (code, expected) in [
        (ErrorCode::Unauthenticated, 401),
        (ErrorCode::Unauthorized, 403),
        (ErrorCode::InvalidInput, 400),
        (ErrorCode::Conflict, 409),
        (ErrorCode::NotFound, 404),
        (ErrorCode::Overload, 429),
        (ErrorCode::Unavailable, 503),
        (ErrorCode::Corruption, 500),
        (ErrorCode::Uncertain, 409),
        (ErrorCode::UnsupportedVersion, 426),
        (ErrorCode::Timeout, 504),
        (ErrorCode::Internal, 500),
    ] {
        for retryable in [false, true] {
            let failure = crate::host::PublicFailure {
                code,
                message: "retained operation evidence".into(),
                retryable,
                details: std::collections::BTreeMap::from([(
                    "cause_code".into(),
                    "fixture".into(),
                )]),
            };
            let response =
                super::response::owner_error(failure, "error-table".into()).into_response();
            assert_eq!(response.status().as_u16(), expected);
            assert_eq!(code.http_status_code(), expected);
            let bytes = axum::body::to_bytes(
                response.into_body(),
                milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
            )
            .await?;
            let error: ErrorEnvelope = milkdrift_control_protocol::decode_json(&bytes)?;
            assert_eq!(error.code, code);
            assert_eq!(error.message, "retained operation evidence");
            assert_eq!(error.request_id.as_deref(), Some("error-table"));
            assert_eq!(
                error.details.get("cause_code").map(String::as_str),
                Some("fixture")
            );
            let client = ClientError::Api(error);
            assert_eq!(
                status_class(&client).map(|status| status.as_u16()),
                Some(expected)
            );
            assert_eq!(client.retryable(), retryable);
        }
    }
    for local in [
        ClientError::Timeout,
        ClientError::Transport("fixture".into()),
        ClientError::Protocol(milkdrift_control_protocol::ProtocolError::InvalidJson(
            "fixture".into(),
        )),
        ClientError::Stream("fixture".into()),
        ClientError::Configuration("fixture".into()),
    ] {
        assert!(status_class(&local).is_none());
    }
    Ok(())
}

#[test]
fn artifact_ranges_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
    let header = HeaderValue::from_static("bytes=100-9999999");
    let (start, maximum) =
        parse_range(Some(&header), "request").map_err(|error| error.envelope.message)?;
    assert_eq!(start, 100);
    assert_eq!(maximum, MAX_ARTIFACT_HTTP_RANGE);
    Ok(())
}

#[test]
fn every_local_external_route_declares_typed_authority_and_resource_mapping() {
    let source = include_str!("../http.rs");
    let raw_route_marker = [".", "route("].concat();
    assert_eq!(
        source.match_indices(&raw_route_marker).count(),
        1,
        "add external routes through authorized_routes! with a typed authority mapping"
    );
}
