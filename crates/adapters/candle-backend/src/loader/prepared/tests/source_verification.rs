use super::support::*;

#[test]
fn ignored_ranges_are_hashed_without_materialization_or_transfer() -> Result<(), String> {
    let header = br#"{"ignored":{"dtype":"U8","shape":[3],"data_offsets":[0,3]},"required":{"dtype":"F32","shape":[1],"data_offsets":[3,7]}}"#;
    let payload = [9_u8, 8, 7, 0, 0, 128, 63];

    for (device_kind, expected_transfer_count) in [(DeviceKind::Cpu, 0), (DeviceKind::Cuda, 1)] {
        let tensors = vec![
            inspected_tensor("ignored", SourceTensorDType::U8, &[3], 0, 3, false)?,
            inspected_tensor("required", SourceTensorDType::F32, &[1], 3, 4, true)?,
        ];
        let shard = inspected_shard(header, &payload, tensors)?;
        let mut prepared = test_prepared(vec![shard], DType::F32)?;
        configure_test_device(&mut prepared, device_kind)?;
        let mut events = Events::default();
        prepared
            .materialize_shard(0, &mut events)
            .map_err(|error| format!("materialize shard: {error:?}"))?;

        assert_eq!(events.prefix_header_bytes, 8 + header.len());
        assert_eq!(events.ignored_bytes, 3);
        assert_eq!(events.required_bytes, 4);
        assert_eq!(events.source_owned_count, 1);
        assert_eq!(events.cast_owned_count, 0);
        assert_eq!(events.transfer_owned_count, expected_transfer_count);
        assert_eq!(events.map_owned_count, 1);
        assert_eq!(events.batch_synchronizations, expected_transfer_count);
        assert_eq!(
            events.verified_establishments.as_slice(),
            &[ContentIdentityEstablishment::SuppliedExpectation]
        );
        assert!(prepared.final_tensors.contains_key("required"));
        assert!(!prepared.final_tensors.contains_key("ignored"));
    }
    Ok(())
}

#[test]
fn header_payload_and_truncation_mutations_fail_from_retained_files() -> Result<(), String> {
    let header = br#"{"required":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#;
    let payload = [0_u8, 0, 128, 63];

    let header_tensor = inspected_tensor("required", SourceTensorDType::F32, &[1], 0, 4, true)?;
    let header_shard = inspected_shard(header, &payload, vec![header_tensor])?;
    let mut header_prepared = test_prepared(vec![header_shard], DType::F32)?;
    {
        let shard = first_shard_mut(&mut header_prepared)?;
        shard
            .file
            .seek(SeekFrom::Start(8))
            .map_err(|error| error.to_string())?;
        shard
            .file
            .write_all(b"[")
            .map_err(|error| error.to_string())?;
    }
    let error = required_error(
        header_prepared.materialize_shard(0, &mut Events::default()),
        "header mutation must fail before payload processing",
    )?;
    assert_eq!(failure_code(error), Some(CODE_HEADER_IDENTITY_MISMATCH));
    assert!(header_prepared.final_tensors.is_empty());

    let payload_tensor = inspected_tensor("required", SourceTensorDType::F32, &[1], 0, 4, true)?;
    let payload_shard = inspected_shard(header, &payload, vec![payload_tensor])?;
    let mut payload_prepared = test_prepared(vec![payload_shard], DType::F32)?;
    let last_byte = first_shard_mut(&mut payload_prepared)?
        .file_length
        .checked_sub(1)
        .ok_or_else(|| "missing payload byte".to_owned())?;
    {
        let shard = first_shard_mut(&mut payload_prepared)?;
        shard
            .file
            .seek(SeekFrom::Start(last_byte))
            .map_err(|error| error.to_string())?;
        shard
            .file
            .write_all(&[0_u8])
            .map_err(|error| error.to_string())?;
    }
    let error = required_error(
        payload_prepared.materialize_shard(0, &mut Events::default()),
        "payload mutation must fail at whole-shard verification",
    )?;
    assert_eq!(failure_code(error), Some(CODE_SOURCE_IDENTITY_MISMATCH));
    assert!(payload_prepared.final_tensors.contains_key("required"));

    let truncated_tensor = inspected_tensor("required", SourceTensorDType::F32, &[1], 0, 4, true)?;
    let truncated_shard = inspected_shard(header, &payload, vec![truncated_tensor])?;
    let mut truncated_prepared = test_prepared(vec![truncated_shard], DType::F32)?;
    let truncated_length = first_shard_mut(&mut truncated_prepared)?
        .file_length
        .checked_sub(1)
        .ok_or_else(|| "cannot truncate empty file".to_owned())?;
    first_shard_mut(&mut truncated_prepared)?
        .file
        .set_len(truncated_length)
        .map_err(|error| error.to_string())?;
    let error = required_error(
        truncated_prepared.materialize_shard(0, &mut Events::default()),
        "truncation must fail before streaming",
    )?;
    assert_eq!(failure_code(error), Some(CODE_SOURCE_IDENTITY_LENGTH));
    Ok(())
}

#[test]
fn concrete_required_payload_read_failure_has_exact_tensor_context() -> Result<(), String> {
    let header = br#"{"required":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#;
    let payload = [0_u8, 0, 128, 63];
    let tensor = inspected_tensor("required", SourceTensorDType::F32, &[1], 0, 4, true)?;
    let shard = inspected_shard(header, &payload, vec![tensor])?;
    let mut prepared = test_prepared(vec![shard], DType::F32)?;
    TEST_REQUIRED_PAYLOAD_READ_FAILURES.with(|remaining| remaining.set(1));
    let error = required_error(
        prepared.materialize_shard(0, &mut Events::default()),
        "injected required payload read must fail",
    )?;
    assert_tensor_context(
        error,
        LoadFailureStage::PayloadRead,
        TensorFailureLocation::new(
            0,
            0,
            tensor_name_fingerprint("required"),
            Some(ScalarType::F32),
        ),
    );
    Ok(())
}
