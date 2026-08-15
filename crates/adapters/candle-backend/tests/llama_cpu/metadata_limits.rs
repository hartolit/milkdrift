use super::support::*;

#[test]
fn excessive_shard_count_is_rejected_before_files_are_opened() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let source = fixture.source_with_paths(
        ConfigDeclaration::F32,
        vec![fixture.weight_path.clone(); 257],
    )?;
    fs::remove_file(&fixture.weight_path).map_err(|error| error.to_string())?;
    let loader = CandleLlamaLoader::new(BACKEND);

    assert!(matches!(
        loader.inspect(&source),
        Err(LoadError::CapacityExhausted(capacity))
            if capacity.resource == CapacityResource::BackendScratch
                && capacity.required == 257
                && capacity.available == 256
    ));
    Ok(())
}

#[test]
fn malformed_truncated_and_eight_mib_plus_one_headers_are_rejected() -> TestResult {
    assert_raw_header_rejected(10_u64.to_le_bytes(), b"{}", &[])?;
    assert_raw_header_rejected((PER_SHARD_HEADER_LIMIT + 1).to_le_bytes(), &[], &[])?;
    assert_raw_header_rejected(8_u64.to_le_bytes(), b"not-json", &[])?;
    assert_raw_bytes_rejected(b"short")?;
    Ok(())
}

#[test]
fn overlap_explicit_gap_bounds_shape_mismatch_and_overflow_are_rejected() -> TestResult {
    for (header, payload) in [
        (
            r#"{"a":{"dtype":"F32","shape":[1],"data_offsets":[0,4]},"b":{"dtype":"F32","shape":[1],"data_offsets":[3,7]}}"#.to_owned(),
            vec![0_u8; 7],
        ),
        (
            r#"{"a":{"dtype":"F32","shape":[1],"data_offsets":[0,4]},"b":{"dtype":"F32","shape":[1],"data_offsets":[5,9]}}"#.to_owned(),
            vec![0_u8; 9],
        ),
        (
            r#"{"a":{"dtype":"F32","shape":[2],"data_offsets":[0,4]}}"#.to_owned(),
            vec![0_u8; 4],
        ),
        (
            r#"{"a":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#.to_owned(),
            vec![0_u8; 3],
        ),
        (
            r#"{"a":{"dtype":"F32","shape":[18446744073709551615,2],"data_offsets":[0,0]}}"#.to_owned(),
            Vec::new(),
        ),
    ] {
        assert_valid_prefix_header_rejected(header.as_str(), payload.as_slice())?;
    }
    Ok(())
}

#[test]
fn tensor_name_rank_and_metadata_limits_are_rejected() -> TestResult {
    let long_name = "n".repeat(513);
    let name_header =
        format!(r#"{{"{long_name}":{{"dtype":"F32","shape":[0],"data_offsets":[0,0]}}}}"#);
    assert_valid_prefix_header_rejected(name_header.as_str(), &[])?;

    let rank_header =
        r#"{"rank":{"dtype":"F32","shape":[1,1,1,1,1,1,1,1,1],"data_offsets":[0,4]}}"#;
    assert_valid_prefix_header_rejected(rank_header, &[0_u8; 4])?;

    let metadata_header = serde_json::to_string(&json!({
        "__metadata__": {"key": "v".repeat(4 * 1024 + 1)}
    }))
    .map_err(|error| error.to_string())?;
    assert_valid_prefix_header_rejected(metadata_header.as_str(), &[])?;
    Ok(())
}
