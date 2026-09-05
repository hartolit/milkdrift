use super::artifacts::parse_range;
use super::*;
use axum::http::HeaderValue;

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
