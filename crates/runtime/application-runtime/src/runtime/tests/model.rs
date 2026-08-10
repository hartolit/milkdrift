use domain_contracts::{
    BackendFailure, BackendFailureKind, BackendId, CapacityExhausted, CapacityResource, DeviceId,
    DeviceKind, ExecutionDevice, LoadError, MemoryBudget, MemoryFootprint, MemoryKind, ModelId,
    ScalarType, ScalarTypeSet,
};

use redb_storage::{RedbStorage, StoredScalarType};

use super::support::*;
use crate::runtime::model::{LoadAdmission, LoadReceiptMismatch};
use crate::{
    ApplicationDevice, ApplicationError, ApplicationEvent, ApplicationFailureKind,
    ApplicationRuntime, ApplicationScalarType, ModelSelection, ResolvedModel,
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
        assert_eq!(
            record.configuration_declared_scalar_type,
            Some(StoredScalarType::F32)
        );
        Ok(())
    });

    let cleanup_result = remove_database(&database_path);
    result.and(cleanup_result)
}

#[test]
fn absent_configuration_declaration_remains_loadable_and_is_persisted() -> TestResult {
    let database_path = unique_database_path();
    let config_path = database_path.with_extension("config.json");
    write_fixture_configuration_without_declaration(&config_path)?;
    let result = with_runtime_at(&database_path, default_test_configuration, |runtime| {
        let (selection, resolved) = resolve_fixture_with_configuration(
            runtime,
            REPOSITORY,
            COMMIT,
            "tokenizer.json",
            &config_path,
            None,
        )?;
        assert_eq!(resolved.configuration_declared_scalar_type(), None);
        assert!(runtime.state().can_load(&selection));

        runtime.load_model(&selection).map_err(application_error)?;
        let event = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelLoaded { .. })
        })?;
        let ApplicationEvent::ModelLoaded { model } = event else {
            return Err("model without a configuration declaration did not load".to_owned());
        };
        assert_eq!(model.execution_scalar_type(), ApplicationScalarType::F32);

        runtime.unload_model().map_err(application_error)?;
        let _unloaded = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        Ok(())
    })
    .and_then(|()| {
        let storage = RedbStorage::open(&database_path).map_err(application_error)?;
        let name = format!("{REPOSITORY}@{COMMIT}");
        let record = storage
            .load_model(&name)
            .map_err(application_error)?
            .ok_or_else(|| "resolved model catalogue record was absent".to_owned())?;
        assert_eq!(record.configuration_declared_scalar_type, None);
        Ok(())
    });

    let database_cleanup = remove_database(&database_path);
    let config_cleanup = remove_test_file(&config_path);
    result.and(database_cleanup).and(config_cleanup)
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
fn f32_fixture_reports_declared_metadata_and_execution_facts_from_distinct_evidence() -> TestResult
{
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        assert_eq!(
            runtime
                .state()
                .resolved()
                .and_then(ResolvedModel::configuration_declared_scalar_type),
            Some(ApplicationScalarType::F32)
        );
        assert_eq!(loaded.execution_scalar_type(), ApplicationScalarType::F32);
        assert_eq!(loaded.device(), ApplicationDevice::Cpu);
        Ok(())
    })
}

#[test]
fn controlled_mixed_dtype_receipt_allows_bf16_declaration_with_f32_execution() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, mut receipt) = receive_successful_load_receipt(runtime)?;

        // This isolates E1 evidence handling. The physical fixture loaded by Candle is F32;
        // this test deliberately controls only the private resolution/admission/receipt facts and
        // does not claim that the backend executed a BF16 fixture.
        let transaction = runtime
            .pending_load
            .as_mut()
            .ok_or_else(|| "load transaction was absent".to_owned())?;
        transaction.resolved.model = ResolvedModel::new(
            resolved.selection().clone(),
            resolved.identity().clone(),
            resolved.vocabulary_size(),
            Some(ApplicationScalarType::Bf16),
            resolved.prompt_compatibility_profile(),
        );
        receipt
            .descriptor
            .metadata
            .configuration_declared_scalar_type = Some(ScalarType::Bf16);
        receipt.descriptor.metadata.observed_tensor_scalar_types =
            ScalarTypeSet::from_scalar(ScalarType::Bf16)
                .union(ScalarTypeSet::from_scalar(ScalarType::F32));
        receipt.execution_scalar_type = ScalarType::F32;

        let event = runtime.process_model_loaded(ticket, Ok(receipt));
        let Some(ApplicationEvent::ModelLoaded { model }) = event else {
            return Err(format!(
                "controlled scalar evidence was rejected: {event:?}"
            ));
        };
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
fn execution_scalar_is_taken_from_the_verified_receipt_not_declaration_or_device() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, mut receipt) = receive_successful_load_receipt(runtime)?;

        // The received scalar is an E0-verified fact at this boundary. Controlling it here proves
        // E1 does not reproduce Candle's declaration- or device-aware scalar-selection policy.
        receipt.execution_scalar_type = ScalarType::F16;
        let event = runtime.process_model_loaded(ticket, Ok(receipt));
        let Some(ApplicationEvent::ModelLoaded { model }) = event else {
            return Err(format!(
                "supported execution scalar was rejected: {event:?}"
            ));
        };
        assert_eq!(model.execution_scalar_type(), ApplicationScalarType::F16);
        assert_eq!(model.device(), ApplicationDevice::Cpu);

        runtime.unload_model().map_err(application_error)?;
        let _unloaded = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        Ok(())
    })
}

#[test]
fn observed_scalar_extras_are_lower_evidence_not_e1_compatibility_policy() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, mut receipt) = receive_successful_load_receipt(runtime)?;
        receipt.descriptor.metadata.observed_tensor_scalar_types =
            ScalarTypeSet::from_scalar(ScalarType::F32)
                .union(ScalarTypeSet::from_scalar(ScalarType::F16))
                .union(ScalarTypeSet::from_scalar(ScalarType::Bf16))
                .union(ScalarTypeSet::from_scalar(ScalarType::I8))
                .union(ScalarTypeSet::from_scalar(ScalarType::U8))
                .union(ScalarTypeSet::from_scalar(ScalarType::Other(7)));

        let event = runtime.process_model_loaded(ticket, Ok(receipt));
        let Some(ApplicationEvent::ModelLoaded { model }) = event else {
            return Err(format!(
                "E1 rejected lower-accepted unused scalar extras: {event:?}"
            ));
        };
        assert_eq!(model.execution_scalar_type(), ApplicationScalarType::F32);

        runtime.unload_model().map_err(application_error)?;
        let _unloaded = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        Ok(())
    })
}

#[test]
fn load_receipt_validation_classifies_each_independent_transaction_fact() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (_ticket, receipt) = receive_successful_load_receipt(runtime)?;

        let transaction = runtime
            .pending_load
            .as_ref()
            .ok_or_else(|| "load transaction was absent".to_owned())?;
        assert!(runtime.validate_load_receipt(transaction, &receipt).is_ok());

        let mut mismatched = receipt;
        mismatched.handle.id = ModelId::new(2);
        assert_eq!(
            runtime
                .validate_load_receipt(transaction, &mismatched)
                .err(),
            Some(LoadReceiptMismatch::ModelIdentity)
        );

        let mut mismatched = receipt;
        mismatched
            .descriptor
            .metadata
            .configuration_declared_scalar_type = None;
        assert_eq!(
            runtime
                .validate_load_receipt(transaction, &mismatched)
                .err(),
            Some(LoadReceiptMismatch::Declaration)
        );

        let mut mismatched = receipt;
        mismatched.execution_scalar_type = ScalarType::I8;
        assert_eq!(
            runtime
                .validate_load_receipt(transaction, &mismatched)
                .err(),
            Some(LoadReceiptMismatch::ExecutionScalar)
        );

        let mut mismatched = receipt;
        mismatched.execution_device = ExecutionDevice::new(DeviceId::new(1), DeviceKind::Cpu);
        assert_eq!(
            runtime
                .validate_load_receipt(transaction, &mismatched)
                .err(),
            Some(LoadReceiptMismatch::ExecutionDevice)
        );

        let mut mismatched = receipt;
        mismatched.execution_device = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);
        assert_eq!(
            runtime
                .validate_load_receipt(transaction, &mismatched)
                .err(),
            Some(LoadReceiptMismatch::SelectedDevice)
        );

        let mut mismatched = receipt;
        mismatched.reserved_footprint.host_weight_bytes = u64::MAX;
        mismatched.reserved_footprint.host_working_bytes = 1;
        assert_eq!(
            runtime
                .validate_load_receipt(transaction, &mismatched)
                .err(),
            Some(LoadReceiptMismatch::FinalFootprint)
        );

        let mut mismatched = receipt;
        mismatched.descriptor.metadata.observed_tensor_scalar_types = ScalarTypeSet::EMPTY;
        assert_eq!(
            runtime
                .validate_load_receipt(transaction, &mismatched)
                .err(),
            Some(LoadReceiptMismatch::ObservedEvidence)
        );

        let original_budget = runtime.memory_budget;
        runtime.memory_budget.host_bytes = original_budget.host_bytes.saturating_sub(1);
        let transaction = runtime
            .pending_load
            .as_ref()
            .ok_or_else(|| "load transaction disappeared".to_owned())?;
        assert_eq!(
            runtime.validate_load_receipt(transaction, &receipt).err(),
            Some(LoadReceiptMismatch::MemoryBudget)
        );
        runtime.memory_budget = original_budget;
        Ok(())
    })
}

#[test]
fn stale_load_events_do_not_consume_the_active_transaction() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, receipt) = receive_successful_load_receipt(runtime)?;
        let stale_ticket = inference_runtime::CommandTicket::new(ticket.get().saturating_add(1));

        assert_eq!(
            runtime.process_model_loaded(
                stale_ticket,
                Err(inference_runtime::RuntimeError::Load(
                    LoadError::UnsupportedFormat
                )),
            ),
            None
        );
        assert!(runtime.pending_load.is_some());
        assert_eq!(
            runtime.process_model_loaded(stale_ticket, Ok(receipt)),
            None
        );
        assert!(runtime.pending_load.is_some());

        let event = runtime.process_model_loaded(ticket, Ok(receipt));
        assert!(matches!(event, Some(ApplicationEvent::ModelLoaded { .. })));
        runtime.unload_model().map_err(application_error)?;
        let _unloaded = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        Ok(())
    })
}

#[test]
fn unloading_clears_loaded_execution_facts_but_preserves_resolution_and_selection() -> TestResult {
    with_loaded_runtime(default_test_configuration, |runtime, loaded| {
        assert_eq!(loaded.device(), ApplicationDevice::Cpu);
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
                .and_then(ResolvedModel::configuration_declared_scalar_type),
            Some(ApplicationScalarType::F32)
        );
        Ok(())
    })
}

#[test]
fn model_load_failures_are_normalized_into_stable_application_categories() -> TestResult {
    with_runtime(default_test_configuration, |_runtime| {
        let cases = [
            (
                inference_runtime::RuntimeError::Load(LoadError::UnsupportedFormat),
                ApplicationFailureKind::UnsupportedArtifact,
            ),
            (
                inference_runtime::RuntimeError::Load(LoadError::CapacityExhausted(
                    CapacityExhausted::new(CapacityResource::BackendScratch, 65, 64),
                )),
                ApplicationFailureKind::UnsupportedArtifact,
            ),
            (
                inference_runtime::RuntimeError::Load(LoadError::Backend(BackendFailure::new(
                    BackendId::new(1),
                    BackendFailureKind::Unsupported,
                    22,
                ))),
                ApplicationFailureKind::UnsupportedArtifact,
            ),
            (
                inference_runtime::RuntimeError::Load(LoadError::InsufficientMemory {
                    kind: MemoryKind::Host,
                    required_bytes: 2,
                    available_bytes: 1,
                }),
                ApplicationFailureKind::MemoryAdmission,
            ),
            (
                inference_runtime::RuntimeError::Load(LoadError::Backend(BackendFailure::new(
                    BackendId::new(1),
                    BackendFailureKind::DeviceExecution,
                    29,
                ))),
                ApplicationFailureKind::ModelLoad,
            ),
        ];

        for (error, expected_kind) in cases {
            let failure = crate::runtime::model::model_load_failure(error);
            assert_eq!(failure.kind, expected_kind);
        }
        Ok(())
    })
}

#[test]
fn load_footprint_validation_checks_only_arithmetic_and_budget() {
    let admission = LoadAdmission {
        selected_device: ApplicationDevice::Cpu,
        execution_device: ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu),
        memory_budget: MemoryBudget {
            host_bytes: 100,
            device_bytes: 100,
        },
    };

    let mixed_domains = MemoryFootprint {
        host_weight_bytes: 40,
        device_weight_bytes: 40,
        host_working_bytes: 10,
        device_working_bytes: 10,
    };
    assert!(ApplicationRuntime::load_footprint_matches(
        &admission,
        mixed_domains
    ));

    let host_over_budget = MemoryFootprint {
        host_weight_bytes: 91,
        host_working_bytes: 10,
        ..MemoryFootprint::default()
    };
    assert!(!ApplicationRuntime::load_footprint_matches(
        &admission,
        host_over_budget
    ));
    let device_over_budget = MemoryFootprint {
        device_weight_bytes: 91,
        device_working_bytes: 10,
        ..MemoryFootprint::default()
    };
    assert!(!ApplicationRuntime::load_footprint_matches(
        &admission,
        device_over_budget
    ));
    let overflowing = MemoryFootprint {
        host_weight_bytes: u64::MAX,
        host_working_bytes: 1,
        ..MemoryFootprint::default()
    };
    assert!(!ApplicationRuntime::load_footprint_matches(
        &admission,
        overflowing
    ));
}
