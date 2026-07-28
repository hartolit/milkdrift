use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::mpsc;
use std::time::Duration;

use domain_contracts::{
    BackendFailure, BackendFailureKind, BackendId, BackendSequence, CapabilitySet,
    DecodeBufferRequirements, DecodeInput, DecodeOutcome, DeviceId, DeviceKind, LoadConfiguration,
    LoadError, LoadPlan, LoadedModel, MemoryBudget, MemoryFootprint, ModelArchitecture,
    ModelCapabilities, ModelDescriptor, ModelError, ModelHandle, ModelId, ModelLoader,
    ModelMetadata, PrefillBufferRequirements, PrefillInput, PrefillOutcome, PreparedDecodeBuffers,
    PreparedPrefillBuffers, QuantizationFormat, RequestId, ScalarType, SequenceConfiguration,
    SequenceError, SequenceId, SequencePlan, SequenceState, SynchronizationError,
};
use host_runtime::spawn_named;
use inference_runtime::{
    CleanupResource, FailureClass, HostedRuntimeConfiguration, LoadReceipt, RuntimeLimits,
    RuntimeOperation, RuntimeThread, start_hosted_runtime,
};

use super::*;

const BACKEND_ID: BackendId = BackendId::new(72);
const TEST_TIMEOUT: Duration = Duration::from_secs(1);
const TEST_POLL: Duration = Duration::from_millis(1);

type TestResult = Result<(), String>;

#[derive(Clone, Copy)]
struct TestSource;

struct TestLoader {
    fail_unload: bool,
}

struct TestModel {
    handle: ModelHandle,
    metadata: ModelMetadata,
    fail_unload: bool,
}

struct TestSequence {
    id: SequenceId,
    state: SequenceState,
    capacity: usize,
}

impl BackendSequence for TestSequence {
    fn id(&self) -> SequenceId {
        self.id
    }

    fn state(&self) -> SequenceState {
        self.state
    }

    fn position(&self) -> usize {
        0
    }

    fn token_capacity(&self) -> usize {
        self.capacity
    }
}

impl ModelLoader for TestLoader {
    type Source = TestSource;
    type Model = TestModel;

    fn inspect(&self, _source: &Self::Source) -> Result<ModelDescriptor, LoadError> {
        Ok(descriptor())
    }

    fn plan_load(
        &self,
        source: &Self::Source,
        _configuration: &LoadConfiguration,
    ) -> Result<LoadPlan, LoadError> {
        let descriptor = self.inspect(source)?;
        Ok(LoadPlan {
            descriptor,
            expected_footprint: descriptor.estimated_footprint,
        })
    }

    fn load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Model, LoadError> {
        Ok(TestModel {
            handle: configuration.handle,
            metadata: self.inspect(source)?.metadata,
            fail_unload: self.fail_unload,
        })
    }
}

impl LoadedModel for TestModel {
    type Sequence = TestSequence;

    fn handle(&self) -> ModelHandle {
        self.handle
    }

    fn metadata(&self) -> &ModelMetadata {
        &self.metadata
    }

    fn plan_sequence(
        &self,
        configuration: &SequenceConfiguration,
    ) -> Result<SequencePlan, ModelError> {
        Ok(SequencePlan {
            configuration: *configuration,
            expected_footprint: MemoryFootprint::default(),
            logits_capacity: self.metadata.vocabulary_size as usize,
        })
    }

    fn create_sequence(
        &mut self,
        sequence_id: SequenceId,
        configuration: &SequenceConfiguration,
    ) -> Result<Self::Sequence, ModelError> {
        let capacity = usize::try_from(configuration.maximum_tokens.get())
            .map_err(|_| ModelError::Backend(backend_failure()))?;
        Ok(TestSequence {
            id: sequence_id,
            state: SequenceState::Empty,
            capacity,
        })
    }

    fn prefill_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        _input: &PrefillInput<'_>,
    ) -> PrefillBufferRequirements {
        PrefillBufferRequirements { logits: 0 }
    }

    fn decode_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        _input: DecodeInput,
    ) -> DecodeBufferRequirements {
        DecodeBufferRequirements { logits: 0 }
    }

    fn prefill_prepared(
        &mut self,
        _sequence: &mut Self::Sequence,
        _input: PrefillInput<'_>,
        _buffers: PreparedPrefillBuffers<'_>,
    ) -> Result<PrefillOutcome, SequenceError> {
        Err(SequenceError::Unsupported)
    }

    fn decode_prepared(
        &mut self,
        _sequence: &mut Self::Sequence,
        _input: DecodeInput,
        _buffers: PreparedDecodeBuffers<'_>,
    ) -> Result<DecodeOutcome, SequenceError> {
        Err(SequenceError::Unsupported)
    }

    fn destroy_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        sequence.state = SequenceState::Finished;
        Ok(())
    }

    fn reset_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        sequence.state = SequenceState::Empty;
        Ok(())
    }

    fn synchronize(&mut self) -> Result<(), SynchronizationError> {
        Ok(())
    }

    fn prepare_unload(&mut self) -> Result<(), SynchronizationError> {
        if self.fail_unload {
            Err(SynchronizationError::Backend(backend_failure()))
        } else {
            Ok(())
        }
    }
}

#[test]
fn disconnected_inference_worker_is_a_successful_shutdown() -> TestResult {
    let (runtime, thread) = test_runtime(false)?;
    let first = shutdown_runtime_worker(&runtime, CommandTicket::new(1), TEST_TIMEOUT, TEST_POLL)
        .map_err(debug_error)?;
    assert!(matches!(first, RuntimeShutdown::Finished(Ok(_))));
    thread.join().map_err(debug_error)?;

    let second = shutdown_runtime_worker(&runtime, CommandTicket::new(2), TEST_TIMEOUT, TEST_POLL)
        .map_err(debug_error)?;
    assert_eq!(second, RuntimeShutdown::Disconnected);
    assert_eq!(normalize_runtime_shutdown(second), Ok(()));
    Ok(())
}

#[test]
fn host_worker_shutdown_timeout_is_bounded() -> TestResult {
    let (release_sender, release_receiver) = mpsc::channel();
    let thread = spawn_named("application-shutdown-timeout-test", move || {
        let _release_result = release_receiver.recv();
    })
    .map_err(|error| error.to_string())?;

    let result = wait_for_host_thread(&thread, Duration::from_millis(5), Duration::from_millis(1));
    assert_eq!(
        result,
        Err(ApplicationError::ShutdownTimeout(ApplicationWorker::Hub))
    );

    release_sender
        .send(())
        .map_err(|_| "timeout test worker disconnected".to_owned())?;
    thread.join().map_err(debug_error)?;
    Ok(())
}

#[test]
fn host_worker_join_failure_is_reported() -> TestResult {
    let thread = spawn_named("application-shutdown-panic-test", || {
        std::panic::resume_unwind(Box::new("intentional test panic"));
    })
    .map_err(|error| error.to_string())?;

    let result = finish_host_thread(thread, TEST_TIMEOUT, TEST_POLL);
    assert!(matches!(
        result,
        Err(ApplicationError::Failure(ApplicationFailure {
            kind: ApplicationFailureKind::Worker,
            ..
        }))
    ));
    Ok(())
}

#[test]
fn shutdown_cancels_active_request_and_unloads_model() -> TestResult {
    let (runtime, thread) = test_runtime(false)?;
    let loaded = load_model(&runtime, 10)?;
    start_request(&runtime, loaded.handle, 11)?;

    let outcome =
        shutdown_runtime_worker(&runtime, CommandTicket::new(12), TEST_TIMEOUT, TEST_POLL)
            .map_err(debug_error)?;
    let RuntimeShutdown::Finished(Ok(receipt)) = outcome else {
        return Err(format!("unexpected active-request shutdown: {outcome:?}"));
    };
    assert_eq!(receipt.cancelled_requests, 1);
    assert_eq!(receipt.unloaded_models, 1);
    thread.join().map_err(debug_error)?;
    Ok(())
}

#[test]
fn failed_cleanup_shutdown_stops_worker_without_dropping_client() -> TestResult {
    let (runtime, thread) = test_runtime(true)?;
    let _loaded = load_model(&runtime, 20)?;

    let outcome =
        shutdown_runtime_worker(&runtime, CommandTicket::new(21), TEST_TIMEOUT, TEST_POLL)
            .map_err(debug_error)?;
    let RuntimeShutdown::Finished(Err(RuntimeError::CleanupRetryExhausted(state))) = outcome else {
        return Err(format!("unexpected failed-cleanup shutdown: {outcome:?}"));
    };
    assert_eq!(
        state.resource,
        CleanupResource::Model {
            model_id: ModelId::new(20),
        }
    );
    assert_eq!(state.failure.primary_operation, RuntimeOperation::Shutdown);
    assert_eq!(state.failure.primary_failure, FailureClass::Shutdown);
    assert_eq!(
        state.failure.cleanup_operation,
        RuntimeOperation::ModelUnload
    );
    assert_eq!(state.failure.cleanup_failure, FailureClass::Synchronization);
    assert_eq!(state.attempts, 3);
    assert_eq!(state.maximum_attempts, 3);
    assert!(state.exhausted());
    assert!(matches!(
        normalize_runtime_shutdown(outcome),
        Err(ApplicationError::Failure(ApplicationFailure {
            kind: ApplicationFailureKind::Inference,
            ..
        }))
    ));

    thread.join().map_err(debug_error)?;
    Ok(())
}

#[test]
fn deadline_overflow_is_rejected_as_invalid_configuration() {
    assert_eq!(
        checked_deadline(
            Duration::MAX,
            crate::ApplicationConfigurationField::RuntimeShutdownTimeout,
        ),
        Err(ApplicationError::InvalidConfiguration(
            crate::ApplicationConfigurationField::RuntimeShutdownTimeout,
        ))
    );
}

fn test_runtime(fail_unload: bool) -> Result<(HostedRuntime<TestSource>, RuntimeThread), String> {
    let limits = RuntimeLimits::new(
        NonZeroU32::MIN,
        NonZeroU32::MIN,
        MemoryBudget {
            host_bytes: 1_024,
            device_bytes: 0,
        },
    );
    let configuration = HostedRuntimeConfiguration::new(
        NonZeroUsize::new(4).unwrap_or(NonZeroUsize::MIN),
        NonZeroUsize::new(4).unwrap_or(NonZeroUsize::MIN),
        NonZeroU64::MIN,
    );
    start_hosted_runtime(TestLoader { fail_unload }, limits, configuration)
        .map_err(|error| error.to_string())
}

fn load_model(runtime: &HostedRuntime<TestSource>, ticket: u64) -> Result<LoadReceipt, String> {
    runtime
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(ticket),
            model_id: ModelId::new(ticket),
            source: TestSource,
            device: DeviceId::new(0),
            device_kind: DeviceKind::Cpu,
        })
        .map_err(|_| "load command was rejected".to_owned())?;
    let event = runtime.receive_timeout(TEST_TIMEOUT).map_err(debug_error)?;
    let RuntimeEvent::ModelLoaded {
        result: Ok(receipt),
        ..
    } = event
    else {
        return Err("unexpected model-load event".to_owned());
    };
    Ok(receipt)
}

fn start_request(
    runtime: &HostedRuntime<TestSource>,
    handle: ModelHandle,
    ticket: u64,
) -> TestResult {
    runtime
        .try_submit(RuntimeCommand::StartRequest {
            ticket: CommandTicket::new(ticket),
            handle,
            request_id: RequestId::new(ticket),
            sequence_id: SequenceId::new(ticket),
            configuration: SequenceConfiguration::new(NonZeroU32::MIN, NonZeroU32::MIN),
        })
        .map_err(|_| "request-start command was rejected".to_owned())?;
    let event = runtime.receive_timeout(TEST_TIMEOUT).map_err(debug_error)?;
    if matches!(event, RuntimeEvent::RequestStarted { result: Ok(_), .. }) {
        Ok(())
    } else {
        Err("unexpected request-start event".to_owned())
    }
}

fn descriptor() -> ModelDescriptor {
    ModelDescriptor {
        backend: BACKEND_ID,
        metadata: ModelMetadata {
            architecture: ModelArchitecture::Llama,
            scalar_type: ScalarType::F32,
            quantization: QuantizationFormat::None,
            vocabulary_size: 4,
            context_length: 16,
        },
        capabilities: ModelCapabilities {
            operations: CapabilitySet::PREFILL
                .union(CapabilitySet::INCREMENTAL_DECODE)
                .union(CapabilitySet::MULTIPLE_SEQUENCES)
                .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION),
            maximum_context_tokens: 16,
            maximum_sequences: 1,
            maximum_prefill_batch: 1,
        },
        estimated_footprint: MemoryFootprint {
            host_weight_bytes: 1,
            device_weight_bytes: 0,
            host_working_bytes: 0,
            device_working_bytes: 0,
            cache_bytes_per_token: 0,
        },
    }
}

const fn backend_failure() -> BackendFailure {
    BackendFailure::new(BACKEND_ID, BackendFailureKind::Internal, 1)
}

fn debug_error(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}
