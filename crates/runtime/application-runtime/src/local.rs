//! Private concrete composition for the two supported local E0 capabilities.

use std::num::{NonZeroI32, NonZeroU32};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use candle_backend::{CandleLlamaLoader, CandleLlamaSource};
use domain_contracts::{
    BackendId, CancellationReason, DeviceId, DeviceKind, ModelDescriptor, ModelHandle, ModelId,
    TokenId, UnloadPolicy,
};
use gguf_backend::{
    GgufBackendRuntime, GgufExecutionConfiguration, GgufLoader, GgufMetadata,
    GgufOwnedStreamingDecoder, GgufSource, GgufTokenizer, Sha256Digest, inspect_metadata,
    sha256_file,
};
use hf_tokenizer::{HfOwnedStreamingDecoder, HfTokenizer};
use host_runtime::{OutputPullError, TokenOutputBatch};
use inference_runtime::{
    CommandTicket, GenerationOutputState, GenerationRequest, HostedRuntime,
    HostedRuntimeConfiguration, RuntimeCommand, RuntimeEvent, RuntimeLimits, RuntimeReceiveError,
    RuntimeThread, start_hosted_runtime,
};
use tokenization::{
    DecodeOptions, DecodeReport, EncodeOptions, EncodeReport, StreamingDecoder, TextSink,
    TokenizationError, Tokenizer,
};

use crate::{
    ApplicationBackend, ApplicationConfigurationField, ApplicationError, ApplicationFailure,
    ApplicationFailureKind, ApplicationGgufConfiguration,
};

const CANDLE_BACKEND_ID: BackendId = BackendId::new(1);
const GGUF_BACKEND_ID: BackendId = BackendId::new(2);

static GGUF_BACKEND: OnceLock<
    Result<GgufBackendRuntime, gguf_backend::BackendInitializationError>,
> = OnceLock::new();

struct LocalEndpoint<S> {
    runtime: HostedRuntime<S>,
    thread: Option<RuntimeThread>,
    available: bool,
}

/// Private owner of two concrete, monomorphized E0 endpoints.
pub struct LocalInference {
    candle: LocalEndpoint<CandleLlamaSource>,
    gguf: LocalEndpoint<GgufSource>,
    gguf_backend: GgufBackendRuntime,
    active: Option<ApplicationBackend>,
}

pub enum LocalModelSource {
    Candle(CandleLlamaSource),
    Gguf(GgufSource),
}

pub enum LocalCommand {
    LoadModel {
        ticket: CommandTicket,
        model_id: ModelId,
        source: LocalModelSource,
        device: DeviceId,
    },
    Generate {
        ticket: CommandTicket,
        handle: ModelHandle,
        request: GenerationRequest,
    },
    CancelRequest {
        ticket: CommandTicket,
        request_id: domain_contracts::RequestId,
        reason: CancellationReason,
    },
    UnloadModel {
        ticket: CommandTicket,
        handle: ModelHandle,
        policy: UnloadPolicy,
    },
    #[cfg(test)]
    ShutdownActive { ticket: CommandTicket },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalSubmitError {
    Full,
    Disconnected,
    NoActiveBackend,
    BackendMismatch,
}

impl LocalInference {
    pub(crate) fn start(
        limits: RuntimeLimits,
        hosted: HostedRuntimeConfiguration,
        gguf_configuration: &ApplicationGgufConfiguration,
    ) -> Result<Self, ApplicationError> {
        let gguf_backend = shared_gguf_backend()?;
        if gguf_configuration.use_mmap && !gguf_backend.supports_mmap() {
            return Err(ApplicationError::InvalidConfiguration(
                ApplicationConfigurationField::GgufMemoryMapping,
            ));
        }
        if gguf_configuration.use_mlock && !gguf_backend.supports_mlock() {
            return Err(ApplicationError::InvalidConfiguration(
                ApplicationConfigurationField::GgufMemoryLocking,
            ));
        }

        let (candle_runtime, candle_thread) =
            start_hosted_runtime(CandleLlamaLoader::new(CANDLE_BACKEND_ID), limits, hosted)
                .map_err(worker_start_failure)?;
        let (gguf_runtime, gguf_thread) = match start_hosted_runtime(
            GgufLoader::new(GGUF_BACKEND_ID, gguf_backend.clone()),
            limits,
            hosted,
        ) {
            Ok(started) => started,
            Err(error) => {
                drop(candle_runtime);
                let _join_result = candle_thread.join();
                return Err(worker_start_failure(error));
            }
        };

        Ok(Self {
            candle: LocalEndpoint {
                runtime: candle_runtime,
                thread: Some(candle_thread),
                available: true,
            },
            gguf: LocalEndpoint {
                runtime: gguf_runtime,
                thread: Some(gguf_thread),
                available: true,
            },
            gguf_backend,
            active: None,
        })
    }

    pub(crate) const fn activate(&mut self, backend: ApplicationBackend) -> bool {
        self.active = Some(backend);
        self.endpoint_available(backend)
    }

    pub(crate) const fn active_backend(&self) -> Option<ApplicationBackend> {
        self.active
    }

    #[expect(
        clippy::too_many_lines,
        reason = "closed static dispatch keeps every E1 command visibly routed to exactly one monomorphized endpoint"
    )]
    pub(crate) fn submit(&mut self, command: LocalCommand) -> Result<(), LocalSubmitError> {
        match command {
            LocalCommand::LoadModel {
                ticket,
                model_id,
                source,
                device,
            } => match source {
                LocalModelSource::Candle(source) => {
                    self.require_active(ApplicationBackend::Candle)?;
                    submit_endpoint(
                        &mut self.candle,
                        RuntimeCommand::LoadModel {
                            ticket,
                            model_id,
                            source,
                            device,
                            device_kind: DeviceKind::Cpu,
                        },
                    )
                }
                LocalModelSource::Gguf(source) => {
                    self.require_active(ApplicationBackend::LlamaCpp)?;
                    submit_endpoint(
                        &mut self.gguf,
                        RuntimeCommand::LoadModel {
                            ticket,
                            model_id,
                            source,
                            device,
                            device_kind: DeviceKind::Cpu,
                        },
                    )
                }
            },
            LocalCommand::Generate {
                ticket,
                handle,
                request,
            } => match self.active.ok_or(LocalSubmitError::NoActiveBackend)? {
                ApplicationBackend::Candle => submit_endpoint(
                    &mut self.candle,
                    RuntimeCommand::Generate {
                        ticket,
                        handle,
                        request,
                    },
                ),
                ApplicationBackend::LlamaCpp => submit_endpoint(
                    &mut self.gguf,
                    RuntimeCommand::Generate {
                        ticket,
                        handle,
                        request,
                    },
                ),
            },
            LocalCommand::CancelRequest {
                ticket,
                request_id,
                reason,
            } => match self.active.ok_or(LocalSubmitError::NoActiveBackend)? {
                ApplicationBackend::Candle => submit_endpoint(
                    &mut self.candle,
                    RuntimeCommand::CancelRequest {
                        ticket,
                        request_id,
                        reason,
                    },
                ),
                ApplicationBackend::LlamaCpp => submit_endpoint(
                    &mut self.gguf,
                    RuntimeCommand::CancelRequest {
                        ticket,
                        request_id,
                        reason,
                    },
                ),
            },
            LocalCommand::UnloadModel {
                ticket,
                handle,
                policy,
            } => match self.active.ok_or(LocalSubmitError::NoActiveBackend)? {
                ApplicationBackend::Candle => submit_endpoint(
                    &mut self.candle,
                    RuntimeCommand::UnloadModel {
                        ticket,
                        handle,
                        policy,
                    },
                ),
                ApplicationBackend::LlamaCpp => submit_endpoint(
                    &mut self.gguf,
                    RuntimeCommand::UnloadModel {
                        ticket,
                        handle,
                        policy,
                    },
                ),
            },
            #[cfg(test)]
            LocalCommand::ShutdownActive { ticket } => {
                match self.active.ok_or(LocalSubmitError::NoActiveBackend)? {
                    ApplicationBackend::Candle => {
                        submit_endpoint(&mut self.candle, RuntimeCommand::Shutdown { ticket })
                    }
                    ApplicationBackend::LlamaCpp => {
                        submit_endpoint(&mut self.gguf, RuntimeCommand::Shutdown { ticket })
                    }
                }
            }
        }
    }

    pub(crate) fn try_receive(&mut self) -> Result<RuntimeEvent, RuntimeReceiveError> {
        match self.active {
            Some(ApplicationBackend::Candle) => receive_endpoint(&mut self.candle),
            Some(ApplicationBackend::LlamaCpp) => receive_endpoint(&mut self.gguf),
            None => Err(RuntimeReceiveError::Timeout),
        }
    }

    #[cfg(test)]
    pub(crate) fn receive_active_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<RuntimeEvent, RuntimeReceiveError> {
        let result = match self.active {
            Some(ApplicationBackend::Candle) => self.candle.runtime.receive_timeout(timeout),
            Some(ApplicationBackend::LlamaCpp) => self.gguf.runtime.receive_timeout(timeout),
            None => return Err(RuntimeReceiveError::Timeout),
        };
        if matches!(result, Err(RuntimeReceiveError::Disconnected)) {
            self.mark_active_disconnected();
        }
        result
    }

    pub(crate) fn pull_token_output<R, F>(&self, consume: F) -> Result<R, OutputPullError>
    where
        F: for<'batch> FnOnce(TokenOutputBatch<'batch, GenerationOutputState>) -> R,
    {
        match self.active {
            Some(ApplicationBackend::Candle) => self.candle.runtime.pull_token_output(consume),
            Some(ApplicationBackend::LlamaCpp) => self.gguf.runtime.pull_token_output(consume),
            None => Err(OutputPullError::Poisoned),
        }
    }

    pub(crate) fn resolve_gguf(
        &self,
        path: &Path,
        configuration: &ApplicationGgufConfiguration,
        maximum_sequences: u32,
    ) -> Result<(ResolvedGgufArtifacts, LocalTokenizer), ApplicationFailure> {
        let canonical_path = std::fs::canonicalize(path)
            .map_err(|error| ApplicationFailure::new(ApplicationFailureKind::ModelSource, error))?;
        let digest_before = sha256_file(&canonical_path)
            .map_err(|error| ApplicationFailure::new(ApplicationFailureKind::ModelSource, error))?;
        let metadata = inspect_metadata(
            &canonical_path,
            gguf_backend::GgufInspectionLimits::default(),
        )
        .map_err(|error| ApplicationFailure::new(ApplicationFailureKind::ModelSource, error))?;
        let digest_after = sha256_file(&canonical_path)
            .map_err(|error| ApplicationFailure::new(ApplicationFailureKind::ModelSource, error))?;
        if digest_before != digest_after {
            return Err(ApplicationFailure::new(
                ApplicationFailureKind::ModelSource,
                format!(
                    "GGUF content changed during metadata inspection: {digest_before} became {digest_after}"
                ),
            ));
        }

        let execution = gguf_execution(configuration, &metadata, maximum_sequences)?;
        let source = GgufSource::new_verified(&canonical_path, execution, digest_after);
        let tokenizer = GgufTokenizer::from_source(self.gguf_backend.clone(), &source)
            .map_err(|error| ApplicationFailure::new(ApplicationFailureKind::Tokenizer, error))?;
        Ok((
            ResolvedGgufArtifacts {
                path: canonical_path,
                digest: digest_after,
                metadata,
                execution,
            },
            LocalTokenizer::Gguf(tokenizer),
        ))
    }

    pub(crate) const fn gguf_runtime(&self) -> &HostedRuntime<GgufSource> {
        &self.gguf.runtime
    }

    pub(crate) const fn candle_runtime(&self) -> &HostedRuntime<CandleLlamaSource> {
        &self.candle.runtime
    }

    pub(crate) const fn take_gguf_thread(&mut self) -> Option<RuntimeThread> {
        self.gguf.thread.take()
    }

    pub(crate) const fn take_candle_thread(&mut self) -> Option<RuntimeThread> {
        self.candle.thread.take()
    }

    #[cfg(test)]
    pub(crate) const fn candle_thread_is_present(&self) -> bool {
        self.candle.thread.is_some()
    }

    #[cfg(test)]
    pub(crate) const fn gguf_thread_is_present(&self) -> bool {
        self.gguf.thread.is_some()
    }

    const fn endpoint_available(&self, backend: ApplicationBackend) -> bool {
        match backend {
            ApplicationBackend::Candle => self.candle.available,
            ApplicationBackend::LlamaCpp => self.gguf.available,
        }
    }

    fn require_active(&self, backend: ApplicationBackend) -> Result<(), LocalSubmitError> {
        if self.active == Some(backend) {
            Ok(())
        } else {
            Err(LocalSubmitError::BackendMismatch)
        }
    }

    #[cfg(test)]
    const fn mark_active_disconnected(&mut self) {
        match self.active {
            Some(ApplicationBackend::Candle) => self.candle.available = false,
            Some(ApplicationBackend::LlamaCpp) => self.gguf.available = false,
            None => {}
        }
    }
}

fn shared_gguf_backend() -> Result<GgufBackendRuntime, ApplicationError> {
    match GGUF_BACKEND.get_or_init(GgufBackendRuntime::initialize) {
        Ok(runtime) => Ok(runtime.clone()),
        Err(error) => Err(ApplicationFailure::new(ApplicationFailureKind::Worker, *error).into()),
    }
}

fn worker_start_failure(error: inference_runtime::HostedRuntimeStartError) -> ApplicationError {
    ApplicationFailure::new(ApplicationFailureKind::Worker, error).into()
}

fn submit_endpoint<S>(
    endpoint: &mut LocalEndpoint<S>,
    command: RuntimeCommand<S>,
) -> Result<(), LocalSubmitError> {
    match endpoint.runtime.try_submit(command) {
        Ok(()) => Ok(()),
        Err(inference_runtime::RuntimeSubmitError::Full(_)) => Err(LocalSubmitError::Full),
        Err(inference_runtime::RuntimeSubmitError::Disconnected(_)) => {
            endpoint.available = false;
            Err(LocalSubmitError::Disconnected)
        }
    }
}

fn receive_endpoint<S>(
    endpoint: &mut LocalEndpoint<S>,
) -> Result<RuntimeEvent, RuntimeReceiveError> {
    let result = endpoint.runtime.try_receive();
    if matches!(result, Err(RuntimeReceiveError::Disconnected)) {
        endpoint.available = false;
    }
    result
}

fn gguf_execution(
    configuration: &ApplicationGgufConfiguration,
    metadata: &GgufMetadata,
    maximum_sequences: u32,
) -> Result<GgufExecutionConfiguration, ApplicationFailure> {
    let context = configuration
        .maximum_context_tokens
        .min(metadata.context_length());
    let prefill = configuration.maximum_prefill_tokens.min(context);
    let micro_batch = configuration.micro_batch_tokens.min(prefill);
    let context = NonZeroU32::new(context).ok_or_else(invalid_gguf_execution)?;
    let prefill = NonZeroU32::new(prefill).ok_or_else(invalid_gguf_execution)?;
    let micro_batch = NonZeroU32::new(micro_batch).ok_or_else(invalid_gguf_execution)?;
    let maximum_sequences =
        NonZeroU32::new(maximum_sequences).ok_or_else(invalid_gguf_execution)?;
    let threads = i32::try_from(configuration.threads)
        .ok()
        .and_then(NonZeroI32::new)
        .ok_or_else(invalid_gguf_execution)?;

    GgufExecutionConfiguration::new(
        context,
        prefill,
        micro_batch,
        maximum_sequences,
        threads,
        threads,
    )
    .map(|execution| {
        execution
            .with_mmap(configuration.use_mmap)
            .with_mlock(configuration.use_mlock)
    })
    .map_err(|error| ApplicationFailure::new(ApplicationFailureKind::ModelSource, error))
}

fn invalid_gguf_execution() -> ApplicationFailure {
    ApplicationFailure::new(
        ApplicationFailureKind::ModelSource,
        "GGUF execution defaults cannot be represented by the native backend",
    )
}

pub struct ResolvedGgufArtifacts {
    path: PathBuf,
    digest: Sha256Digest,
    metadata: GgufMetadata,
    execution: GgufExecutionConfiguration,
}

impl ResolvedGgufArtifacts {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    pub(crate) const fn metadata(&self) -> &GgufMetadata {
        &self.metadata
    }

    pub(crate) fn matches_descriptor(&self, descriptor: &ModelDescriptor) -> bool {
        descriptor.backend == GGUF_BACKEND_ID
            && descriptor.metadata.architecture == self.metadata.architecture()
            && descriptor.metadata.scalar_type == self.metadata.scalar_type()
            && descriptor.metadata.quantization == self.metadata.quantization()
            && descriptor.metadata.vocabulary_size == self.metadata.vocabulary_size()
            && descriptor.metadata.context_length == self.metadata.context_length()
            && descriptor.capabilities.maximum_context_tokens
                == self.execution.context_tokens_per_sequence().get()
            && descriptor.capabilities.maximum_sequences == self.execution.maximum_sequences().get()
            && descriptor.capabilities.maximum_prefill_batch
                == self.execution.maximum_prefill_batch().get()
    }

    pub(crate) fn source(&self) -> GgufSource {
        GgufSource::new_verified(&self.path, self.execution, self.digest)
    }
}

pub enum LocalTokenizer {
    Hf(Box<HfTokenizer>),
    Gguf(GgufTokenizer),
}

impl LocalTokenizer {
    pub(crate) fn owned_decoder(&self, options: DecodeOptions) -> LocalOwnedDecoder {
        match self {
            Self::Hf(tokenizer) => {
                LocalOwnedDecoder::Hf(Box::new(tokenizer.owned_decoder(options)))
            }
            Self::Gguf(tokenizer) => LocalOwnedDecoder::Gguf(tokenizer.owned_decoder(options)),
        }
    }

    pub(crate) fn hf_token_id(&self, spelling: &str) -> Option<TokenId> {
        match self {
            Self::Hf(tokenizer) => tokenizer.token_id(spelling),
            Self::Gguf(_) => None,
        }
    }

    pub(crate) fn gguf_digest(&self) -> Option<Sha256Digest> {
        match self {
            Self::Gguf(tokenizer) => Some(tokenizer.content_digest()),
            Self::Hf(_) => None,
        }
    }

    pub(crate) const fn backend(&self) -> ApplicationBackend {
        match self {
            Self::Hf(_) => ApplicationBackend::Candle,
            Self::Gguf(_) => ApplicationBackend::LlamaCpp,
        }
    }
}

impl Tokenizer for LocalTokenizer {
    fn vocabulary_size(&self) -> u32 {
        match self {
            Self::Hf(tokenizer) => tokenizer.vocabulary_size(),
            Self::Gguf(tokenizer) => tokenizer.vocabulary_size(),
        }
    }

    fn encode<S: tokenization::TokenSink>(
        &self,
        text: &str,
        options: EncodeOptions,
        output: &mut S,
    ) -> Result<EncodeReport, TokenizationError> {
        match self {
            Self::Hf(tokenizer) => tokenizer.encode(text, options, output),
            Self::Gguf(tokenizer) => tokenizer.encode(text, options, output),
        }
    }

    fn decode_token<S: tokenization::ByteSink>(
        &self,
        token: TokenId,
        options: DecodeOptions,
        output: &mut S,
    ) -> Result<DecodeReport, TokenizationError> {
        match self {
            Self::Hf(tokenizer) => tokenizer.decode_token(token, options, output),
            Self::Gguf(tokenizer) => tokenizer.decode_token(token, options, output),
        }
    }
}

pub enum LocalOwnedDecoder {
    Hf(Box<HfOwnedStreamingDecoder>),
    Gguf(GgufOwnedStreamingDecoder),
}

impl StreamingDecoder for LocalOwnedDecoder {
    fn step<S: TextSink>(
        &mut self,
        token: TokenId,
        output: &mut S,
    ) -> Result<DecodeReport, TokenizationError> {
        match self {
            Self::Hf(decoder) => decoder.step(token, output),
            Self::Gguf(decoder) => decoder.step(token, output),
        }
    }
}
