use super::*;

pub(crate) fn assert_valid_prefix_header_rejected(header: &str, payload: &[u8]) -> TestResult {
    let fixture = TinyLlamaFixture::empty()?;
    write_raw_safetensors(&fixture.weight_path, header, payload)?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert_invalid_model(loader.inspect(&source));
    Ok(())
}

pub(crate) fn assert_raw_header_rejected(
    prefix: [u8; 8],
    header: &[u8],
    payload: &[u8],
) -> TestResult {
    let fixture = TinyLlamaFixture::empty()?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&prefix);
    bytes.extend_from_slice(header);
    bytes.extend_from_slice(payload);
    fs::write(&fixture.weight_path, bytes).map_err(|error| error.to_string())?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert_invalid_model(loader.inspect(&source));
    Ok(())
}

pub(crate) fn assert_raw_bytes_rejected(bytes: &[u8]) -> TestResult {
    let fixture = TinyLlamaFixture::empty()?;
    fs::write(&fixture.weight_path, bytes).map_err(|error| error.to_string())?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert_invalid_model(loader.inspect(&source));
    Ok(())
}

pub(crate) fn assert_invalid_model<T>(result: Result<T, LoadError>) {
    let matches_kind = matches!(
        &result,
        Err(LoadError::Backend(failure))
            if failure.failure.kind == BackendFailureKind::InvalidModel
    );
    drop(result);
    assert!(matches_kind);
}

pub(crate) fn assert_unsupported<T>(result: Result<T, LoadError>) {
    let matches_kind = matches!(
        &result,
        Err(LoadError::Backend(failure))
            if failure.failure.kind == BackendFailureKind::Unsupported
    );
    drop(result);
    assert!(matches_kind);
}

pub(crate) fn assert_exact_tensor_load_failure<T>(
    result: Result<T, LoadError>,
    expected_kind: BackendFailureKind,
    expected_code: u32,
    expected_stage: LoadFailureStage,
    expected_location: TensorFailureLocation,
) -> TestResult {
    let error = result
        .err()
        .ok_or_else(|| "tensor failure case unexpectedly succeeded".to_owned())?;
    let LoadError::Backend(failure) = error else {
        return Err(format!("unexpected tensor failure: {error:?}"));
    };
    assert_eq!(failure.failure.kind, expected_kind);
    assert_eq!(failure.failure.code, expected_code);
    let context = failure
        .context
        .ok_or_else(|| "tensor failure lost bounded context".to_owned())?;
    assert_eq!(context.stage, expected_stage);
    assert_eq!(context.tensor, Some(expected_location));
    Ok(())
}

pub(crate) fn maximum_logit_token(logits: &[f32]) -> TestResult<TokenId> {
    let (index, _) = logits
        .iter()
        .copied()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .ok_or_else(|| "logits must not be empty".to_owned())?;
    let token = u32::try_from(index).map_err(|error| error.to_string())?;
    Ok(TokenId::new(token))
}

pub(crate) fn debug_error(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}
