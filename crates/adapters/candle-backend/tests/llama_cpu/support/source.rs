use super::*;

pub(crate) const fn load_configuration() -> LoadConfiguration {
    LoadConfiguration {
        handle: ModelHandle::new(ModelId::new(9), ModelGeneration::new(1)),
        execution_device: ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu),
        memory_budget: MemoryBudget::ZERO.with_host_bytes(ByteCount::MAX),
    }
}

pub(crate) fn assert_failed_preparation_invalid_model(
    loader: &mut CandleLlamaLoader,
    prepared: CandleLlamaPreparedLoad,
) -> TestResult {
    let failed = match loader.load_prepared(prepared) {
        Err(failed) => failed,
        Ok(mut model) => {
            clean_model(&mut model)?;
            return Err("mutated or mismatched preparation unexpectedly loaded".to_owned());
        }
    };
    assert!(matches!(
        failed.primary(),
        LoadError::Backend(failure) if failure.failure.kind == BackendFailureKind::InvalidModel
    ));
    let mut failed = failed;
    failed.cleanup().map_err(debug_error)?;
    failed.cleanup().map_err(debug_error)
}

pub(crate) fn assert_failed_preparation_invalid_model_code(
    loader: &mut CandleLlamaLoader,
    prepared: CandleLlamaPreparedLoad,
    expected_code: u32,
) -> TestResult {
    let failed = match loader.load_prepared(prepared) {
        Err(failed) => failed,
        Ok(mut model) => {
            clean_model(&mut model)?;
            return Err("mismatched expectation unexpectedly loaded".to_owned());
        }
    };
    assert!(matches!(
        failed.primary(),
        LoadError::Backend(failure)
            if failure.failure.kind == BackendFailureKind::InvalidModel
                && failure.failure.code == expected_code
    ));
    let mut failed = failed;
    failed.cleanup().map_err(debug_error)?;
    failed.cleanup().map_err(debug_error)
}

pub(crate) fn assert_unverified_prepared_mutation_rejected(
    mutation: PreparedMutation,
) -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    assert!(
        source
            .weight_shards()
            .iter()
            .all(|shard| shard.expected_content().is_none())
    );
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let prepared = loader
        .prepare_load(&source, &load_configuration())
        .map_err(debug_error)?;
    mutate_prepared_file(&fixture.weight_path, mutation)?;
    assert_failed_preparation_invalid_model(&mut loader, prepared)
}

pub(crate) fn mutate_prepared_file(path: &Path, mutation: PreparedMutation) -> TestResult {
    match mutation {
        PreparedMutation::Payload => {
            let mut file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|error| format!("open payload for mutation: {error}"))?;
            file.seek(SeekFrom::End(-1))
                .map_err(|error| format!("seek payload: {error}"))?;
            let mut final_byte = [0_u8; 1];
            file.read_exact(&mut final_byte)
                .map_err(|error| format!("read payload: {error}"))?;
            final_byte[0] ^= 1;
            file.seek(SeekFrom::End(-1))
                .map_err(|error| format!("reseek payload: {error}"))?;
            file.write_all(&final_byte)
                .map_err(|error| format!("write payload mutation: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("sync payload mutation: {error}"))
        }
        PreparedMutation::SameLengthHeader => {
            let bytes = fs::read(path).map_err(|error| error.to_string())?;
            let header_length = read_header_length(bytes.as_slice())?;
            let header = bytes
                .get(8..8 + header_length)
                .ok_or_else(|| "fixture header is truncated".to_owned())?;
            let offset = header
                .iter()
                .rposition(|byte| *byte == b' ')
                .ok_or_else(|| "fixture header has no padding byte to mutate".to_owned())?;
            let absolute = 8_u64
                .checked_add(u64::try_from(offset).map_err(|error| error.to_string())?)
                .ok_or_else(|| "header mutation offset overflow".to_owned())?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(|error| format!("open header for mutation: {error}"))?;
            file.seek(SeekFrom::Start(absolute))
                .map_err(|error| format!("seek header: {error}"))?;
            file.write_all(b"\n")
                .map_err(|error| format!("write header mutation: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("sync header mutation: {error}"))
        }
        PreparedMutation::Truncate => {
            let file = fs::OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(|error| format!("open file for truncation: {error}"))?;
            let length = file
                .metadata()
                .map_err(|error| format!("read length for truncation: {error}"))?
                .len();
            file.set_len(
                length
                    .checked_sub(1)
                    .ok_or_else(|| "cannot truncate empty fixture".to_owned())?,
            )
            .map_err(|error| format!("truncate fixture: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("sync truncation: {error}"))
        }
        PreparedMutation::Extend => {
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(path)
                .map_err(|error| format!("open file for extension: {error}"))?;
            file.write_all(&[0])
                .map_err(|error| format!("extend fixture: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("sync extension: {error}"))
        }
    }
}
