use std::fs;
use std::path::Path;

use domain_contracts::{
    BackendFailure, BackendFailureKind, BackendId, CapacityExhausted, CapacityResource, DeviceId,
    DeviceKind, ExecutionDevice, LoadError, MemoryBudget, MemoryFootprint, MemoryKind, ModelHandle,
    ModelId, ScalarType, ScalarTypeSet,
};
use hf_hub_adapter::ArtifactScalarType;
use inference_runtime::{
    CleanupFailureReport, CleanupResource, CleanupRetryState, FailureClass, RetainedOwnership,
    RuntimeError, RuntimeOperation,
};
use redb_storage::{RedbStorage, StoredScalarType};

use super::support::*;
use crate::runtime::model::{LoadAdmission, LoadReceiptMismatch};
use crate::{
    ApplicationActivity, ApplicationDevice, ApplicationError, ApplicationEvent,
    ApplicationFailureKind, ApplicationMemoryFootprint, ApplicationRetainedModelResource,
    ApplicationRetainedOwnership, ApplicationRuntime, ApplicationScalarType, ModelSelection,
    ResolvedModel,
};

fn retained_load_cleanup(
    handle: ModelHandle,
    footprint: MemoryFootprint,
    attempts: u32,
) -> CleanupRetryState {
    CleanupRetryState {
        resource: CleanupResource::FailedLoad { handle },
        failure: CleanupFailureReport::new(
            RuntimeOperation::ModelLoad,
            FailureClass::Load,
            RuntimeOperation::FailedLoadCleanup,
            FailureClass::Synchronization,
        ),
        ownership: RetainedOwnership::Exact(footprint),
        attempts,
        maximum_attempts: 3,
    }
}

fn copy_test_file(source: &Path, destination: &Path) -> TestResult {
    fs::copy(source, destination).map(|_| ()).map_err(|error| {
        format!(
            "failed to copy test file {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })
}

fn mutate_test_file_without_changing_size(path: &Path) -> TestResult {
    let mut bytes = fs::read(path)
        .map_err(|error| format!("failed to read test file {}: {error}", path.display()))?;
    let byte_length = bytes.len();
    let first = bytes
        .first_mut()
        .ok_or_else(|| format!("test file {} was empty", path.display()))?;
    *first = first.wrapping_add(1);
    fs::write(path, bytes)
        .map_err(|error| format!("failed to mutate test file {}: {error}", path.display()))?;
    let mutated_length = usize::try_from(
        fs::metadata(path)
            .map_err(|error| format!("failed to inspect test file {}: {error}", path.display()))?
            .len(),
    )
    .map_err(|error| error.to_string())?;
    if mutated_length != byte_length {
        return Err(format!(
            "same-size mutation changed {} from {byte_length} to {mutated_length} bytes",
            path.display()
        ));
    }
    Ok(())
}

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
fn same_size_tokenizer_mutation_before_acceptance_is_rejected_as_artifact_identity() -> TestResult {
    let database_path = unique_database_path();
    let tokenizer_path = database_path.with_extension("tokenizer.json");
    let result = copy_test_file(
        tokenizer_fixture_path("tokenizer.json").as_path(),
        &tokenizer_path,
    )
    .and_then(|()| {
        with_runtime_at(&database_path, default_test_configuration, |runtime| {
            let config_path = candle_fixture_configuration_path();
            let artifacts = fixture_artifacts_with_paths(
                REPOSITORY,
                COMMIT,
                &tokenizer_path,
                config_path.as_path(),
                Some(ArtifactScalarType::F32),
            )?;
            mutate_test_file_without_changing_size(&tokenizer_path)?;

            let event = runtime.accept_resolved_artifacts(artifacts);
            let ApplicationEvent::ModelResolutionFailed { failure } = event else {
                return Err(format!(
                    "mutated tokenizer identity was accepted: {event:?}"
                ));
            };
            assert_eq!(failure.kind, ApplicationFailureKind::ArtifactResolution);
            assert!(
                failure
                    .message
                    .contains("resolved tokenizer content verification failed")
            );
            assert!(runtime.state().resolved().is_none());
            assert!(runtime.tokenizer.is_none());
            Ok(())
        })
    });

    let database_cleanup = remove_database(&database_path);
    let tokenizer_cleanup = remove_test_file(&tokenizer_path);
    result.and(database_cleanup).and(tokenizer_cleanup)
}

#[test]
fn verified_malformed_tokenizer_is_rejected_as_tokenizer_parser_failure() -> TestResult {
    let database_path = unique_database_path();
    let tokenizer_path = database_path.with_extension("tokenizer.json");
    let result = fs::write(&tokenizer_path, br#"{"not":"a tokenizer"}"#)
        .map_err(|error| error.to_string())
        .and_then(|()| {
            with_runtime_at(&database_path, default_test_configuration, |runtime| {
                let config_path = candle_fixture_configuration_path();
                let artifacts = fixture_artifacts_with_paths(
                    REPOSITORY,
                    COMMIT,
                    &tokenizer_path,
                    config_path.as_path(),
                    Some(ArtifactScalarType::F32),
                )?;

                let event = runtime.accept_resolved_artifacts(artifacts);
                let ApplicationEvent::ModelResolutionFailed { failure } = event else {
                    return Err(format!("malformed tokenizer was accepted: {event:?}"));
                };
                assert_eq!(failure.kind, ApplicationFailureKind::Tokenizer);
                assert!(failure.message.contains("valid supported tokenizer"));
                assert!(runtime.state().resolved().is_none());
                Ok(())
            })
        });

    let database_cleanup = remove_database(&database_path);
    let tokenizer_cleanup = remove_test_file(&tokenizer_path);
    result.and(database_cleanup).and(tokenizer_cleanup)
}

#[test]
fn same_size_config_mutation_after_acceptance_is_rejected_before_load_submission() -> TestResult {
    let database_path = unique_database_path();
    let config_path = database_path.with_extension("config.json");
    let result = copy_test_file(candle_fixture_configuration_path().as_path(), &config_path)
        .and_then(|()| {
            with_runtime_at(&database_path, default_test_configuration, |runtime| {
                let (selection, _resolved) = resolve_fixture_with_configuration(
                    runtime,
                    REPOSITORY,
                    COMMIT,
                    "tokenizer.json",
                    &config_path,
                    Some(ArtifactScalarType::F32),
                )?;
                mutate_test_file_without_changing_size(&config_path)?;

                let error = runtime
                    .load_model(&selection)
                    .err()
                    .ok_or_else(|| "mutated configuration was submitted for loading".to_owned())?;
                let ApplicationError::Failure(failure) = error else {
                    return Err(format!(
                        "configuration identity failure had the wrong error shape: {error:?}"
                    ));
                };
                assert_eq!(failure.kind, ApplicationFailureKind::ArtifactResolution);
                assert!(
                    failure
                        .message
                        .contains("resolved model configuration verification failed before load")
                );
                assert!(runtime.pending_load.is_none());
                assert!(runtime.state().can_load(&selection));
                Ok(())
            })
        });

    let database_cleanup = remove_database(&database_path);
    let config_cleanup = remove_test_file(&config_path);
    result.and(database_cleanup).and(config_cleanup)
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

        let event = runtime.process_model_loaded(ticket, &Ok(receipt));
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
        let event = runtime.process_model_loaded(ticket, &Ok(receipt));
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

        let event = runtime.process_model_loaded(ticket, &Ok(receipt));
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
fn stale_non_ownership_load_failure_does_not_consume_the_active_transaction() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, receipt) = receive_successful_load_receipt(runtime)?;
        let stale_ticket = inference_runtime::CommandTicket::new(ticket.get().saturating_add(1));

        assert_eq!(
            runtime.process_model_loaded(
                stale_ticket,
                &Err(RuntimeError::Load(LoadError::UnsupportedFormat)),
            ),
            None
        );
        assert!(runtime.pending_load.is_some());

        let event = runtime.process_model_loaded(ticket, &Ok(receipt));
        assert!(matches!(event, Some(ApplicationEvent::ModelLoaded { .. })));
        runtime.unload_model().map_err(application_error)?;
        let _unloaded = wait_for_event(runtime, |event| {
            matches!(event, ApplicationEvent::ModelUnloaded { .. })
        })?;
        Ok(())
    })
}

#[test]
fn wrong_ticket_load_receipt_is_quarantined_and_invalidates_the_transaction() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, receipt) = receive_successful_load_receipt(runtime)?;
        let stale_ticket = inference_runtime::CommandTicket::new(ticket.get().saturating_add(1));

        let event = runtime.process_model_loaded(stale_ticket, &Ok(receipt));
        let Some(ApplicationEvent::ModelCompatibilityFailed { failure }) = event else {
            return Err(format!(
                "wrong-ticket receipt was not quarantined: {event:?}"
            ));
        };
        assert_eq!(failure.kind, ApplicationFailureKind::IncompatibleReceipt);
        assert_eq!(failure.message, LoadReceiptMismatch::Ticket.message());
        assert!(runtime.pending_load.is_none());
        let retained = runtime
            .state()
            .retained_model()
            .ok_or_else(|| "wrong-ticket receipt did not retain its owner".to_owned())?;
        assert_eq!(
            retained.resource(),
            ApplicationRetainedModelResource::LoadedModel {
                handle: receipt.handle,
            }
        );
        assert_eq!(
            retained.ownership(),
            ApplicationRetainedOwnership::Exact(ApplicationMemoryFootprint::from(
                receipt.reserved_footprint
            ))
        );
        assert_eq!(retained.primary_failure(), &failure);
        assert_eq!(
            runtime.state().activity(),
            ApplicationActivity::RetainedCleanup
        );
        assert!(runtime.state().loaded().is_none());
        assert!(!runtime.state().can_load(&selection));
        assert!(!runtime.state().can_select_device());
        Ok(())
    })
}

#[test]
fn load_receipt_without_a_pending_transaction_is_quarantined_from_idle() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (_ticket, receipt) = receive_successful_load_receipt(runtime)?;
        runtime.pending_load = None;
        runtime.state.set_idle();

        let event =
            runtime.process_model_loaded(inference_runtime::CommandTicket::new(991), &Ok(receipt));
        let Some(ApplicationEvent::ModelCompatibilityFailed { failure }) = event else {
            return Err(format!(
                "uncorrelated receipt was not quarantined: {event:?}"
            ));
        };
        assert_eq!(failure.kind, ApplicationFailureKind::IncompatibleReceipt);
        assert_eq!(
            failure.message,
            LoadReceiptMismatch::MissingTransaction.message()
        );
        assert!(runtime.pending_load.is_none());
        let retained = runtime
            .state()
            .retained_model()
            .ok_or_else(|| "uncorrelated receipt did not retain its owner".to_owned())?;
        assert_eq!(
            retained.resource(),
            ApplicationRetainedModelResource::LoadedModel {
                handle: receipt.handle,
            }
        );
        assert_eq!(
            runtime.state().activity(),
            ApplicationActivity::RetainedCleanup
        );
        assert!(runtime.state().loaded().is_none());
        assert!(!runtime.state().can_load(&selection));
        assert!(!runtime.state().can_select_device());
        Ok(())
    })
}

#[test]
fn wrong_ticket_retained_cleanup_result_is_not_dropped() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let (selection, _resolved) =
            resolve_fixture_with(runtime, REPOSITORY, COMMIT, "tokenizer.json")?;
        runtime.load_model(&selection).map_err(application_error)?;
        let (ticket, receipt) = receive_successful_load_receipt(runtime)?;
        let stale_ticket = inference_runtime::CommandTicket::new(ticket.get().saturating_add(1));
        let lower = retained_load_cleanup(receipt.handle, receipt.reserved_footprint, 1);

        let event =
            runtime.process_model_loaded(stale_ticket, &Err(RuntimeError::CleanupFailed(lower)));
        let Some(ApplicationEvent::ModelCleanupPending { .. }) = event else {
            return Err(format!(
                "wrong-ticket retained cleanup was not quarantined: {event:?}"
            ));
        };
        let cleanup = runtime
            .state()
            .retained_model()
            .cloned()
            .ok_or_else(|| "cleanup event omitted durable retained state".to_owned())?;
        assert_eq!(
            cleanup.primary_failure().kind,
            ApplicationFailureKind::IncompatibleReceipt
        );
        assert_eq!(
            cleanup.primary_failure().message,
            LoadReceiptMismatch::Ticket.message()
        );
        assert_eq!(
            cleanup.resource(),
            ApplicationRetainedModelResource::FailedLoad {
                handle: receipt.handle,
            }
        );
        assert_eq!(runtime.state().retained_model(), Some(&cleanup));
        assert!(runtime.pending_load.is_none());
        assert_eq!(
            runtime.state().activity(),
            ApplicationActivity::RetainedCleanup
        );
        assert!(!runtime.state().can_load(&selection));
        Ok(())
    })
}

#[test]
fn exhausted_retained_cleanup_without_a_pending_transaction_is_not_dropped() -> TestResult {
    with_runtime(default_test_configuration, |runtime| {
        let handle = ModelHandle::new(ModelId::new(17), domain_contracts::ModelGeneration::new(19));
        let lower = retained_load_cleanup(handle, MemoryFootprint::default(), 3);
        runtime.state.set_idle();

        let event = runtime.process_model_loaded(
            inference_runtime::CommandTicket::new(992),
            &Err(RuntimeError::CleanupRetryExhausted(lower)),
        );
        let Some(ApplicationEvent::ModelCleanupPending { .. }) = event else {
            return Err(format!(
                "uncorrelated exhausted cleanup was not quarantined: {event:?}"
            ));
        };
        let cleanup = runtime
            .state()
            .retained_model()
            .cloned()
            .ok_or_else(|| "cleanup event omitted durable retained state".to_owned())?;
        assert_eq!(
            cleanup.primary_failure().kind,
            ApplicationFailureKind::IncompatibleReceipt
        );
        assert_eq!(
            cleanup.primary_failure().message,
            LoadReceiptMismatch::MissingTransaction.message()
        );
        assert_eq!(
            cleanup.resource(),
            ApplicationRetainedModelResource::FailedLoad { handle }
        );
        assert!(runtime.pending_load.is_none());
        assert_eq!(
            runtime.state().activity(),
            ApplicationActivity::RetainedCleanup
        );
        assert!(!runtime.state().can_select_device());
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
            let failure = crate::runtime::model::model_load_failure(&error);
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
