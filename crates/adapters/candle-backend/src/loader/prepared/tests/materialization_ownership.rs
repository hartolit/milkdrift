use super::support::*;

#[test]
fn source_cast_and_map_faults_retain_real_owners() -> Result<(), String> {
    for (checkpoint, source_dtype, execution_dtype) in [
        (
            MaterializationCheckpoint::SourceOwned {
                shard_index: 0,
                tensor_index: 0,
            },
            SourceTensorDType::F32,
            DType::F32,
        ),
        (
            MaterializationCheckpoint::HostOwned {
                shard_index: 0,
                tensor_index: 0,
            },
            SourceTensorDType::F32,
            DType::F32,
        ),
        (
            MaterializationCheckpoint::CastOwned {
                shard_index: 0,
                tensor_index: 0,
            },
            SourceTensorDType::F32,
            DType::F16,
        ),
        (
            MaterializationCheckpoint::BeforeCpuMapInsertion {
                shard_index: 0,
                tensor_index: 0,
            },
            SourceTensorDType::F32,
            DType::F32,
        ),
        (
            MaterializationCheckpoint::CpuMapOwned {
                shard_index: 0,
                tensor_index: 0,
            },
            SourceTensorDType::F32,
            DType::F32,
        ),
    ] {
        let header = br#"{"required":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#;
        let payload = [0_u8, 0, 128, 63];
        let tensor = inspected_tensor("required", source_dtype, &[1], 0, 4, true)?;
        let shard = inspected_shard(header, &payload, vec![tensor])?;
        let mut prepared = test_prepared(vec![shard], execution_dtype)?;
        let error = required_error(
            prepared.materialize_shard(0, &mut FailAt(checkpoint)),
            "injected ownership checkpoint must fail",
        )?;
        let expected_stage = match checkpoint {
            MaterializationCheckpoint::CastOwned { .. } => LoadFailureStage::ScalarConversion,
            MaterializationCheckpoint::BeforeCpuMapInsertion { .. }
            | MaterializationCheckpoint::CpuMapOwned { .. } => LoadFailureStage::RetainedPlacement,
            _ => LoadFailureStage::HostMaterialization,
        };
        assert_tensor_context(
            error,
            expected_stage,
            TensorFailureLocation::new(
                0,
                0,
                tensor_name_fingerprint("required"),
                Some(ScalarType::F32),
            ),
        );
        match checkpoint {
            MaterializationCheckpoint::SourceOwned { .. } => {
                assert!(prepared.pending_source_tensor.is_some());
            }
            MaterializationCheckpoint::HostOwned { .. } => {
                assert!(prepared.pending_source_tensor.is_none());
                assert!(prepared.pending_host_tensor.is_some());
            }
            MaterializationCheckpoint::CastOwned { .. } => {
                assert!(prepared.pending_source_tensor.is_some());
                assert!(prepared.pending_host_tensor.is_some());
            }
            MaterializationCheckpoint::BeforeCpuMapInsertion { .. } => {
                assert!(prepared.pending_host_tensor.is_some());
            }
            MaterializationCheckpoint::CpuMapOwned { .. } => {
                assert!(prepared.final_tensors.contains_key("required"));
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

#[test]
fn duplicate_cpu_placement_reports_the_current_tensor() -> Result<(), String> {
    let header = br#"{"required":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#;
    let payload = [0_u8, 0, 128, 63];
    let tensor = inspected_tensor("required", SourceTensorDType::F32, &[1], 0, 4, true)?;
    let shard = inspected_shard(header, &payload, vec![tensor])?;
    let mut prepared = test_prepared(vec![shard], DType::F32)?;
    prepared.final_tensors.insert(
        "required".to_owned(),
        Tensor::zeros(1, DType::F32, &Device::Cpu)
            .map_err(|error| format!("create existing tensor: {error}"))?,
    );

    let error = required_error(
        prepared.materialize_shard(0, &mut Events::default()),
        "duplicate placement must fail",
    )?;
    assert_tensor_context(
        error,
        LoadFailureStage::RetainedPlacement,
        TensorFailureLocation::new(
            0,
            0,
            tensor_name_fingerprint("required"),
            Some(ScalarType::F32),
        ),
    );
    Ok(())
}
