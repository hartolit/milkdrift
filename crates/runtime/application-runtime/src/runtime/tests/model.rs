use domain_contracts::{
    DeviceId, DeviceKind, ExecutionDevice, MemoryBudget, MemoryFootprint, ScalarType,
};
use hf_hub_adapter::ArtifactScalarType;
use redb_storage::{RedbStorage, StoredScalarType};

use super::support::*;
use crate::runtime::model::LoadAdmission;
use crate::{
    ApplicationDevice, ApplicationError, ApplicationEvent, ApplicationRuntime,
    ApplicationScalarType, ModelSelection, ResolvedModel,
};

#[test]
fn resolved_selection_defaults_persist_without_restoring_resolution() -> TestResult {
    let database_path = unique_database_path();
    let result = with_runtime_at(&database_path, default_test_configuration, |runtime| {
        let (_selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        assert_eq!(runtime.preferences().default_repository, REPOSITORY);
        assert_eq!(runtime.preferences().default_revision, REVISION);
        Ok(())
    })
    .and_then(|()| {
        with_runtime_at(&database_path, default_test_configuration, |runtime| {
            assert_eq!(runtime.preferences().default_repository, REPOSITORY);
            assert_eq!(runtime.preferences().default_revision, REVISION);
            assert!(runtime.state().resolved().is_none());
            Ok(())
        })
    })
    .and_then(|()| {
        let storage = RedbStorage::open(&database_path).map_err(application_error)?;
        let name = format!("{REPOSITORY}@{COMMIT}");
        let record = storage
            .load_model(&name)
            .map_err(application_error)?
            .ok_or_else(|| "resolved model catalogue record was absent".to_owned())?;
        assert_eq!(record.repository, REPOSITORY);
        assert_eq!(record.revision, COMMIT);
        assert_eq!(record.scalar_type, StoredScalarType::F32);
        Ok(())
    });

    let cleanup_result = remove_database(&database_path);
    result.and(cleanup_result)
}

#[test]
fn repository_or_revision_change_is_rejected_after_resolution() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        let changed_repository = ModelSelection::new("fixture/other-model", REVISION);
        assert_eq!(
            runtime.load_model(&changed_repository),
            Err(ApplicationError::SelectionChanged)
        );
        let changed_revision = ModelSelection::new(REPOSITORY, "other-revision");
        assert_eq!(
            runtime.load_model(&changed_revision),
            Err(ApplicationError::SelectionChanged)
        );
        assert!(runtime.state().can_load(&selection));
        Ok(())
    })
}

#[test]
fn f32_fixture_reports_source_and_execution_scalars_from_distinct_evidence() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        assert_eq!(
            runtime
                .state()
                .resolved()
                .and_then(ResolvedModel::source_scalar_type),
            Some(ApplicationScalarType::F32)
        );
        assert_eq!(loaded.source_scalar_type(), ApplicationScalarType::F32);
        assert_eq!(loaded.execution_scalar_type(), ApplicationScalarType::F32);
        assert_eq!(loaded.device(), ApplicationDevice::Cpu);
        Ok(())
    })
}

#[test]
fn controlled_receipt_evidence_allows_bf16_source_with_f32_execution() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, mut receipt) = receive_successful_load_receipt(runtime)?;

        // This isolates E1 evidence handling. The physical fixture loaded by Candle is F32;
        // this test deliberately controls only the private resolution/admission/receipt facts and
        // does not claim that the backend executed a BF16 fixture.
        runtime
            .resolved_artifacts
            .as_mut()
            .ok_or_else(|| "resolved artifacts were absent".to_owned())?
            .declared_scalar_type = Some(ArtifactScalarType::Bf16);
        runtime
            .pending_load
            .as_mut()
            .ok_or_else(|| "load admission evidence was absent".to_owned())?
            .source_scalar_type = ApplicationScalarType::Bf16;
        runtime.state.set_resolved(ResolvedModel::new(
            resolved.selection().clone(),
            resolved.identity().clone(),
            resolved.vocabulary_size(),
            Some(ApplicationScalarType::Bf16),
            resolved.chat_compatibility(),
        ));
        receipt.descriptor.metadata.scalar_type = ScalarType::Bf16;
        receipt.execution_scalar_type = ScalarType::F32;

        let event = runtime.process_model_loaded(ticket, Ok(receipt));
        let ApplicationEvent::ModelLoaded { model } = event else {
            return Err(format!(
                "controlled scalar evidence was rejected: {event:?}"
            ));
        };
        assert_eq!(model.source_scalar_type(), ApplicationScalarType::Bf16);
        assert_eq!(model.execution_scalar_type(), ApplicationScalarType::F32);
        assert_eq!(model.device(), ApplicationDevice::Cpu);

        runtime.unload_model().map_err(application_error)?;
        let _unloaded = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        Ok(())
    })
}

#[test]
fn unloading_clears_loaded_scalar_and_device_facts_but_preserves_selection() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        assert_eq!(loaded.device(), ApplicationDevice::Cpu);
        assert_eq!(loaded.source_scalar_type(), ApplicationScalarType::F32);
        assert_eq!(loaded.execution_scalar_type(), ApplicationScalarType::F32);
        assert_eq!(runtime.state().selected_device(), ApplicationDevice::Cpu);

        runtime.unload_model().map_err(application_error)?;
        let _unloaded = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;

        assert!(runtime.state().loaded().is_none());
        assert_eq!(runtime.state().selected_device(), ApplicationDevice::Cpu);
        assert_eq!(
            runtime
                .state()
                .resolved()
                .and_then(ResolvedModel::source_scalar_type),
            Some(ApplicationScalarType::F32)
        );
        Ok(())
    })
}

#[test]
fn load_footprint_validation_rejects_budget_overflow_and_wrong_memory_domains() {
    let cpu_device = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu);
    let admission = LoadAdmission {
        ticket: inference_runtime::CommandTicket::new(1),
        source_scalar_type: ApplicationScalarType::F32,
        selected_device: ApplicationDevice::Cpu,
        execution_device: cpu_device,
        memory_budget: MemoryBudget {
            host_bytes: 100,
            device_bytes: 100,
        },
    };
    let valid_cpu = MemoryFootprint {
        host_weight_bytes: 60,
        host_working_bytes: 40,
        ..MemoryFootprint::default()
    };
    assert!(ApplicationRuntime::load_footprint_matches(
        admission, valid_cpu
    ));

    let over_budget = MemoryFootprint {
        host_weight_bytes: 61,
        ..valid_cpu
    };
    assert!(!ApplicationRuntime::load_footprint_matches(
        admission,
        over_budget
    ));
    let overflowing = MemoryFootprint {
        host_weight_bytes: u64::MAX,
        host_working_bytes: 1,
        ..MemoryFootprint::default()
    };
    assert!(!ApplicationRuntime::load_footprint_matches(
        admission,
        overflowing
    ));
    let cpu_with_device_weight = MemoryFootprint {
        host_weight_bytes: 99,
        device_weight_bytes: 1,
        ..MemoryFootprint::default()
    };
    assert!(!ApplicationRuntime::load_footprint_matches(
        admission,
        cpu_with_device_weight
    ));

    let cuda_admission = LoadAdmission {
        selected_device: CUDA_ZERO,
        execution_device: ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda),
        ..admission
    };
    let cuda_with_host_weight = MemoryFootprint {
        host_weight_bytes: 1,
        device_weight_bytes: 99,
        ..MemoryFootprint::default()
    };
    assert!(!ApplicationRuntime::load_footprint_matches(
        cuda_admission,
        cuda_with_host_weight
    ));
}
