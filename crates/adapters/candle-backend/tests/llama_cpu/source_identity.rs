use super::support::*;

#[test]
fn owned_config_bytes_remain_bound_while_local_paths_stay_late_bound() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let local_source = fixture.source(ConfigDeclaration::F32)?;
    let config_bytes = fs::read(&fixture.config_path).map_err(|error| error.to_string())?;
    let bound_source = CandleLlamaSource::from_config_bytes(
        config_bytes,
        vec![CandleWeightShard::unverified_local(
            fixture.weight_path.clone(),
        )],
    )
    .map_err(|error| error.to_string())?;

    write_tiny_config(&fixture.config_path, ConfigDeclaration::Unsupported)?;
    let loader = CandleLlamaLoader::new(BACKEND);
    let descriptor = loader.inspect(&bound_source).map_err(debug_error)?;
    assert_eq!(
        descriptor.metadata.configuration_declared_scalar_type,
        Some(ScalarType::F32)
    );
    assert_unsupported(loader.inspect(&local_source));
    Ok(())
}

#[test]
fn unverified_fallback_detects_same_inode_payload_mutation() -> TestResult {
    assert_unverified_prepared_mutation_rejected(PreparedMutation::Payload)
}

#[test]
fn unverified_fallback_detects_same_length_header_mutation() -> TestResult {
    assert_unverified_prepared_mutation_rejected(PreparedMutation::SameLengthHeader)
}

#[test]
fn unverified_fallback_detects_truncation_and_extension() -> TestResult {
    for mutation in [PreparedMutation::Truncate, PreparedMutation::Extend] {
        assert_unverified_prepared_mutation_rejected(mutation)?;
    }
    Ok(())
}

#[test]
fn supplied_expected_content_succeeds_and_digest_mismatch_rejects() -> TestResult {
    let matching = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let (byte_length, sha256) = file_identity(&matching.weight_path)?;
    let source = matching.source_with_shards(
        ConfigDeclaration::F32,
        vec![CandleWeightShard::with_expected_content(
            matching.weight_path.clone(),
            CandleExpectedContentIdentity::new(byte_length, sha256),
        )],
    )?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let (_plan, mut model) = prepare_and_load(&mut loader, &source, load_configuration())?;
    clean_model(&mut model)?;

    let mismatched = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let (byte_length, mut sha256) = file_identity(&mismatched.weight_path)?;
    sha256[0] ^= 1;
    let source = mismatched.source_with_shards(
        ConfigDeclaration::F32,
        vec![CandleWeightShard::with_expected_content(
            mismatched.weight_path.clone(),
            CandleExpectedContentIdentity::new(byte_length, sha256),
        )],
    )?;
    let prepared = loader
        .prepare_load(&source, &load_configuration())
        .map_err(debug_error)?;
    assert_failed_preparation_invalid_model_code(
        &mut loader,
        prepared,
        SOURCE_IDENTITY_MISMATCH_CODE,
    )
}

#[test]
fn supplied_expected_content_length_mismatch_rejects_explicitly() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let (byte_length, sha256) = file_identity(&fixture.weight_path)?;
    let source = fixture.source_with_shards(
        ConfigDeclaration::F32,
        vec![CandleWeightShard::with_expected_content(
            fixture.weight_path.clone(),
            CandleExpectedContentIdentity::new(
                byte_length
                    .checked_add(1)
                    .ok_or_else(|| "fixture length overflow".to_owned())?,
                sha256,
            ),
        )],
    )?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let error = loader
        .prepare_load(&source, &load_configuration())
        .err()
        .ok_or_else(|| "wrong expected content length was admitted".to_owned())?;
    assert!(matches!(
        error,
        LoadError::Backend(failure)
            if failure.failure.kind == BackendFailureKind::InvalidModel
                && failure.failure.code == SOURCE_IDENTITY_LENGTH_CODE
    ));
    Ok(())
}

#[test]
fn supplied_expected_content_detects_same_inode_mutation_before_publication() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let (byte_length, sha256) = file_identity(&fixture.weight_path)?;
    let source = fixture.source_with_shards(
        ConfigDeclaration::F32,
        vec![CandleWeightShard::with_expected_content(
            fixture.weight_path.clone(),
            CandleExpectedContentIdentity::new(byte_length, sha256),
        )],
    )?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let prepared = loader
        .prepare_load(&source, &load_configuration())
        .map_err(debug_error)?;
    mutate_prepared_file(&fixture.weight_path, PreparedMutation::Payload)?;
    assert_failed_preparation_invalid_model(&mut loader, prepared)
}

#[test]
fn same_header_same_path_and_distinct_cross_shard_duplicates_are_rejected() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let duplicate_path_source = fixture.source_with_paths(
        ConfigDeclaration::F32,
        vec![fixture.weight_path.clone(), fixture.weight_path.clone()],
    )?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert_invalid_model(loader.inspect(&duplicate_path_source));

    let same_header = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    write_raw_safetensors(
        &same_header.weight_path,
        r#"{"dup":{"dtype":"F32","shape":[0],"data_offsets":[0,0]},"dup":{"dtype":"F32","shape":[0],"data_offsets":[0,0]}}"#,
        &[],
    )?;
    let same_header_source = same_header.source(ConfigDeclaration::F32)?;
    assert_invalid_model(loader.inspect(&same_header_source));

    let cross_shard = TinyLlamaFixture::create_sharded(RequiredProfile::F32, true)?;
    let cross_shard_source = cross_shard.source_with_paths(
        ConfigDeclaration::F32,
        vec![
            cross_shard.weight_path.clone(),
            cross_shard.second_weight_path.clone(),
        ],
    )?;
    assert_invalid_model(loader.inspect(&cross_shard_source));
    Ok(())
}
