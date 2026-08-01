//! Repeatable component-like cross-crate measurements through hosted public E0.
//!
//! # Exact questions and boundaries
//!
//! `e0_hosted_checked_prefill/4_tokens` asks whether the production hosted E0
//! checked-prefill submission-to-event boundary regresses for the deterministic
//! four-token prompt. `e0_hosted_incremental_decode/1_token_after_2_token_prefill`
//! asks the same for one incremental decode after an untimed two-token prefill. Each accumulated
//! duration starts immediately before `HostedRuntime::try_submit` and ends after
//! reception and ticket matching of `PrefillCompleted` or `DecodeCompleted`.
//!
//! Model source construction, fixture checks, worker start, model load, request
//! and sequence creation, prompt/token construction, vector allocation, setup
//! prefill for decode, request completion, model unload, shutdown, and join are
//! outside accumulated time. The returned vocabulary-sized logits allocation is
//! moved back into the next iteration instead of being reallocated. The timed
//! boundary does include bounded command/event transport, public E0 dispatch,
//! checked validation, and Candle CPU execution. Candle may allocate native or
//! tensor/KV resources internally; this benchmark times but does not count or
//! attribute those allocations. It records neither allocator events nor RSS.
//! The project-authored Llama/Safetensors/F32 vocabulary-16/context-16 fixture is
//! synthetic-system-class integration evidence, not product performance.

#![forbid(unsafe_code)]

use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::process;
use std::time::{Duration, Instant};

use candle_backend::{CandleLlamaLoader, CandleLlamaSource};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use domain_contracts::{
    BackendId, DecodeOutcome, DeviceId, DeviceKind, FinishReason, MemoryBudget, ModelArchitecture,
    ModelHandle, ModelId, PrefillOutcome, QuantizationFormat, RequestId, ScalarType,
    SequenceConfiguration, SequenceId, TokenId, UnloadPolicy,
};
use inference_runtime::{
    CommandTicket, HostedRuntime, HostedRuntimeConfiguration, RuntimeCommand, RuntimeEvent,
    RuntimeLimits, RuntimeThread, UnloadStatus, start_hosted_runtime,
};

const BACKEND_ID: BackendId = BackendId::new(10_002);
const MODEL_ID: ModelId = ModelId::new(7);
const VOCABULARY_SIZE: usize = 16;
const CONTEXT_CAPACITY: u32 = 16;
const PREFILL_TOKEN_COUNT: u32 = 4;
const PREFILL_TOKEN_COUNT_USIZE: usize = 4;
const PREFILL_THROUGHPUT: u64 = 4;
const DECODE_THROUGHPUT: u64 = 1;

const EVENT_TIMEOUT: Duration = Duration::from_secs(10);
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);
const WAIT_INTERVAL: Duration = Duration::from_millis(1);
const BENCHMARK_FAILURE_EXIT_CODE: i32 = 2;

struct BenchHarness {
    runtime: HostedRuntime<CandleLlamaSource>,
    thread: Option<RuntimeThread>,
    handle: ModelHandle,
    next_ticket: u64,
    next_request: u64,
}

impl BenchHarness {
    fn start() -> Result<Self, String> {
        let source =
            runtime_benchmarks::synthetic_fixture_source().map_err(|error| error.to_string())?;
        let configuration = HostedRuntimeConfiguration::new(
            nonzero_usize(16)?,
            nonzero_usize(16)?,
            NonZeroU64::MIN,
        )
        .with_token_output_capacity(nonzero_usize(16)?, nonzero_usize(64)?);
        let (runtime, thread) = start_hosted_runtime(
            CandleLlamaLoader::new(BACKEND_ID),
            RuntimeLimits::new(
                NonZeroU32::MIN,
                NonZeroU32::MIN,
                MemoryBudget {
                    host_bytes: u64::MAX,
                    device_bytes: 0,
                },
            ),
            configuration,
        )
        .map_err(|error| error.to_string())?;
        let mut harness = Self {
            runtime,
            thread: Some(thread),
            handle: ModelHandle::new(MODEL_ID, domain_contracts::ModelGeneration::new(0)),
            next_ticket: 1,
            next_request: 1,
        };
        let ticket = harness.ticket()?;
        harness.submit(
            RuntimeCommand::LoadModel {
                ticket,
                model_id: MODEL_ID,
                source,
                device: DeviceId::new(0),
                device_kind: DeviceKind::Cpu,
            },
            "model load",
        )?;
        match harness.receive(ticket, "model load")? {
            RuntimeEvent::ModelLoaded {
                result: Ok(receipt),
                ..
            } if receipt.descriptor.backend == BACKEND_ID
                && receipt.descriptor.metadata.architecture == ModelArchitecture::Llama
                && receipt.descriptor.metadata.scalar_type == ScalarType::F32
                && receipt.descriptor.metadata.quantization == QuantizationFormat::None
                && receipt.descriptor.metadata.vocabulary_size
                    == u32::try_from(VOCABULARY_SIZE)
                        .map_err(|_| "vocabulary conversion failed".to_owned())?
                && receipt.descriptor.metadata.context_length == CONTEXT_CAPACITY =>
            {
                harness.handle = receipt.handle;
            }
            RuntimeEvent::ModelLoaded {
                result: Err(error), ..
            } => return Err(format!("model load failed: {error:?}")),
            _ => return Err("model load returned unexpected descriptor or event".to_owned()),
        }
        Ok(harness)
    }

    fn ticket(&mut self) -> Result<CommandTicket, String> {
        let ticket = CommandTicket::new(self.next_ticket);
        self.next_ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or_else(|| "benchmark command ticket exhausted".to_owned())?;
        Ok(ticket)
    }

    fn request_identity(&mut self) -> Result<(RequestId, SequenceId), String> {
        let value = self.next_request;
        self.next_request = self
            .next_request
            .checked_add(1)
            .ok_or_else(|| "benchmark request identity exhausted".to_owned())?;
        Ok((RequestId::new(value), SequenceId::new(value)))
    }

    fn submit(
        &self,
        command: RuntimeCommand<CandleLlamaSource>,
        operation: &str,
    ) -> Result<(), String> {
        self.runtime
            .try_submit(command)
            .map_err(|error| format!("{operation} command rejected: {error:?}"))
    }

    fn receive(&self, ticket: CommandTicket, operation: &str) -> Result<RuntimeEvent, String> {
        let event = self
            .runtime
            .receive_timeout(EVENT_TIMEOUT)
            .map_err(|error| format!("{operation} event failed: {error:?}"))?;
        if event.ticket() != ticket {
            return Err(format!(
                "{operation} returned ticket {}, expected {}",
                event.ticket().get(),
                ticket.get()
            ));
        }
        Ok(event)
    }

    fn start_request(&mut self) -> Result<(RequestId, usize), String> {
        let (request_id, sequence_id) = self.request_identity()?;
        let ticket = self.ticket()?;
        self.submit(
            RuntimeCommand::StartRequest {
                ticket,
                handle: self.handle,
                request_id,
                sequence_id,
                configuration: SequenceConfiguration::new(
                    nonzero_u32(CONTEXT_CAPACITY)?,
                    nonzero_u32(PREFILL_TOKEN_COUNT)?,
                ),
            },
            "request setup",
        )?;
        match self.receive(ticket, "request setup")? {
            RuntimeEvent::RequestStarted {
                result: Ok(receipt),
                ..
            } if receipt.request_id == request_id
                && receipt.sequence_id == sequence_id
                && receipt.logits_capacity == VOCABULARY_SIZE =>
            {
                Ok((request_id, receipt.logits_capacity))
            }
            RuntimeEvent::RequestStarted {
                result: Err(error), ..
            } => Err(format!("request setup failed: {error:?}")),
            _ => Err("request setup returned unexpected identity or capacity".to_owned()),
        }
    }

    fn setup_prefill(
        &mut self,
        request_id: RequestId,
        logits: Vec<f32>,
        tokens: Box<[TokenId]>,
    ) -> Result<Vec<f32>, String> {
        let ticket = self.ticket()?;
        self.submit(
            RuntimeCommand::Prefill {
                ticket,
                request_id,
                tokens,
                emit_logits: true,
                logits,
            },
            "untimed setup prefill",
        )?;
        match self.receive(ticket, "untimed setup prefill")? {
            RuntimeEvent::PrefillCompleted {
                request_id: event_request,
                result: Ok(receipt),
                logits,
                ..
            } if event_request == request_id && logits.len() == VOCABULARY_SIZE => {
                match receipt.outcome {
                    PrefillOutcome::Ready { .. } => Ok(logits),
                    PrefillOutcome::Finished(reason) => {
                        Err(format!("setup prefill finished early: {reason:?}"))
                    }
                }
            }
            RuntimeEvent::PrefillCompleted {
                result: Err(error), ..
            } => Err(format!("setup prefill failed: {error:?}")),
            _ => Err("setup prefill returned unexpected event".to_owned()),
        }
    }

    fn complete_request(&mut self, request_id: RequestId) -> Result<(), String> {
        let ticket = self.ticket()?;
        self.submit(
            RuntimeCommand::CompleteRequest {
                ticket,
                request_id,
                reason: FinishReason::TokenLimit,
            },
            "request completion",
        )?;
        match self.receive(ticket, "request completion")? {
            RuntimeEvent::RequestFinished {
                request_id: event_request,
                result: Ok(FinishReason::TokenLimit),
                ..
            } if event_request == request_id => Ok(()),
            RuntimeEvent::RequestFinished {
                result: Err(error), ..
            } => Err(format!("request completion failed: {error:?}")),
            _ => Err("request completion returned unexpected event".to_owned()),
        }
    }

    fn shutdown(mut self) -> Result<(), String> {
        let unload_ticket = self.ticket()?;
        self.submit(
            RuntimeCommand::UnloadModel {
                ticket: unload_ticket,
                handle: self.handle,
                policy: UnloadPolicy::RejectIfBusy,
            },
            "model unload",
        )?;
        match self.receive(unload_ticket, "model unload")? {
            RuntimeEvent::ModelUnload {
                result: Ok(receipt),
                ..
            } if receipt.status == UnloadStatus::Unloaded
                && receipt.handle == self.handle
                && receipt.cancelled_requests == 0 => {}
            RuntimeEvent::ModelUnload {
                result: Err(error), ..
            } => return Err(format!("model unload failed: {error:?}")),
            _ => return Err("model unload returned unexpected accounting".to_owned()),
        }

        let shutdown_ticket = self.ticket()?;
        self.submit(
            RuntimeCommand::Shutdown {
                ticket: shutdown_ticket,
            },
            "runtime shutdown",
        )?;
        match self.receive(shutdown_ticket, "runtime shutdown")? {
            RuntimeEvent::Shutdown {
                result: Ok(receipt),
                ..
            } if receipt.unloaded_models == 0 && receipt.cancelled_requests == 0 => {}
            RuntimeEvent::Shutdown {
                result: Err(error), ..
            } => return Err(format!("runtime shutdown failed: {error:?}")),
            _ => return Err("runtime shutdown returned unexpected accounting".to_owned()),
        }
        let deadline = Instant::now()
            .checked_add(JOIN_TIMEOUT)
            .ok_or_else(|| "runtime join deadline overflowed".to_owned())?;
        loop {
            let finished = self
                .thread
                .as_ref()
                .ok_or_else(|| "runtime thread handle missing".to_owned())?
                .is_finished();
            if finished {
                break;
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|duration| !duration.is_zero())
                .ok_or_else(|| "runtime worker join timed out".to_owned())?;
            std::thread::sleep(WAIT_INTERVAL.min(remaining));
        }
        let thread = self
            .thread
            .take()
            .ok_or_else(|| "runtime thread handle disappeared".to_owned())?;
        thread.join().map_err(|error| error.to_string())
    }
}

fn benchmark_runtime_components(criterion: &mut Criterion) {
    benchmark_prefill(criterion);
    benchmark_decode(criterion);
}

fn benchmark_prefill(criterion: &mut Criterion) {
    let mut harness = result_or_exit(BenchHarness::start(), "prefill harness setup");
    let mut logits = vec![0.0_f32; VOCABULARY_SIZE];
    let mut group = criterion.benchmark_group("e0_hosted_checked_prefill");
    group.throughput(Throughput::Elements(PREFILL_THROUGHPUT));
    group.bench_function("4_tokens", |benchmark| {
        benchmark.iter_custom(|iterations| {
            measure_prefill_iterations(&mut harness, &mut logits, iterations)
        });
    });
    group.finish();
    result_or_exit(harness.shutdown(), "prefill harness teardown");
}

fn measure_prefill_iterations(
    harness: &mut BenchHarness,
    logits: &mut Vec<f32>,
    iterations: u64,
) -> Duration {
    let mut measured = Duration::ZERO;
    for _ in 0..iterations {
        let (request_id, logits_capacity) =
            result_or_exit(harness.start_request(), "prefill request setup");
        if logits.len() != logits_capacity {
            benchmark_failure("prefill reusable logits capacity changed");
        }
        let prompt = vec![
            TokenId::new(1),
            TokenId::new(2),
            TokenId::new(3),
            TokenId::new(4),
        ]
        .into_boxed_slice();
        let ticket = result_or_exit(harness.ticket(), "prefill ticket allocation");
        let command = RuntimeCommand::Prefill {
            ticket,
            request_id,
            tokens: prompt,
            emit_logits: true,
            logits: std::mem::take(logits),
        };
        let started = Instant::now();
        result_or_exit(
            harness.submit(command, "timed checked prefill"),
            "timed checked prefill",
        );
        let event = result_or_exit(
            harness.receive(ticket, "timed checked prefill"),
            "timed checked prefill",
        );
        measured = measured.saturating_add(started.elapsed());
        match event {
            RuntimeEvent::PrefillCompleted {
                request_id: event_request,
                result: Ok(receipt),
                logits: returned,
                ..
            } if event_request == request_id && returned.len() == VOCABULARY_SIZE => {
                match receipt.outcome {
                    PrefillOutcome::Ready {
                        consumed_tokens,
                        logits_written,
                        ..
                    } if consumed_tokens == PREFILL_TOKEN_COUNT_USIZE
                        && logits_written == VOCABULARY_SIZE =>
                    {
                        *logits = returned;
                    }
                    _ => benchmark_failure("timed checked prefill outcome validation"),
                }
            }
            RuntimeEvent::PrefillCompleted {
                result: Err(error), ..
            } => benchmark_failure_with_debug("timed checked prefill execution", &error),
            _ => benchmark_failure("timed checked prefill event validation"),
        }
        result_or_exit(
            harness.complete_request(request_id),
            "prefill request completion",
        );
    }
    measured
}

fn benchmark_decode(criterion: &mut Criterion) {
    let mut harness = result_or_exit(BenchHarness::start(), "decode harness setup");
    let mut logits = vec![0.0_f32; VOCABULARY_SIZE];
    let mut group = criterion.benchmark_group("e0_hosted_incremental_decode");
    group.throughput(Throughput::Elements(DECODE_THROUGHPUT));
    group.bench_function("1_token_after_2_token_prefill", |benchmark| {
        benchmark.iter_custom(|iterations| {
            measure_decode_iterations(&mut harness, &mut logits, iterations)
        });
    });
    group.finish();
    result_or_exit(harness.shutdown(), "decode harness teardown");
}

fn measure_decode_iterations(
    harness: &mut BenchHarness,
    logits: &mut Vec<f32>,
    iterations: u64,
) -> Duration {
    let mut measured = Duration::ZERO;
    for _ in 0..iterations {
        let (request_id, logits_capacity) =
            result_or_exit(harness.start_request(), "decode request setup");
        if logits.len() != logits_capacity {
            benchmark_failure("decode reusable logits capacity changed");
        }
        let setup_logits = std::mem::take(logits);
        *logits = result_or_exit(
            harness.setup_prefill(
                request_id,
                setup_logits,
                vec![TokenId::new(1), TokenId::new(2)].into_boxed_slice(),
            ),
            "decode setup prefill",
        );
        let ticket = result_or_exit(harness.ticket(), "decode ticket allocation");
        let command = RuntimeCommand::Decode {
            ticket,
            request_id,
            token: TokenId::new(2),
            logits: std::mem::take(logits),
        };
        let started = Instant::now();
        result_or_exit(
            harness.submit(command, "timed incremental decode"),
            "timed incremental decode",
        );
        let event = result_or_exit(
            harness.receive(ticket, "timed incremental decode"),
            "timed incremental decode",
        );
        measured = measured.saturating_add(started.elapsed());
        match event {
            RuntimeEvent::DecodeCompleted {
                request_id: event_request,
                result: Ok(receipt),
                logits: returned,
                ..
            } if event_request == request_id && returned.len() == VOCABULARY_SIZE => {
                match receipt.outcome {
                    DecodeOutcome::Ready { logits_written, .. }
                        if logits_written == VOCABULARY_SIZE =>
                    {
                        *logits = returned;
                    }
                    _ => benchmark_failure("timed incremental decode outcome validation"),
                }
            }
            RuntimeEvent::DecodeCompleted {
                result: Err(error), ..
            } => benchmark_failure_with_debug("timed incremental decode execution", &error),
            _ => benchmark_failure("timed incremental decode event validation"),
        }
        result_or_exit(
            harness.complete_request(request_id),
            "decode request completion",
        );
    }
    measured
}

fn nonzero_usize(value: usize) -> Result<NonZeroUsize, String> {
    NonZeroUsize::new(value).ok_or_else(|| "capacity must be non-zero".to_owned())
}

fn nonzero_u32(value: u32) -> Result<NonZeroU32, String> {
    NonZeroU32::new(value).ok_or_else(|| "capacity must be non-zero".to_owned())
}

fn result_or_exit<T>(result: Result<T, String>, operation: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{operation} failed: {error}");
            process::exit(BENCHMARK_FAILURE_EXIT_CODE);
        }
    }
}

fn benchmark_failure(operation: &str) -> ! {
    eprintln!("{operation} failed");
    process::exit(BENCHMARK_FAILURE_EXIT_CODE);
}

fn benchmark_failure_with_debug(operation: &str, error: &impl std::fmt::Debug) -> ! {
    eprintln!("{operation} failed: {error:?}");
    process::exit(BENCHMARK_FAILURE_EXIT_CODE);
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(2))
        .sample_size(10);
    targets = benchmark_runtime_components
}
criterion_main!(benches);
