//! Deterministic fake-backend coverage for the worker-owned generation kernel.

use std::collections::{BTreeMap, BTreeSet};
use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use domain_contracts::{
    BackendFailure, BackendFailureKind, BackendId, BackendLoadFailure, BackendSequence,
    CapabilitySet, DecodeBufferRequirements, DecodeInput, DecodeOutcome, DeviceId, DeviceKind,
    DrainTimeout, ExecutionDevice, FailedLoad, FailedLoadOwner, FinishReason, LoadConfiguration,
    LoadError, LoadPlan, LoadedModel, MemoryBudget, MemoryFootprint, ModelArchitecture,
    ModelCapabilities, ModelDescriptor, ModelError, ModelGeneration, ModelHandle, ModelId,
    ModelLoader, ModelMetadata, PrefillBufferRequirements, PrefillInput, PrefillOutcome,
    PreparedDecodeBuffers, PreparedLoad, PreparedPrefillBuffers, QuantizationFormat, RequestId,
    ScalarType, ScalarTypeSet, SequenceConfiguration, SequenceError, SequenceId, SequencePlan,
    SequenceReservation, SequenceState, SynchronizationError, TokenId, UnloadPolicy,
};
use host_runtime::TokenOutputRecordKind;
use inference_runtime::{
    CleanupPoll, CleanupResource, CommandTicket, FailureClass, GenerationOutcome,
    GenerationOutputCapacityPolicy, GenerationOutputState, GenerationRequest,
    GenerationStopSequence, HostedRuntime, HostedRuntimeConfiguration, InferenceRuntime,
    RetainedOwnership, RuntimeCommand, RuntimeError, RuntimeEvent, RuntimeLimits, RuntimeThread,
    start_hosted_runtime,
};
use sampling::SamplingConfig;

const BACKEND: BackendId = BackendId::new(93);
const MODEL: ModelId = ModelId::new(1);
const MODEL_HOST_BYTES: u64 = 100;
const SEQUENCE_HOST_BYTES: u64 = 32;

const fn cpu_device() -> ExecutionDevice {
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu)
}
const GENERATION_OPERATIONS: CapabilitySet = CapabilitySet::PREFILL
    .union(CapabilitySet::INCREMENTAL_DECODE)
    .union(CapabilitySet::MULTIPLE_SEQUENCES)
    .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);

type TestResult<T = ()> = Result<T, String>;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct ContractFaults(u16);

impl ContractFaults {
    const EMPTY: Self = Self(0);
    const SHORT_PREFILL_LOGITS: Self = Self(1 << 0);
    const SHORT_DECODE_LOGITS: Self = Self(1 << 1);
    const INVALID_PREFILL_POSITION: Self = Self(1 << 2);
    const INVALID_DECODE_POSITION: Self = Self(1 << 3);
    const INVALID_CONSUMED_TOKENS: Self = Self(1 << 4);
    const PREFILL_INVALID_STATE: Self = Self(1 << 5);
    const PREFILL_MUTATED_IDENTITY: Self = Self(1 << 6);
    const PREFILL_MUTATED_CAPACITY: Self = Self(1 << 7);
    const DECODE_INVALID_STATE: Self = Self(1 << 8);
    const DECODE_MUTATED_IDENTITY: Self = Self(1 << 9);
    const DECODE_MUTATED_CAPACITY: Self = Self(1 << 10);

    const fn contains(self, fault: Self) -> bool {
        self.0 & fault.0 != 0
    }
}
type HostedParts = (
    HostedRuntime<FakeSource>,
    RuntimeThread,
    Arc<Mutex<Counters>>,
    ModelHandle,
);
type SynchronousParts = (
    InferenceRuntime<FakeLoader>,
    Arc<Mutex<Counters>>,
    ModelHandle,
);

#[derive(Clone)]
struct FakeSource {
    script: [u32; 8],
    script_len: usize,
    uniform_logits: bool,
    no_candidate: bool,
    fail_prefill: bool,
    fail_decode_call: Option<u32>,
    failed_load_cleanup_failures: Option<u32>,
    destroy_failures: u32,
    unload_failures: u32,
    logits_capacity: usize,
    operations: CapabilitySet,
    contract_faults: ContractFaults,
    load_gate: Option<Arc<BlockingGate>>,
    prefill_gate: Option<Arc<BlockingGate>>,
}

impl FakeSource {
    const fn scripted(script: [u32; 8], script_len: usize) -> Self {
        Self {
            script,
            script_len,
            uniform_logits: false,
            no_candidate: false,
            fail_prefill: false,
            fail_decode_call: None,
            failed_load_cleanup_failures: None,
            destroy_failures: 0,
            unload_failures: 0,
            logits_capacity: 4,
            operations: GENERATION_OPERATIONS,
            contract_faults: ContractFaults::EMPTY,
            load_gate: None,
            prefill_gate: None,
        }
    }
}

struct BlockingGate {
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

fn blocking_gate() -> (Arc<BlockingGate>, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    (
        Arc::new(BlockingGate {
            entered: entered_sender,
            release: Mutex::new(release_receiver),
        }),
        entered_receiver,
        release_sender,
    )
}

#[derive(Default)]
struct Counters {
    loads: u32,
    unload_attempts: u32,
    sequence_creations: u32,
    destruction_attempts: u32,
    successful_destructions: u32,
    prefill_calls: u32,
    decode_calls: u32,
    sampling_opportunities: u32,
    active_sequences: u32,
    retained_memory_bytes: u64,
    failed_load_cleanup_attempts: u32,
    successful_failed_load_cleanups: u32,
    prepared_drops: u32,
    retained_prepared_drops: u32,
}

#[derive(Clone)]
struct FakeLoader {
    counters: Arc<Mutex<Counters>>,
}

struct FakePrepared {
    plan: LoadPlan,
    source: FakeSource,
    configuration: LoadConfiguration,
    counters: Arc<Mutex<Counters>>,
    remaining_failed_load_cleanup_failures: u32,
    partial_resources_retained: bool,
}

impl PreparedLoad for FakePrepared {
    type Failed = FakePrepared;

    fn plan(&self) -> &LoadPlan {
        &self.plan
    }
}

impl FailedLoadOwner for FakePrepared {
    fn plan(&self) -> &LoadPlan {
        &self.plan
    }

    fn cleanup(&mut self) -> Result<(), SynchronizationError> {
        let mut counters = self
            .counters
            .lock()
            .map_err(|_| SynchronizationError::Backend(failure(16)))?;
        counters.failed_load_cleanup_attempts =
            counters.failed_load_cleanup_attempts.saturating_add(1);
        if self.remaining_failed_load_cleanup_failures > 0 {
            self.remaining_failed_load_cleanup_failures = self
                .remaining_failed_load_cleanup_failures
                .saturating_sub(1);
            return Err(SynchronizationError::Backend(failure(17)));
        }
        if self.partial_resources_retained {
            self.partial_resources_retained = false;
            counters.successful_failed_load_cleanups =
                counters.successful_failed_load_cleanups.saturating_add(1);
            counters.retained_memory_bytes = counters
                .retained_memory_bytes
                .saturating_sub(MODEL_HOST_BYTES);
        }
        Ok(())
    }
}

impl Drop for FakePrepared {
    fn drop(&mut self) {
        if let Ok(mut counters) = self.counters.lock() {
            counters.prepared_drops = counters.prepared_drops.saturating_add(1);
            if self.partial_resources_retained {
                counters.retained_prepared_drops =
                    counters.retained_prepared_drops.saturating_add(1);
            }
        }
    }
}

struct FakeModel {
    handle: ModelHandle,
    execution_device: ExecutionDevice,
    execution_scalar_type: ScalarType,
    descriptor: ModelDescriptor,
    reported_footprint: MemoryFootprint,
    source: FakeSource,
    counters: Arc<Mutex<Counters>>,
    remaining_destroy_failures: u32,
    remaining_unload_failures: u32,
    model_released: bool,
}

struct FakeSequence {
    id: SequenceId,
    state: SequenceState,
    position: usize,
    capacity: usize,
    generated: usize,
    plan: SequencePlan,
}

impl BackendSequence for FakeSequence {
    fn id(&self) -> SequenceId {
        self.id
    }

    fn state(&self) -> SequenceState {
        self.state
    }

    fn position(&self) -> usize {
        self.position
    }

    fn token_capacity(&self) -> usize {
        self.capacity
    }

    fn reported_plan(&self) -> SequencePlan {
        self.plan
    }
}

impl ModelLoader for FakeLoader {
    type Source = FakeSource;
    type Prepared = FakePrepared;
    type FailedPreparation = FakePrepared;
    type Model = FakeModel;

    fn inspect(&self, source: &Self::Source) -> Result<ModelDescriptor, LoadError> {
        Ok(descriptor(source.operations))
    }

    fn prepare_load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Prepared, LoadError> {
        Ok(FakePrepared {
            plan: LoadPlan {
                accepted_configuration: *configuration,
                descriptor: self.inspect(source)?,
                execution_scalar_type: ScalarType::F32,
                final_footprint: model_footprint(),
                loading_peak_footprint: model_footprint(),
            },
            source: source.clone(),
            configuration: *configuration,
            counters: Arc::clone(&self.counters),
            remaining_failed_load_cleanup_failures: source
                .failed_load_cleanup_failures
                .unwrap_or(0),
            partial_resources_retained: false,
        })
    }

    fn load_prepared(
        &mut self,
        mut prepared: Self::Prepared,
    ) -> Result<Self::Model, FailedLoad<Self::FailedPreparation>> {
        if prepared.source.failed_load_cleanup_failures.is_some() {
            let retained_bytes = MODEL_HOST_BYTES;
            let retained = (|| {
                let mut counters = self
                    .counters
                    .lock()
                    .map_err(|_| LoadError::Backend(BackendLoadFailure::new(failure(18))))?;
                counters.loads = counters.loads.saturating_add(1);
                counters.retained_memory_bytes = counters
                    .retained_memory_bytes
                    .saturating_add(retained_bytes);
                Ok::<(), LoadError>(())
            })();
            if let Err(primary) = retained {
                return Err(FailedLoad::new(primary, prepared));
            }
            prepared.partial_resources_retained = true;
            return Err(FailedLoad::new(
                LoadError::Backend(BackendLoadFailure::new(failure(19))),
                prepared,
            ));
        }

        let load_attempt = (|| {
            if let Some(gate) = &prepared.source.load_gate {
                gate.entered
                    .send(())
                    .map_err(|_| LoadError::Backend(BackendLoadFailure::new(failure(13))))?;
                gate.release
                    .lock()
                    .map_err(|_| LoadError::Backend(BackendLoadFailure::new(failure(14))))?
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|_| LoadError::Backend(BackendLoadFailure::new(failure(15))))?;
            }
            let mut counters = self
                .counters
                .lock()
                .map_err(|_| LoadError::Backend(BackendLoadFailure::new(failure(1))))?;
            counters.loads = counters.loads.saturating_add(1);
            counters.retained_memory_bytes = counters
                .retained_memory_bytes
                .saturating_add(MODEL_HOST_BYTES);
            Ok::<(), LoadError>(())
        })();
        if let Err(primary) = load_attempt {
            return Err(FailedLoad::new(primary, prepared));
        }

        let remaining_destroy_failures = prepared.source.destroy_failures;
        let remaining_unload_failures = prepared.source.unload_failures;
        Ok(FakeModel {
            handle: prepared.configuration.handle,
            execution_device: prepared.configuration.execution_device,
            execution_scalar_type: ScalarType::F32,
            descriptor: prepared.plan.descriptor,
            reported_footprint: prepared.plan.final_footprint,
            source: prepared.source.clone(),
            counters: Arc::clone(&self.counters),
            remaining_destroy_failures,
            remaining_unload_failures,
            model_released: false,
        })
    }
}

impl LoadedModel for FakeModel {
    type Sequence = FakeSequence;

    fn handle(&self) -> ModelHandle {
        self.handle
    }

    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn execution_device(&self) -> ExecutionDevice {
        self.execution_device
    }

    fn execution_scalar_type(&self) -> ScalarType {
        self.execution_scalar_type
    }

    fn reported_footprint(&self) -> MemoryFootprint {
        self.reported_footprint
    }

    fn plan_sequence(
        &self,
        configuration: &SequenceConfiguration,
    ) -> Result<SequencePlan, ModelError> {
        Ok(SequencePlan {
            configuration: *configuration,
            reservation: SequenceReservation {
                persistent_footprint: sequence_footprint(),
                transient_footprint: MemoryFootprint::default(),
                total_footprint: sequence_footprint(),
            },
            logits_capacity: self.source.logits_capacity,
        })
    }

    fn create_sequence(
        &mut self,
        sequence_id: SequenceId,
        configuration: &SequenceConfiguration,
    ) -> Result<Self::Sequence, ModelError> {
        let plan = self.plan_sequence(configuration)?;
        let mut counters = self
            .counters
            .lock()
            .map_err(|_| ModelError::Backend(failure(2)))?;
        counters.sequence_creations = counters.sequence_creations.saturating_add(1);
        counters.active_sequences = counters.active_sequences.saturating_add(1);
        counters.retained_memory_bytes = counters
            .retained_memory_bytes
            .saturating_add(SEQUENCE_HOST_BYTES);
        drop(counters);
        Ok(FakeSequence {
            id: sequence_id,
            state: SequenceState::Empty,
            position: 0,
            capacity: configuration.maximum_tokens.get() as usize,
            generated: 0,
            plan,
        })
    }

    fn prefill_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        _input: &PrefillInput<'_>,
    ) -> PrefillBufferRequirements {
        PrefillBufferRequirements { logits: 4 }
    }

    fn decode_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        _input: DecodeInput,
    ) -> DecodeBufferRequirements {
        DecodeBufferRequirements { logits: 4 }
    }

    fn prefill_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        input: PrefillInput<'_>,
        mut buffers: PreparedPrefillBuffers<'_>,
    ) -> Result<PrefillOutcome, SequenceError> {
        if let Some(gate) = &self.source.prefill_gate {
            gate.entered
                .send(())
                .map_err(|_| SequenceError::Backend(failure(16)))?;
            gate.release
                .lock()
                .map_err(|_| SequenceError::Backend(failure(17)))?
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| SequenceError::Backend(failure(18)))?;
        }
        self.counters
            .lock()
            .map_err(|_| SequenceError::Backend(failure(3)))?
            .prefill_calls += 1;
        if self.source.fail_prefill {
            return Err(SequenceError::Backend(failure(4)));
        }
        let consumed_position = if self
            .source
            .contract_faults
            .contains(ContractFaults::INVALID_PREFILL_POSITION)
        {
            input.tokens.len().saturating_add(1)
        } else {
            input.tokens.len()
        };
        sequence.position = sequence.position.saturating_add(consumed_position);
        sequence.state = SequenceState::Ready;
        if self
            .source
            .contract_faults
            .contains(ContractFaults::PREFILL_MUTATED_IDENTITY)
        {
            sequence.id = SequenceId::new(9_999);
        }
        if self
            .source
            .contract_faults
            .contains(ContractFaults::PREFILL_MUTATED_CAPACITY)
        {
            sequence.capacity = sequence.capacity.saturating_add(1);
        }
        if self
            .source
            .contract_faults
            .contains(ContractFaults::PREFILL_INVALID_STATE)
        {
            sequence.state = SequenceState::Transitioning;
        }
        write_logits(&self.source, sequence.generated, buffers.logits_mut());
        self.counters
            .lock()
            .map_err(|_| SequenceError::Backend(failure(10)))?
            .sampling_opportunities += 1;
        Ok(PrefillOutcome::Ready {
            consumed_tokens: if self
                .source
                .contract_faults
                .contains(ContractFaults::INVALID_CONSUMED_TOKENS)
            {
                input.tokens.len().saturating_add(1)
            } else {
                input.tokens.len()
            },
            position: sequence.position,
            logits_written: if self
                .source
                .contract_faults
                .contains(ContractFaults::SHORT_PREFILL_LOGITS)
            {
                buffers.required_logits().saturating_sub(1)
            } else {
                buffers.required_logits()
            },
        })
    }

    fn decode_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        _input: DecodeInput,
        mut buffers: PreparedDecodeBuffers<'_>,
    ) -> Result<DecodeOutcome, SequenceError> {
        let call = {
            let mut counters = self
                .counters
                .lock()
                .map_err(|_| SequenceError::Backend(failure(5)))?;
            counters.decode_calls = counters.decode_calls.saturating_add(1);
            counters.decode_calls
        };
        if self.source.fail_decode_call == Some(call) {
            return Err(SequenceError::Backend(failure(6)));
        }
        sequence.position = sequence.position.saturating_add(
            if self
                .source
                .contract_faults
                .contains(ContractFaults::INVALID_DECODE_POSITION)
            {
                2
            } else {
                1
            },
        );
        sequence.generated = sequence.generated.saturating_add(1);
        if self
            .source
            .contract_faults
            .contains(ContractFaults::DECODE_MUTATED_IDENTITY)
        {
            sequence.id = SequenceId::new(9_999);
        }
        if self
            .source
            .contract_faults
            .contains(ContractFaults::DECODE_MUTATED_CAPACITY)
        {
            sequence.capacity = sequence.capacity.saturating_add(1);
        }
        if self
            .source
            .contract_faults
            .contains(ContractFaults::DECODE_INVALID_STATE)
        {
            sequence.state = SequenceState::Transitioning;
        }
        write_logits(&self.source, sequence.generated, buffers.logits_mut());
        self.counters
            .lock()
            .map_err(|_| SequenceError::Backend(failure(11)))?
            .sampling_opportunities += 1;
        Ok(DecodeOutcome::Ready {
            position: sequence.position,
            logits_written: if self
                .source
                .contract_faults
                .contains(ContractFaults::SHORT_DECODE_LOGITS)
            {
                buffers.required_logits().saturating_sub(1)
            } else {
                buffers.required_logits()
            },
        })
    }

    fn destroy_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        let mut counters = self
            .counters
            .lock()
            .map_err(|_| SequenceError::Backend(failure(7)))?;
        counters.destruction_attempts = counters.destruction_attempts.saturating_add(1);
        if self.remaining_destroy_failures > 0 {
            self.remaining_destroy_failures = self.remaining_destroy_failures.saturating_sub(1);
            return Err(SequenceError::Backend(failure(8)));
        }
        if sequence.state != SequenceState::Finished {
            sequence.state = SequenceState::Finished;
            counters.successful_destructions = counters.successful_destructions.saturating_add(1);
            counters.active_sequences = counters.active_sequences.saturating_sub(1);
            counters.retained_memory_bytes = counters
                .retained_memory_bytes
                .saturating_sub(SEQUENCE_HOST_BYTES);
        }
        drop(counters);
        Ok(())
    }

    fn reset_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        sequence.state = SequenceState::Empty;
        sequence.position = 0;
        sequence.generated = 0;
        Ok(())
    }

    fn synchronize(&mut self) -> Result<(), SynchronizationError> {
        Ok(())
    }

    fn prepare_unload(&mut self) -> Result<(), SynchronizationError> {
        let mut counters = self
            .counters
            .lock()
            .map_err(|_| SynchronizationError::Backend(failure(9)))?;

        counters.unload_attempts = counters.unload_attempts.saturating_add(1);

        if self.remaining_unload_failures > 0 {
            self.remaining_unload_failures = self.remaining_unload_failures.saturating_sub(1);
            drop(counters);
            return Err(SynchronizationError::Backend(failure(12)));
        }

        if !self.model_released {
            self.model_released = true;
            counters.retained_memory_bytes = counters
                .retained_memory_bytes
                .saturating_sub(MODEL_HOST_BYTES);
        }

        drop(counters);
        Ok(())
    }
}

#[test]
fn greedy_generation_prefills_once_decodes_and_finishes_on_eos() -> TestResult {
    let source = FakeSource::scripted([1, 2, 3, 0, 0, 0, 0, 0], 3);
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(
        &hosted,
        handle,
        request(10, 100, 8, &[TokenId::new(3)], &[]),
    )?;
    let output = collect_until_released(&hosted, RequestId::new(10), Duration::from_secs(2))?;
    assert_eq!(
        output.tokens,
        vec![TokenId::new(1), TokenId::new(2), TokenId::new(3)]
    );
    assert!(output.states.contains(&GenerationOutputState::Terminal(
        GenerationOutcome::Finished(FinishReason::EndOfSequence(TokenId::new(3)))
    )));
    let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
    assert_eq!(counters.prefill_calls, 1);
    assert_eq!(counters.decode_calls, 2);
    assert_eq!(counters.successful_destructions, 1);
    drop(counters);
    shutdown(hosted, thread)
}

#[test]
fn token_limit_stop_sequence_and_seeded_sampling_are_deterministic() -> TestResult {
    let scripted = FakeSource::scripted([1, 2, 3, 0, 0, 0, 0, 0], 3);
    let (hosted, thread, _, handle) = hosted(scripted, 8, 16)?;
    let stop = GenerationStopSequence {
        code: 7,
        tokens: vec![TokenId::new(1), TokenId::new(2)].into_boxed_slice(),
    };
    submit_generation(&hosted, handle, request(20, 200, 6, &[], &[stop]))?;
    let stopped = collect_until_released(&hosted, RequestId::new(20), Duration::from_secs(2))?;
    assert_eq!(stopped.tokens, vec![TokenId::new(1), TokenId::new(2)]);
    assert!(stopped.states.contains(&GenerationOutputState::Terminal(
        GenerationOutcome::Finished(FinishReason::StopCondition)
    )));
    shutdown(hosted, thread)?;

    let first = stochastic_run(55)?;
    let second = stochastic_run(55)?;
    assert_eq!(first, second);
    assert_eq!(first.len(), 5);
    Ok(())
}

#[test]
fn backpressure_resumes_without_duplicate_tokens_and_cancellation_stays_responsive() -> TestResult {
    let source = FakeSource::scripted([1, 2, 3, 1, 2, 3, 1, 2], 8);
    let (hosted, thread, counters, handle) = hosted(source, 1, 4)?;
    submit_generation(&hosted, handle, request(30, 300, 8, &[], &[]))?;
    std::thread::sleep(Duration::from_millis(20));
    hosted
        .try_submit(RuntimeCommand::CancelRequest {
            ticket: CommandTicket::new(4),
            request_id: RequestId::new(30),
            reason: domain_contracts::CancellationReason::UserRequested,
        })
        .map_err(|_| "cancel command rejected")?;
    let cancellation_event = hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("cancel event: {error:?}"))?;
    assert!(matches!(
        cancellation_event,
        RuntimeEvent::GenerationCancellationRequested { result: Ok(()), .. }
    ));
    let output = collect_until_released(&hosted, RequestId::new(30), Duration::from_secs(2))?;
    assert!(output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::Yielded(domain_contracts::YieldReason::OutputBackpressure(_))
    )));
    assert!(output.states.contains(&GenerationOutputState::Terminal(
        GenerationOutcome::Finished(FinishReason::Cancelled(
            domain_contracts::CancellationReason::UserRequested
        ))
    )));
    let mut deduplicated = output.tokens.clone();
    deduplicated.dedup();
    assert_eq!(deduplicated, output.tokens);
    assert!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .decode_calls
            <= 1
    );
    shutdown(hosted, thread)
}

#[test]
fn shutdown_terminates_without_draining_backpressured_output() -> TestResult {
    let source = FakeSource::scripted([1, 2, 3, 1, 2, 3, 1, 2], 8);
    let (hosted, thread, counters, handle) = hosted(source, 1, 1)?;
    submit_generation(&hosted, handle, request(31, 301, 8, &[], &[]))?;

    let deadline = Instant::now()
        .checked_add(Duration::from_secs(2))
        .ok_or("backpressure deadline overflow")?;
    loop {
        let sampling_opportunities = counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .sampling_opportunities;
        if sampling_opportunities >= 2 {
            break;
        }
        if Instant::now() >= deadline {
            return Err("generation did not reach output backpressure".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(5),
        })
        .map_err(|_| "shutdown command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("shutdown event: {error:?}"))?
    {
        RuntimeEvent::Shutdown { result: Ok(_), .. } => {}
        _ => return Err("unexpected shutdown event".into()),
    }
    thread.join().map_err(|error| error.to_string())?;

    let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
    assert_eq!(counters.successful_destructions, 1);
    assert_eq!(counters.active_sequences, 0);
    assert_eq!(counters.retained_memory_bytes, 0);
    drop(counters);

    Ok(())
}

#[test]
fn exhausted_failed_preparation_is_retained_until_process_exit() -> TestResult {
    let mut source = FakeSource::scripted([0; 8], 0);
    source.failed_load_cleanup_failures = Some(u32::MAX);
    let (hosted, thread, counters) = start_hosted(1, 8, NonZeroU32::MIN, NonZeroU32::MIN, 10_000)?;
    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(60),
            model_id: MODEL,
            source,
            execution_device: cpu_device(),
        })
        .map_err(|_| "failed-load command rejected")?;
    assert!(matches!(
        hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("failed-load event: {error:?}"))?,
        RuntimeEvent::ModelLoaded {
            result: Err(RuntimeError::CleanupFailed(_)),
            ..
        }
    ));

    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(61),
        })
        .map_err(|_| "failed-load shutdown command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("failed-load shutdown event: {error:?}"))?
    {
        RuntimeEvent::Shutdown {
            result: Err(RuntimeError::TerminalCleanupRetention { first, summary }),
            ..
        } => {
            assert_eq!(
                first.resource,
                CleanupResource::FailedLoad {
                    handle: ModelHandle::new(MODEL, ModelGeneration::new(1)),
                }
            );
            assert_eq!(first.ownership, RetainedOwnership::Exact(model_footprint()));
            assert_eq!(summary.failed_preparations, 1);
            assert_eq!(summary.verified_models, 0);
            assert_eq!(summary.incompatible_models, 0);
            assert_eq!(summary.sequences, 0);
        }
        _ => return Err("unexpected failed-load shutdown event".into()),
    }
    drop(hosted);
    thread.join().map_err(|error| error.to_string())?;

    let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
    assert_eq!(counters.loads, 1);
    assert_eq!(counters.failed_load_cleanup_attempts, 3);
    assert_eq!(counters.successful_failed_load_cleanups, 0);
    assert_eq!(counters.retained_memory_bytes, MODEL_HOST_BYTES);
    assert_eq!(counters.prepared_drops, 0);
    assert_eq!(counters.retained_prepared_drops, 0);
    drop(counters);
    Ok(())
}

#[test]
fn backend_failure_and_cleanup_retry_preserve_both_terminal_states() -> TestResult {
    let mut source = FakeSource::scripted([1, 2, 3, 0, 0, 0, 0, 0], 3);
    source.fail_prefill = true;
    source.destroy_failures = 2;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(&hosted, handle, request(40, 400, 4, &[], &[]))?;
    let output = collect_until_released(&hosted, RequestId::new(40), Duration::from_secs(2))?;
    assert!(output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::Terminal(GenerationOutcome::Failed(
            inference_runtime::RuntimeError::Sequence(SequenceError::Backend(_))
        ))
    )));
    assert!(output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::CleanupPending { failure, .. }
            if failure.primary_failure == inference_runtime::FailureClass::Sequence
                && failure.cleanup_failure == inference_runtime::FailureClass::Sequence
    )));
    assert!(
        output
            .states
            .iter()
            .any(|state| matches!(state, GenerationOutputState::Released(_)))
    );
    let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
    assert_eq!(counters.destruction_attempts, 3);
    assert_eq!(counters.successful_destructions, 1);
    assert_eq!(counters.active_sequences, 0);
    assert_eq!(counters.retained_memory_bytes, MODEL_HOST_BYTES);
    drop(counters);
    shutdown(hosted, thread)
}

#[test]
fn exhausted_sequence_cleanup_is_published_after_terminal_and_pending() -> TestResult {
    let mut source = FakeSource::scripted([1; 8], 8);
    source.destroy_failures = u32::MAX;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(&hosted, handle, request(190, 1_900, 1, &[], &[]))?;

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut states = Vec::new();
    while !states
        .iter()
        .any(|state| matches!(state, GenerationOutputState::CleanupExhausted { .. }))
    {
        hosted
            .pull_token_output(|batch| {
                states.extend(
                    batch
                        .records
                        .iter()
                        .filter_map(|record| {
                            (record.request_id == RequestId::new(190)).then_some(record.kind)
                        })
                        .filter_map(|kind| match kind {
                            TokenOutputRecordKind::State(state) => Some(state),
                            TokenOutputRecordKind::Tokens(_) => None,
                        }),
                );
            })
            .map_err(|error| format!("token pull failed: {error:?}"))?;
        if Instant::now() >= deadline {
            return Err(format!("cleanup exhaustion timed out: {states:?}"));
        }
        std::thread::yield_now();
    }
    let terminal = states
        .iter()
        .position(|state| matches!(state, GenerationOutputState::Terminal(_)))
        .ok_or("missing terminal state")?;
    let pending = states
        .iter()
        .position(|state| matches!(state, GenerationOutputState::CleanupPending { .. }))
        .ok_or("missing cleanup-pending state")?;
    let exhausted = states
        .iter()
        .position(|state| matches!(state, GenerationOutputState::CleanupExhausted { .. }))
        .ok_or("missing cleanup-exhausted state")?;
    assert!(terminal < pending && pending < exhausted);
    assert!(
        !states
            .iter()
            .any(|state| matches!(state, GenerationOutputState::Released(_)))
    );
    assert_eq!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .destruction_attempts,
        3
    );

    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(191),
        })
        .map_err(|_| "shutdown command rejected")?;
    assert!(matches!(
        hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("shutdown event failed: {error:?}"))?,
        RuntimeEvent::Shutdown {
            result: Err(RuntimeError::TerminalCleanupRetention { .. }),
            ..
        }
    ));
    drop(hosted);
    thread.join().map_err(|error| error.to_string())
}

#[test]
fn concurrent_runnable_requests_complete_without_starvation() -> TestResult {
    let source = FakeSource::scripted([1, 2, 3, 0, 0, 0, 0, 0], 3);
    let (hosted, thread, _, handle) = hosted(source, 32, 64)?;
    let request_ids = [RequestId::new(70), RequestId::new(71)];
    for (ticket, request_id, sequence_id) in [(70, 70, 700), (71, 71, 701)] {
        hosted
            .try_submit(RuntimeCommand::Generate {
                ticket: CommandTicket::new(ticket),
                handle,
                request: request(request_id, sequence_id, 3, &[], &[]),
            })
            .map_err(|_| "fairness generation command rejected")?;
    }
    for _ in 0..2 {
        assert!(matches!(
            hosted
                .receive_timeout(Duration::from_secs(2))
                .map_err(|error| format!("fairness admission event: {error:?}"))?,
            RuntimeEvent::GenerationAdmitted { result: Ok(_), .. }
        ));
    }

    let outputs = collect_until_all_released(&hosted, &request_ids, Duration::from_secs(2))?;
    for request_id in request_ids {
        let output = outputs
            .get(&request_id)
            .ok_or("missing concurrent generation output")?;
        assert_eq!(output.tokens.len(), 3);
        assert!(output.states.iter().any(|state| matches!(
            state,
            GenerationOutputState::Terminal(GenerationOutcome::Finished(FinishReason::TokenLimit))
        )));
    }
    shutdown(hosted, thread)
}

#[test]
fn decode_sampling_and_generation_capacity_failures_are_stable() -> TestResult {
    let mut decode_source = FakeSource::scripted([1, 2, 3, 0, 0, 0, 0, 0], 3);
    decode_source.fail_decode_call = Some(1);
    let (hosted, thread, _, handle) = hosted(decode_source, 8, 16)?;
    submit_generation(&hosted, handle, request(60, 600, 4, &[], &[]))?;
    let decoded = collect_until_released(&hosted, RequestId::new(60), Duration::from_secs(2))?;
    assert_eq!(decoded.tokens, vec![TokenId::new(1)]);
    assert!(decoded.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::Terminal(GenerationOutcome::Failed(
            inference_runtime::RuntimeError::Sequence(_)
        ))
    )));
    shutdown(hosted, thread)?;

    let mut sampling_source = FakeSource::scripted([0; 8], 0);
    sampling_source.no_candidate = true;
    let (hosted, thread, _, handle) = self::hosted(sampling_source, 8, 16)?;
    submit_generation(&hosted, handle, request(61, 601, 4, &[], &[]))?;
    let sampled = collect_until_released(&hosted, RequestId::new(61), Duration::from_secs(2))?;
    assert!(sampled.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::Terminal(GenerationOutcome::Failed(
            inference_runtime::RuntimeError::Sampling(_)
        ))
    )));
    shutdown(hosted, thread)?;

    let source = FakeSource::scripted([1; 8], 8);
    let (hosted, thread, _, handle) = self::hosted(source, 8, 16)?;
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket: CommandTicket::new(62),
            handle,
            request: request(62, 602, 40, &[], &[]),
        })
        .map_err(|_| "capacity generation command rejected")?;
    assert!(matches!(
        hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("capacity admission event: {error:?}"))?,
        RuntimeEvent::GenerationAdmitted {
            result: Err(inference_runtime::RuntimeError::CapacityExhausted(_)),
            ..
        }
    ));
    shutdown(hosted, thread)
}

#[test]
fn generation_admission_rejects_oversized_prefill_before_native_creation() -> TestResult {
    let source = FakeSource::scripted([1; 8], 8);
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    let mut generation = request(80, 800, 4, &[], &[]);
    generation.prompt_tokens = vec![TokenId::new(0); 9].into_boxed_slice();

    let error = submit_generation_error(&hosted, handle, generation, CommandTicket::new(80))?;
    assert!(matches!(
        error,
        inference_runtime::RuntimeError::CapacityExhausted(_)
    ));
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.sequence_creations, 0);
        assert_eq!(counters.retained_memory_bytes, MODEL_HOST_BYTES);
        drop(counters);
    }
    shutdown(hosted, thread)?;
    assert_eq!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .retained_memory_bytes,
        0
    );
    Ok(())
}

#[test]
fn generation_admission_rejects_insufficient_output_capacity_before_native_creation() -> TestResult
{
    let source = FakeSource::scripted([1; 8], 8);
    let (hosted, thread, counters, handle) = hosted(source, 1, 4)?;
    let mut generation = request(81, 801, 4, &[], &[]);
    generation.output_capacity =
        GenerationOutputCapacityPolicy::new(nonzero_usize(2)?, NonZeroUsize::MIN);

    let error = submit_generation_error(&hosted, handle, generation, CommandTicket::new(81))?;
    assert!(matches!(
        error,
        inference_runtime::RuntimeError::CapacityExhausted(_)
    ));
    assert_eq!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .sequence_creations,
        0
    );
    shutdown(hosted, thread)
}

#[test]
fn generation_workspace_bytes_are_admitted_before_native_sequence_creation() -> TestResult {
    let source = FakeSource::scripted([1; 8], 8);
    let (hosted, thread, counters, handle) = hosted_with_budget(source, 8, 16, 219)?;

    let error = submit_generation_error(
        &hosted,
        handle,
        request(82, 802, 4, &[], &[]),
        CommandTicket::new(82),
    )?;
    assert!(matches!(
        error,
        inference_runtime::RuntimeError::InsufficientMemory {
            kind: inference_runtime::MemoryKind::Host,
            required_bytes: 220,
            available_bytes: 219,
        }
    ));
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.sequence_creations, 0);
        assert_eq!(counters.retained_memory_bytes, MODEL_HOST_BYTES);
        drop(counters);
    }
    shutdown(hosted, thread)
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the hosted workspace-retention scenario keeps ordered terminal and snapshot evidence together"
)]
fn generation_workspace_accounting_is_retained_until_terminal_output_release() -> TestResult {
    let source = FakeSource::scripted([1; 8], 8);
    let (hosted, thread, counters, handle) = hosted(source, 1, 1)?;
    submit_generation(&hosted, handle, request(89, 809, 1, &[], &[]))?;

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .successful_destructions
            == 1
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err("sequence cleanup did not complete".into());
        }
        std::thread::yield_now();
    }

    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(90),
        })
        .map_err(|_| "snapshot command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot {
            runtime,
            retained_models,
            ..
        } => {
            assert_eq!(runtime.active_requests, 0);
            assert_eq!(runtime.generation_workspaces, 1);
            assert_eq!(
                runtime.reserved_generation_workspace,
                MemoryFootprint {
                    host_weight_bytes: 0,
                    device_weight_bytes: 0,
                    host_working_bytes: 64,
                    device_working_bytes: 0,
                }
            );
            assert_eq!(
                runtime.reserved_footprint,
                MemoryFootprint {
                    host_weight_bytes: MODEL_HOST_BYTES,
                    device_weight_bytes: 0,
                    host_working_bytes: 64,
                    device_working_bytes: 0,
                }
            );
            assert!(runtime.unverified_ownership.is_none());
            assert!(!runtime.admission_blocked);
            assert!(retained_models.is_empty());
        }
        _ => return Err("unexpected snapshot event".into()),
    }

    hosted
        .try_submit(RuntimeCommand::StartRequest {
            ticket: CommandTicket::new(92),
            handle,
            request_id: RequestId::new(89),
            sequence_id: SequenceId::new(899),
            configuration: SequenceConfiguration::new(
                NonZeroU32::new(32).unwrap_or(NonZeroU32::MIN),
                NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
            ),
        })
        .map_err(|_| "request reuse command rejected")?;
    assert!(matches!(
        hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("request reuse event failed: {error:?}"))?,
        RuntimeEvent::RequestStarted {
            result: Err(inference_runtime::RuntimeError::RequestAlreadyActive(request_id)),
            ..
        } if request_id == RequestId::new(89)
    ));

    let output = collect_until_released(&hosted, RequestId::new(89), Duration::from_secs(2))?;
    assert_eq!(output.tokens, vec![TokenId::new(1)]);

    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(91),
        })
        .map_err(|_| "snapshot command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot {
            runtime,
            retained_models,
            ..
        } => {
            assert_eq!(runtime.generation_workspaces, 0);
            assert_eq!(
                runtime.reserved_generation_workspace,
                MemoryFootprint::default()
            );
            assert_eq!(runtime.reserved_footprint, model_footprint());
            assert!(runtime.unverified_ownership.is_none());
            assert!(!runtime.admission_blocked);
            assert!(retained_models.is_empty());
        }
        _ => return Err("unexpected snapshot event".into()),
    }
    shutdown(hosted, thread)
}

#[test]
fn generation_admission_requires_exact_full_vocabulary_logits() -> TestResult {
    let mut source = FakeSource::scripted([1; 8], 8);
    source.logits_capacity = 3;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;

    let error = submit_generation_error(
        &hosted,
        handle,
        request(83, 803, 4, &[], &[]),
        CommandTicket::new(83),
    )?;
    assert_eq!(
        error,
        inference_runtime::RuntimeError::BackendContractViolation
    );
    assert_eq!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .sequence_creations,
        0
    );
    shutdown(hosted, thread)
}

#[test]
fn scheduled_generation_requires_prefill_and_incremental_decode_capabilities() -> TestResult {
    let common = CapabilitySet::MULTIPLE_SEQUENCES.union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);
    for (operations, request_id) in [
        (common.union(CapabilitySet::INCREMENTAL_DECODE), 83),
        (common.union(CapabilitySet::PREFILL), 84),
    ] {
        let mut source = FakeSource::scripted([1; 8], 8);
        source.operations = operations;
        let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;

        let error = submit_generation_error(
            &hosted,
            handle,
            request(request_id, request_id + 1_000, 4, &[], &[]),
            CommandTicket::new(request_id),
        )?;
        assert_eq!(error, RuntimeError::Model(ModelError::Unsupported));
        assert_eq!(
            counters
                .lock()
                .map_err(|_| "counter mutex poisoned")?
                .sequence_creations,
            0
        );
        shutdown(hosted, thread)?;
    }
    Ok(())
}

#[test]
fn scheduled_generation_configuration_respects_advertised_limits() -> TestResult {
    let source = FakeSource::scripted([1; 8], 8);
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    let configurations = [
        (
            85,
            SequenceConfiguration::new(
                NonZeroU32::new(65).unwrap_or(NonZeroU32::MIN),
                NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
            ),
        ),
        (
            86,
            SequenceConfiguration::new(
                NonZeroU32::new(32).unwrap_or(NonZeroU32::MIN),
                NonZeroU32::new(9).unwrap_or(NonZeroU32::MIN),
            ),
        ),
        (
            87,
            SequenceConfiguration::new(
                NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
                NonZeroU32::new(5).unwrap_or(NonZeroU32::MIN),
            ),
        ),
    ];

    for (request_id, configuration) in configurations {
        let mut generation = request(request_id, request_id + 1_000, 1, &[], &[]);
        generation.sequence = configuration;
        let error =
            submit_generation_error(&hosted, handle, generation, CommandTicket::new(request_id))?;
        assert_eq!(error, RuntimeError::Model(ModelError::Unsupported));
    }
    assert_eq!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .sequence_creations,
        0
    );
    shutdown(hosted, thread)
}

#[test]
fn direct_prefill_and_decode_require_advertised_capabilities() -> TestResult {
    let common = CapabilitySet::MULTIPLE_SEQUENCES.union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);

    let mut source = FakeSource::scripted([1; 8], 8);
    source.operations = common.union(CapabilitySet::INCREMENTAL_DECODE);
    let (mut runtime, counters, handle) = synchronous_runtime(&source)?;
    let request_id = RequestId::new(88);
    runtime
        .start_request(
            handle,
            request_id,
            SequenceId::new(888),
            direct_sequence_configuration(),
        )
        .map_err(debug_error)?;
    assert_eq!(
        runtime.prefill(request_id, &[TokenId::new(0)], true, &mut [0.0; 4]),
        Err(RuntimeError::Sequence(SequenceError::Unsupported))
    );
    assert!(!runtime.is_request_active(request_id));
    runtime.shutdown().map_err(debug_error)?;
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.prefill_calls, 0);
        assert_eq!(counters.successful_destructions, 1);
        assert_eq!(counters.retained_memory_bytes, 0);
        drop(counters);
    }

    let mut source = FakeSource::scripted([1; 8], 8);
    source.operations = common.union(CapabilitySet::PREFILL);
    let (mut runtime, counters, handle) = synchronous_runtime(&source)?;
    let request_id = RequestId::new(89);
    runtime
        .start_request(
            handle,
            request_id,
            SequenceId::new(889),
            direct_sequence_configuration(),
        )
        .map_err(debug_error)?;
    runtime
        .prefill(request_id, &[TokenId::new(0)], true, &mut [0.0; 4])
        .map_err(debug_error)?;
    assert_eq!(
        runtime.decode(request_id, TokenId::new(1), &mut [0.0; 4]),
        Err(RuntimeError::Sequence(SequenceError::Unsupported))
    );
    assert!(!runtime.is_request_active(request_id));
    runtime.shutdown().map_err(debug_error)?;
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.prefill_calls, 1);
        assert_eq!(counters.decode_calls, 0);
        assert_eq!(counters.successful_destructions, 1);
        assert_eq!(counters.retained_memory_bytes, 0);
        drop(counters);
    }
    Ok(())
}

#[test]
fn ready_results_preserve_sequence_state_identity_and_capacity() -> TestResult {
    let cases = [
        (ContractFaults::PREFILL_INVALID_STATE, false, false, 110),
        (ContractFaults::PREFILL_MUTATED_IDENTITY, false, true, 111),
        (ContractFaults::PREFILL_MUTATED_CAPACITY, false, false, 112),
        (ContractFaults::DECODE_INVALID_STATE, true, false, 113),
        (ContractFaults::DECODE_MUTATED_IDENTITY, true, false, 114),
        (ContractFaults::DECODE_MUTATED_CAPACITY, true, false, 115),
    ];

    for (fault, decode_fault, quarantine, value) in cases {
        let mut source = FakeSource::scripted([1; 8], 8);
        source.contract_faults = fault;
        source.destroy_failures = u32::from(quarantine);
        let (mut runtime, counters, handle) = synchronous_runtime(&source)?;
        let request_id = RequestId::new(value);
        let sequence_id = SequenceId::new(value + 1_000);
        runtime
            .start_request(
                handle,
                request_id,
                sequence_id,
                direct_sequence_configuration(),
            )
            .map_err(debug_error)?;
        let mut logits = [0.0; 4];
        if decode_fault {
            runtime
                .prefill(request_id, &[TokenId::new(0)], true, &mut logits)
                .map_err(debug_error)?;
        }
        let result = if decode_fault {
            runtime
                .decode(request_id, TokenId::new(1), &mut logits)
                .map(|_| ())
        } else {
            runtime
                .prefill(request_id, &[TokenId::new(0)], true, &mut logits)
                .map(|_| ())
        };
        assert_eq!(result, Err(RuntimeError::BackendContractViolation));
        assert!(!runtime.is_request_active(request_id));

        if quarantine {
            let cleanup = runtime
                .request_cleanup_state(request_id)
                .ok_or("missing quarantined sequence")?;
            assert_eq!(
                cleanup.failure.primary_failure,
                FailureClass::BackendContract
            );
            assert!(matches!(
                cleanup.resource,
                CleanupResource::Sequence {
                    handle: retained_handle,
                    request_id: retained_request,
                    sequence_id: retained_sequence,
                } if retained_handle == handle
                    && retained_request == request_id
                    && retained_sequence == sequence_id
            ));
            assert_eq!(
                cleanup.ownership,
                RetainedOwnership::Unverified {
                    accepted_footprint: sequence_footprint(),
                    reported_footprint: sequence_footprint(),
                    conservative_footprint: inference_runtime::ConservativeFootprint::Known(
                        sequence_footprint(),
                    ),
                }
            );
            assert!(matches!(
                runtime.poll_cleanup().map_err(debug_error)?,
                CleanupPoll::Released(_)
            ));
        } else {
            assert!(!runtime.is_request_cleanup_pending(request_id));
        }

        let reuse_request = RequestId::new(value + 2_000);
        runtime
            .start_request(
                handle,
                reuse_request,
                sequence_id,
                direct_sequence_configuration(),
            )
            .map_err(debug_error)?;
        runtime
            .cancel_request(
                reuse_request,
                domain_contracts::CancellationReason::UserRequested,
            )
            .map_err(debug_error)?;
        runtime.shutdown().map_err(debug_error)?;
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.successful_destructions, 2);
        assert_eq!(counters.active_sequences, 0);
        assert_eq!(counters.retained_memory_bytes, 0);
        drop(counters);
    }
    Ok(())
}

#[test]
fn short_prefill_logits_fail_before_sampling_and_cleanup_the_sequence() -> TestResult {
    let mut source = FakeSource::scripted([3; 8], 8);
    source.contract_faults = ContractFaults::SHORT_PREFILL_LOGITS;
    source.destroy_failures = 1;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(&hosted, handle, request(93, 903, 4, &[], &[]))?;

    let output = collect_until_released(&hosted, RequestId::new(93), Duration::from_secs(2))?;
    assert!(output.tokens.is_empty());
    assert_backend_contract_failure(&output);
    assert!(output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::CleanupPending { failure, .. }
            if failure.primary_operation == inference_runtime::RuntimeOperation::Prefill
                && failure.primary_failure == FailureClass::BackendContract
    )));
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.prefill_calls, 1);
        assert_eq!(counters.decode_calls, 0);
        assert_eq!(counters.destruction_attempts, 2);
        assert_eq!(counters.successful_destructions, 1);
        drop(counters);
    }
    shutdown(hosted, thread)
}

#[test]
fn short_decode_logits_fail_before_sampling_and_cleanup_the_sequence() -> TestResult {
    let mut source = FakeSource::scripted([1, 3, 0, 0, 0, 0, 0, 0], 2);
    source.contract_faults = ContractFaults::SHORT_DECODE_LOGITS;
    source.destroy_failures = 1;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(&hosted, handle, request(94, 904, 4, &[], &[]))?;

    let output = collect_until_released(&hosted, RequestId::new(94), Duration::from_secs(2))?;
    assert_eq!(output.tokens, vec![TokenId::new(1)]);
    assert_backend_contract_failure(&output);
    assert!(output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::CleanupPending { failure, .. }
            if failure.primary_operation == inference_runtime::RuntimeOperation::Decode
                && failure.primary_failure == FailureClass::BackendContract
    )));
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.prefill_calls, 1);
        assert_eq!(counters.decode_calls, 1);
        assert_eq!(counters.destruction_attempts, 2);
        assert_eq!(counters.successful_destructions, 1);
        drop(counters);
    }
    shutdown(hosted, thread)
}

#[test]
fn invalid_prefill_and_decode_positions_fail_the_generation_contract() -> TestResult {
    for (fault, request_id, expected_tokens) in [
        (ContractFaults::INVALID_PREFILL_POSITION, 95, 0),
        (ContractFaults::INVALID_DECODE_POSITION, 96, 1),
    ] {
        let mut source = FakeSource::scripted([1, 2, 0, 0, 0, 0, 0, 0], 2);
        source.contract_faults = fault;
        let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
        submit_generation(
            &hosted,
            handle,
            request(request_id, request_id + 1_000, 4, &[], &[]),
        )?;

        let output =
            collect_until_released(&hosted, RequestId::new(request_id), Duration::from_secs(2))?;
        assert_eq!(output.tokens.len(), expected_tokens);
        assert_backend_contract_failure(&output);
        assert_eq!(
            counters
                .lock()
                .map_err(|_| "counter mutex poisoned")?
                .successful_destructions,
            1
        );
        shutdown(hosted, thread)?;
    }
    Ok(())
}

#[test]
fn invalid_prefill_consumed_tokens_fail_the_generation_contract() -> TestResult {
    let mut source = FakeSource::scripted([1; 8], 8);
    source.contract_faults = ContractFaults::INVALID_CONSUMED_TOKENS;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(&hosted, handle, request(97, 907, 4, &[], &[]))?;

    let output = collect_until_released(&hosted, RequestId::new(97), Duration::from_secs(2))?;
    assert!(output.tokens.is_empty());
    assert_backend_contract_failure(&output);
    assert_eq!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .successful_destructions,
        1
    );
    shutdown(hosted, thread)
}

#[test]
fn cancellation_queued_with_admission_is_observed_before_prefill() -> TestResult {
    let (gate, entered, release) = blocking_gate();
    let mut source = FakeSource::scripted([1; 8], 8);
    source.load_gate = Some(gate);
    let (hosted, thread, counters) = start_hosted(
        8,
        16,
        NonZeroU32::MIN,
        NonZeroU32::new(4).ok_or("request limit")?,
        10_000,
    )?;
    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: CommandTicket::new(1),
            model_id: MODEL,
            source,
            execution_device: cpu_device(),
        })
        .map_err(|_| "load command rejected")?;
    entered
        .recv_timeout(Duration::from_secs(2))
        .map_err(|error| format!("load gate was not entered: {error:?}"))?;

    let handle = ModelHandle::new(MODEL, ModelGeneration::new(1));
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket: CommandTicket::new(2),
            handle,
            request: request(84, 804, 4, &[], &[]),
        })
        .map_err(|_| "generation command rejected")?;
    hosted
        .try_submit(RuntimeCommand::CancelRequest {
            ticket: CommandTicket::new(3),
            request_id: RequestId::new(84),
            reason: domain_contracts::CancellationReason::UserRequested,
        })
        .map_err(|_| "cancel command rejected")?;
    release.send(()).map_err(|_| "load gate release failed")?;

    let mut loaded = false;
    let mut admitted = false;
    let mut cancellation_recorded = false;
    for _ in 0..3 {
        match hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("queued command event failed: {error:?}"))?
        {
            RuntimeEvent::ModelLoaded { result: Ok(_), .. } => loaded = true,
            RuntimeEvent::GenerationAdmitted { result: Ok(_), .. } => admitted = true,
            RuntimeEvent::GenerationCancellationRequested { result: Ok(()), .. } => {
                cancellation_recorded = true;
            }
            _ => return Err("unexpected queued command event".into()),
        }
    }
    assert!(loaded && admitted && cancellation_recorded);

    let output = collect_until_released(&hosted, RequestId::new(84), Duration::from_secs(2))?;
    assert!(output.tokens.is_empty());
    assert!(output.states.contains(&GenerationOutputState::Terminal(
        GenerationOutcome::Finished(FinishReason::Cancelled(
            domain_contracts::CancellationReason::UserRequested
        ))
    )));
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.prefill_calls, 0);
        assert_eq!(counters.sampling_opportunities, 0);
        assert_eq!(counters.successful_destructions, 1);
        drop(counters);
    }
    shutdown(hosted, thread)
}

#[test]
fn cancellation_arriving_during_prefill_is_observed_before_decode() -> TestResult {
    let (gate, entered, release) = blocking_gate();
    let mut source = FakeSource::scripted([1; 8], 8);
    source.prefill_gate = Some(gate);
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;
    submit_generation(&hosted, handle, request(184, 1_804, 4, &[], &[]))?;
    entered
        .recv_timeout(Duration::from_secs(2))
        .map_err(|error| format!("prefill gate was not entered: {error:?}"))?;
    hosted
        .try_submit(RuntimeCommand::CancelRequest {
            ticket: CommandTicket::new(184),
            request_id: RequestId::new(184),
            reason: domain_contracts::CancellationReason::UserRequested,
        })
        .map_err(|_| "cancel command rejected")?;
    release
        .send(())
        .map_err(|_| "prefill gate release failed")?;
    assert!(matches!(
        hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("cancel event failed: {error:?}"))?,
        RuntimeEvent::GenerationCancellationRequested { result: Ok(()), .. }
    ));

    let output = collect_until_released(&hosted, RequestId::new(184), Duration::from_secs(2))?;
    assert!(output.tokens.is_empty());
    assert!(output.states.contains(&GenerationOutputState::Terminal(
        GenerationOutcome::Finished(FinishReason::Cancelled(
            domain_contracts::CancellationReason::UserRequested
        ))
    )));
    let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
    assert_eq!(counters.prefill_calls, 1);
    assert_eq!(counters.decode_calls, 0);
    drop(counters);
    shutdown(hosted, thread)
}

#[test]
fn scheduled_generation_escalates_at_the_drain_timeout() -> TestResult {
    let source = FakeSource::scripted([1, 2, 3, 1, 2, 3, 1, 2], 8);
    let (hosted, thread, counters, handle) = hosted(source, 1, 16)?;
    submit_generation(&hosted, handle, request(85, 805, 31, &[], &[]))?;

    let timeout = DrainTimeout::from_millis(2).map_err(|error| format!("{error:?}"))?;
    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(85),
            handle,
            policy: UnloadPolicy::Drain { timeout },
        })
        .map_err(|_| "drain command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("initial drain event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            result: Ok(receipt),
            ..
        } if receipt.status == inference_runtime::UnloadStatus::Draining => {}
        _ => return Err("unexpected initial drain event".into()),
    }
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("terminal drain event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            result: Ok(receipt),
            ..
        } if receipt.status == inference_runtime::UnloadStatus::Unloaded
            && receipt.cancelled_requests == 1 => {}
        _ => return Err("unexpected terminal drain event".into()),
    }

    let output = collect_until_released(&hosted, RequestId::new(85), Duration::from_secs(2))?;
    assert!(output.states.contains(&GenerationOutputState::Terminal(
        GenerationOutcome::Finished(FinishReason::Cancelled(
            domain_contracts::CancellationReason::DrainTimeout
        ))
    )));
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.successful_destructions, 1);
        assert_eq!(counters.active_sequences, 0);
        assert_eq!(counters.retained_memory_bytes, 0);
        drop(counters);
    }
    shutdown(hosted, thread)
}

#[test]
fn model_unload_retry_recovers_and_releases_accounting_once() -> TestResult {
    let mut source = FakeSource::scripted([1; 8], 8);
    source.unload_failures = 2;
    let (hosted, thread, counters, handle) = hosted(source, 8, 16)?;

    hosted
        .try_submit(RuntimeCommand::UnloadModel {
            ticket: CommandTicket::new(88),
            handle,
            policy: UnloadPolicy::RejectIfBusy,
        })
        .map_err(|_| "unload command rejected")?;

    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("initial unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            result: Err(inference_runtime::RuntimeError::CleanupFailed(state)),
            ..
        } if state.failure.primary_operation
            == inference_runtime::RuntimeOperation::ModelUnload
            && state.failure.primary_failure == inference_runtime::FailureClass::Completion
            && state.failure.cleanup_failure
                == inference_runtime::FailureClass::Synchronization => {}
        _ => return Err("unexpected initial unload event".into()),
    }

    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("recovered unload event failed: {error:?}"))?
    {
        RuntimeEvent::ModelUnload {
            result: Ok(receipt),
            ..
        } if receipt.status == inference_runtime::UnloadStatus::Unloaded
            && receipt.cancelled_requests == 0 => {}
        _ => return Err("unexpected recovered unload event".into()),
    }

    hosted
        .try_submit(RuntimeCommand::Snapshot {
            ticket: CommandTicket::new(89),
        })
        .map_err(|_| "snapshot command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("snapshot event failed: {error:?}"))?
    {
        RuntimeEvent::Snapshot {
            runtime,
            models,
            retained_models,
            ..
        } => {
            assert_eq!(runtime.loaded_models, 0);
            assert_eq!(runtime.pending_cleanup_models, 0);
            assert_eq!(runtime.exhausted_cleanup_models, 0);
            assert_eq!(runtime.reserved_footprint, MemoryFootprint::default());
            assert!(runtime.unverified_ownership.is_none());
            assert!(!runtime.admission_blocked);
            assert!(models.is_empty());
            assert!(retained_models.is_empty());
        }
        _ => return Err("unexpected snapshot event".into()),
    }

    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.unload_attempts, 3);
        assert_eq!(counters.retained_memory_bytes, 0);
        drop(counters);
    }
    shutdown(hosted, thread)?;
    let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
    assert_eq!(counters.unload_attempts, 3);
    assert_eq!(counters.retained_memory_bytes, 0);
    drop(counters);
    Ok(())
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the complete two-model isolation scenario is intentionally kept contiguous"
)]
fn healthy_model_progresses_while_another_model_retries_cleanup() -> TestResult {
    let (prefill_gate, entered, release) = blocking_gate();
    let mut failing = FakeSource::scripted([1; 8], 8);
    failing.fail_prefill = true;
    failing.destroy_failures = 2;
    failing.prefill_gate = Some(prefill_gate);
    let healthy = FakeSource::scripted([1, 2, 3, 0, 0, 0, 0, 0], 3);

    let (hosted, thread, counters) = start_hosted(
        32,
        64,
        NonZeroU32::new(2).ok_or("model limit")?,
        NonZeroU32::new(4).ok_or("request limit")?,
        20_000,
    )?;
    let failing_handle = load_model(&hosted, ModelId::new(1), failing, CommandTicket::new(1))?;
    let healthy_handle = load_model(&hosted, ModelId::new(2), healthy, CommandTicket::new(2))?;

    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket: CommandTicket::new(10),
            handle: failing_handle,
            request: request(86, 806, 4, &[], &[]),
        })
        .map_err(|_| "failing generation command rejected")?;
    entered
        .recv_timeout(Duration::from_secs(2))
        .map_err(|error| format!("prefill gate was not entered: {error:?}"))?;
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket: CommandTicket::new(11),
            handle: healthy_handle,
            request: request(87, 807, 3, &[], &[]),
        })
        .map_err(|_| "healthy generation command rejected")?;
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket: CommandTicket::new(12),
            handle: failing_handle,
            request: request(88, 808, 3, &[], &[]),
        })
        .map_err(|_| "degraded-model generation command rejected")?;
    release
        .send(())
        .map_err(|_| "prefill gate release failed")?;

    let mut admitted = 0_u32;
    let mut degraded_rejected = false;
    for _ in 0..3 {
        match hosted
            .receive_timeout(Duration::from_secs(2))
            .map_err(|error| format!("isolation admission event failed: {error:?}"))?
        {
            RuntimeEvent::GenerationAdmitted { result: Ok(_), .. } => {
                admitted = admitted.saturating_add(1);
            }
            RuntimeEvent::GenerationAdmitted {
                ticket,
                result: Err(inference_runtime::RuntimeError::ModelDegraded(model_id)),
            } if ticket == CommandTicket::new(12) && model_id == failing_handle.id => {
                degraded_rejected = true;
            }
            _ => return Err("unexpected isolation admission event".into()),
        }
    }
    assert_eq!(admitted, 2);
    assert!(degraded_rejected);
    let mut outputs = collect_until_all_released(
        &hosted,
        &[RequestId::new(86), RequestId::new(87)],
        Duration::from_secs(2),
    )?;
    let failing_output = outputs
        .remove(&RequestId::new(86))
        .ok_or("missing failing output")?;
    let healthy_output = outputs
        .remove(&RequestId::new(87))
        .ok_or("missing healthy output")?;
    assert!(failing_output.states.iter().any(|state| matches!(
        state,
        GenerationOutputState::Terminal(GenerationOutcome::Failed(_))
    )));
    assert_eq!(
        healthy_output.tokens,
        vec![TokenId::new(1), TokenId::new(2), TokenId::new(3)]
    );
    {
        let counters = counters.lock().map_err(|_| "counter mutex poisoned")?;
        assert_eq!(counters.sequence_creations, 2);
        assert_eq!(counters.destruction_attempts, 4);
        assert_eq!(counters.successful_destructions, 2);
        assert_eq!(counters.sampling_opportunities, 3);
        assert_eq!(counters.active_sequences, 0);
        assert_eq!(counters.retained_memory_bytes, 200);
        drop(counters);
    }
    shutdown(hosted, thread)?;
    assert_eq!(
        counters
            .lock()
            .map_err(|_| "counter mutex poisoned")?
            .retained_memory_bytes,
        0
    );
    Ok(())
}

fn stochastic_run(seed: u64) -> TestResult<Vec<TokenId>> {
    let mut source = FakeSource::scripted([0; 8], 0);
    source.uniform_logits = true;
    let (hosted, thread, _, handle) = hosted(source, 16, 32)?;
    let mut generation = request(50, 500, 5, &[], &[]);
    generation.seed = seed;
    generation.sampling = SamplingConfig {
        temperature: 1.0,
        top_k: 0,
        top_p: 1.0,
        min_p: 0.0,
        repetition_penalty: 1.0,
        repetition_window: 0,
    };
    submit_generation(&hosted, handle, generation)?;
    let output = collect_until_released(&hosted, RequestId::new(50), Duration::from_secs(2))?;
    shutdown(hosted, thread)?;
    Ok(output.tokens)
}

struct CollectedOutput {
    tokens: Vec<TokenId>,
    states: Vec<GenerationOutputState>,
}

fn assert_backend_contract_failure(output: &CollectedOutput) {
    assert!(
        output
            .states
            .contains(&GenerationOutputState::Terminal(GenerationOutcome::Failed(
                inference_runtime::RuntimeError::BackendContractViolation
            )))
    );
}

fn collect_until_released(
    hosted: &HostedRuntime<FakeSource>,
    request_id: RequestId,
    timeout: Duration,
) -> TestResult<CollectedOutput> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("deadline overflow")?;
    let mut output = CollectedOutput {
        tokens: Vec::new(),
        states: Vec::new(),
    };
    loop {
        hosted
            .pull_token_output(|batch| {
                for record in batch.records {
                    if record.request_id != request_id {
                        continue;
                    }
                    match record.kind {
                        TokenOutputRecordKind::Tokens(range) => {
                            if let Some(tokens) = batch.tokens_for(range) {
                                output.tokens.extend_from_slice(tokens);
                            }
                        }
                        TokenOutputRecordKind::State(state) => output.states.push(state),
                    }
                }
            })
            .map_err(|error| format!("token pull failed: {error:?}"))?;
        if output
            .states
            .iter()
            .any(|state| matches!(state, GenerationOutputState::Released(_)))
        {
            return Ok(output);
        }
        if Instant::now() >= deadline {
            return Err(format!("generation timed out: {:?}", output.states));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn collect_until_all_released(
    hosted: &HostedRuntime<FakeSource>,
    request_ids: &[RequestId],
    timeout: Duration,
) -> TestResult<BTreeMap<RequestId, CollectedOutput>> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("deadline overflow")?;
    let requested = request_ids.iter().copied().collect::<BTreeSet<_>>();
    let mut outputs = request_ids
        .iter()
        .copied()
        .map(|request_id| {
            (
                request_id,
                CollectedOutput {
                    tokens: Vec::new(),
                    states: Vec::new(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    loop {
        hosted
            .pull_token_output(|batch| {
                for record in batch.records {
                    let Some(output) = outputs.get_mut(&record.request_id) else {
                        continue;
                    };
                    match record.kind {
                        TokenOutputRecordKind::Tokens(range) => {
                            if let Some(tokens) = batch.tokens_for(range) {
                                output.tokens.extend_from_slice(tokens);
                            }
                        }
                        TokenOutputRecordKind::State(state) => output.states.push(state),
                    }
                }
            })
            .map_err(|error| format!("token pull failed: {error:?}"))?;
        let released = outputs
            .iter()
            .filter(|(request_id, output)| {
                requested.contains(request_id)
                    && output
                        .states
                        .iter()
                        .any(|state| matches!(state, GenerationOutputState::Released(_)))
            })
            .count();
        if released == requested.len() {
            return Ok(outputs);
        }
        if Instant::now() >= deadline {
            return Err("multi-request generation timed out".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn synchronous_runtime(source: &FakeSource) -> TestResult<SynchronousParts> {
    let counters = Arc::new(Mutex::new(Counters::default()));
    let mut runtime = InferenceRuntime::new(
        FakeLoader {
            counters: Arc::clone(&counters),
        },
        RuntimeLimits::new(
            NonZeroU32::MIN,
            NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
            MemoryBudget {
                host_bytes: 10_000,
                device_bytes: 0,
            },
        ),
    );
    let handle = runtime
        .load_model(MODEL, source, cpu_device())
        .map_err(debug_error)?
        .handle;
    Ok((runtime, counters, handle))
}

fn direct_sequence_configuration() -> SequenceConfiguration {
    SequenceConfiguration::new(
        NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
        NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
    )
}

fn debug_error(error: impl core::fmt::Debug) -> String {
    format!("{error:?}")
}

fn hosted(
    source: FakeSource,
    token_capacity: usize,
    record_capacity: usize,
) -> TestResult<HostedParts> {
    hosted_with_budget(source, token_capacity, record_capacity, 10_000)
}

fn hosted_with_budget(
    source: FakeSource,
    token_capacity: usize,
    record_capacity: usize,
    host_bytes: u64,
) -> TestResult<HostedParts> {
    let (hosted, thread, counters) = start_hosted(
        token_capacity,
        record_capacity,
        NonZeroU32::MIN,
        NonZeroU32::new(4).ok_or("request limit")?,
        host_bytes,
    )?;
    let handle = load_model(&hosted, MODEL, source, CommandTicket::new(1))?;
    Ok((hosted, thread, counters, handle))
}

fn start_hosted(
    token_capacity: usize,
    record_capacity: usize,
    maximum_loaded_models: NonZeroU32,
    maximum_active_requests: NonZeroU32,
    host_bytes: u64,
) -> TestResult<(
    HostedRuntime<FakeSource>,
    RuntimeThread,
    Arc<Mutex<Counters>>,
)> {
    let counters = Arc::new(Mutex::new(Counters::default()));
    let configuration =
        HostedRuntimeConfiguration::new(nonzero_usize(8)?, nonzero_usize(8)?, NonZeroU64::MIN)
            .with_token_output_capacity(
                nonzero_usize(token_capacity)?,
                nonzero_usize(record_capacity)?,
            );
    let (hosted, thread) = start_hosted_runtime(
        FakeLoader {
            counters: Arc::clone(&counters),
        },
        RuntimeLimits::new(
            maximum_loaded_models,
            maximum_active_requests,
            MemoryBudget {
                host_bytes,
                device_bytes: 0,
            },
        ),
        configuration,
    )
    .map_err(|error| error.to_string())?;
    Ok((hosted, thread, counters))
}

fn load_model(
    hosted: &HostedRuntime<FakeSource>,
    model_id: ModelId,
    source: FakeSource,
    ticket: CommandTicket,
) -> TestResult<ModelHandle> {
    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket,
            model_id,
            source,
            execution_device: cpu_device(),
        })
        .map_err(|_| "load command rejected")?;
    let event = hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("load event: {error:?}"))?;
    let RuntimeEvent::ModelLoaded {
        ticket: event_ticket,
        result: Ok(receipt),
    } = event
    else {
        return Err("model load failed".into());
    };
    if event_ticket != ticket {
        return Err("model load ticket mismatch".into());
    }
    Ok(receipt.handle)
}

fn submit_generation(
    hosted: &HostedRuntime<FakeSource>,
    handle: ModelHandle,
    request: GenerationRequest,
) -> TestResult {
    submit_generation_with_ticket(hosted, handle, request, CommandTicket::new(2))
}

fn submit_generation_with_ticket(
    hosted: &HostedRuntime<FakeSource>,
    handle: ModelHandle,
    request: GenerationRequest,
    ticket: CommandTicket,
) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket,
            handle,
            request,
        })
        .map_err(|_| "generation command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("generation admission event: {error:?}"))?
    {
        RuntimeEvent::GenerationAdmitted {
            ticket: event_ticket,
            result: Ok(_),
        } if event_ticket == ticket => Ok(()),
        RuntimeEvent::GenerationAdmitted {
            result: Err(error), ..
        } => Err(format!("generation admission failed: {error:?}")),
        _ => Err("unexpected generation admission event".into()),
    }
}

fn submit_generation_error(
    hosted: &HostedRuntime<FakeSource>,
    handle: ModelHandle,
    request: GenerationRequest,
    ticket: CommandTicket,
) -> TestResult<inference_runtime::RuntimeError> {
    hosted
        .try_submit(RuntimeCommand::Generate {
            ticket,
            handle,
            request,
        })
        .map_err(|_| "generation command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("generation admission event: {error:?}"))?
    {
        RuntimeEvent::GenerationAdmitted {
            ticket: event_ticket,
            result: Err(error),
        } if event_ticket == ticket => Ok(error),
        _ => Err("generation admission unexpectedly succeeded".into()),
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the helper owns the client through worker join and then drops both endpoints together"
)]
fn shutdown(hosted: HostedRuntime<FakeSource>, thread: RuntimeThread) -> TestResult {
    hosted
        .try_submit(RuntimeCommand::Shutdown {
            ticket: CommandTicket::new(99),
        })
        .map_err(|_| "shutdown command rejected")?;
    match hosted
        .receive_timeout(Duration::from_secs(2))
        .map_err(|error| format!("shutdown event: {error:?}"))?
    {
        RuntimeEvent::Shutdown { result: Ok(_), .. } => {}
        _ => return Err("unexpected shutdown event".into()),
    }
    thread.join().map_err(|error| error.to_string())
}

fn request(
    request: u64,
    sequence: u64,
    maximum_generated_tokens: u32,
    eos_tokens: &[TokenId],
    stops: &[GenerationStopSequence],
) -> GenerationRequest {
    GenerationRequest {
        request_id: RequestId::new(request),
        sequence_id: SequenceId::new(sequence),
        prompt_tokens: vec![TokenId::new(0)].into_boxed_slice(),
        sequence: SequenceConfiguration::new(
            NonZeroU32::new(32).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
        ),
        maximum_generated_tokens: NonZeroU32::new(maximum_generated_tokens)
            .unwrap_or(NonZeroU32::MIN),
        sampling: SamplingConfig::greedy(),
        seed: 17,
        eos_tokens: eos_tokens.to_vec().into_boxed_slice(),
        stop_sequences: stops.to_vec().into_boxed_slice(),
        output_capacity: GenerationOutputCapacityPolicy::default(),
    }
}

fn write_logits(source: &FakeSource, generated: usize, logits: &mut [f32]) {
    if source.no_candidate {
        logits.fill(f32::NEG_INFINITY);
        return;
    }
    if source.uniform_logits {
        logits.fill(0.0);
        return;
    }
    logits.fill(-100.0);
    let token = if generated < source.script_len {
        source.script.get(generated).copied().unwrap_or(0)
    } else {
        0
    };
    if let Some(logit) = logits.get_mut(token as usize) {
        *logit = 100.0;
    }
}

const fn descriptor(operations: CapabilitySet) -> ModelDescriptor {
    ModelDescriptor {
        backend: BACKEND,
        metadata: ModelMetadata {
            architecture: ModelArchitecture::Llama,
            configuration_declared_scalar_type: Some(ScalarType::F32),
            observed_tensor_scalar_types: ScalarTypeSet::from_scalar(ScalarType::F32),
            quantization: QuantizationFormat::None,
            vocabulary_size: 4,
            context_length: 64,
        },
        capabilities: ModelCapabilities {
            operations,
            maximum_context_tokens: 64,
            maximum_sequences: 4,
            maximum_prefill_batch: 8,
        },
        estimated_footprint: model_footprint(),
        sequence_cache_bytes_per_token: 0,
    }
}

const fn model_footprint() -> MemoryFootprint {
    MemoryFootprint {
        host_weight_bytes: MODEL_HOST_BYTES,
        device_weight_bytes: 0,
        host_working_bytes: 0,
        device_working_bytes: 0,
    }
}

const fn sequence_footprint() -> MemoryFootprint {
    MemoryFootprint {
        host_weight_bytes: 0,
        device_weight_bytes: 0,
        host_working_bytes: SEQUENCE_HOST_BYTES,
        device_working_bytes: 0,
    }
}

const fn failure(code: u32) -> BackendFailure {
    BackendFailure::new(BACKEND, BackendFailureKind::Internal, code)
}

fn nonzero_usize(value: usize) -> TestResult<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| "non-zero capacity required".into())
}
