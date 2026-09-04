use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Instant,
};

use milkdrift_authority::{AuthorityBudget, CapabilityExecutionRequirements, NetworkProfileRef};
use milkdrift_capability::{
    AdmissionBound, AdmissionConstraints, BoundedJson, CancellationAcknowledgement,
    CancellationBehavior, CancellationRequest, CapabilityCategory, CapabilityDescriptor,
    CapabilityId, CapabilityObservation, DescriptorBuilder, ErrorClass, FeatureContract, FeatureId,
    IdempotencyBehavior, InvocationAdmissionEnvelope, InvocationEvent, InvocationEventKind,
    InvocationRequest, InvocationTerminal, InvocationValueReference, Locality, OperationContract,
    OperationId, SchemaContract, SchemaId, SideEffectClass, StreamingMode, TerminalStatus,
    TrustZone, UsageObservation,
};
use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter, InvocationDataAccess,
    MaterializationLimits, SecretResolver,
};
use milkdrift_model::{
    ContentPart, ContextManifest, ContextManifestDocument, ContextSource, MAX_MODEL_OUTPUT_UNITS,
    MODEL_GENERATE_OPERATION, MODEL_TASK_INPUT_NAME, ModelResponse, ModelResponseDocument,
    ModelTaskRequest, ModelTaskRequestDocument, SessionSelection,
};
use milkdrift_workspace::ContentDigest;
use reqwest::header::{HeaderName, HeaderValue};
use serde_json::json;

use crate::{
    EndpointProfile, ModelFeature, ProviderProtocol, anthropic,
    http::{self, HttpError},
    openai_compatible,
    profile::AuthMode,
};

mod report;

use report::{ProviderFailure, report_failure, report_uncertain};

const CONTEXT_MEDIA: &str = "application/vnd.milkdrift.context-manifest.v2+json";
const RESPONSE_MEDIA: &str = "application/vnd.milkdrift.model-response.v1+json";
const TOOL_CALLS_MEDIA: &str = "application/vnd.milkdrift.model-tool-calls.v1+json";
const STRUCTURED_MEDIA: &str = "application/vnd.milkdrift.model-structured-output.v1+json";
const PROVIDER_METADATA_MEDIA: &str = "application/vnd.milkdrift.model-provider-metadata.v1+json";

fn verify_context_bytes(
    entry: &milkdrift_model::ContextManifestEntry,
    bytes: &[u8],
) -> Result<(), AdapterError> {
    let expected_bytes = if entry.selected_artifact_bytes() > 0 {
        entry.selected_artifact_bytes()
    } else {
        entry.selected_bytes()
    };
    if u64::try_from(bytes.len()) != Ok(expected_bytes)
        || ContentDigest::for_bytes(bytes) != entry.content_digest()
    {
        return Err(AdapterError::rejected(
            "selected context content contradicts the frozen manifest",
        ));
    }
    Ok(())
}

/// Exact selected content after host-owned integrity verification.
pub(crate) enum MaterializedContextPart {
    Text {
        label: String,
        text: String,
    },
    Image {
        label: String,
        media_type: String,
        bytes: Vec<u8>,
    },
}

/// Immutable state shared by every phase of one entered provider request.
struct ModelExecution<'a> {
    context: &'a milkdrift_capability_host::AdapterExecutionContext,
    request: &'a InvocationRequest,
    task: &'a ModelTaskRequest,
    context_manifest: &'a str,
    context_parts: &'a [MaterializedContextPart],
    secret: Option<&'a [u8]>,
    cancelled: &'a AtomicBool,
    reporter: &'a dyn AdapterReporter,
    started: Instant,
    materialization: &'a RefCell<MaterializationLedger>,
}

struct MaterializationLedger {
    files: u32,
    bytes: u64,
    limits: MaterializationLimits,
}

impl MaterializationLedger {
    const fn new(limits: MaterializationLimits) -> Self {
        Self {
            files: 0,
            bytes: 0,
            limits,
        }
    }

    fn record(&mut self, bytes: usize) -> Result<(), AdapterError> {
        let bytes = u64::try_from(bytes)
            .map_err(|_| AdapterError::rejected("model materialization exceeds the platform"))?;
        self.files = self
            .files
            .checked_add(1)
            .ok_or_else(|| AdapterError::rejected("model materialization file count overflowed"))?;
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| AdapterError::rejected("model materialization byte count overflowed"))?;
        if self.files > self.limits.max_files
            || bytes > self.limits.max_file_bytes
            || self.bytes > self.limits.max_total_bytes
        {
            return Err(AdapterError::rejected(
                "model materialization exceeds the aggregate file or byte bound",
            ));
        }
        Ok(())
    }
}

struct ActiveInvocationGuard<'a> {
    active: &'a Mutex<BTreeMap<milkdrift_capability::InvocationId, Arc<AtomicBool>>>,
    invocation: milkdrift_capability::InvocationId,
}

impl Drop for ActiveInvocationGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.invocation);
        }
    }
}

struct PreparedOutput {
    name: &'static str,
    media_type: &'static str,
    bytes: Vec<u8>,
}

/// One synchronous host adapter for an exact endpoint-profile revision.
pub struct ModelEndpointAdapter {
    capability: CapabilityId,
    profile: EndpointProfile,
    client: reqwest::blocking::Client,
    secrets: Arc<dyn SecretResolver>,
    data: Arc<dyn InvocationDataAccess>,
    active: Mutex<BTreeMap<milkdrift_capability::InvocationId, Arc<AtomicBool>>>,
    lifecycle: AtomicU8,
    authority_requirements: CapabilityExecutionRequirements,
}

impl ModelEndpointAdapter {
    /// Creates an adapter after endpoint policy and HTTP client construction succeed.
    pub fn new(
        capability: CapabilityId,
        profile: EndpointProfile,
        secrets: Arc<dyn SecretResolver>,
        data: Arc<dyn InvocationDataAccess>,
    ) -> Result<Self, AdapterError> {
        let client =
            http::client(&profile).map_err(|error| AdapterError::rejected(error.to_string()))?;
        let endpoint = profile
            .endpoint_url()
            .map_err(|error| AdapterError::rejected(error.to_string()))?;
        let destination = network_destination(&endpoint)?;
        let network_profile = NetworkProfileRef::new(profile.identity().as_str().to_owned())
            .map_err(|error| AdapterError::rejected(error.to_string()))?;
        let limits = profile.limits();
        let artifact_bytes = limits
            .max_response_bytes
            .checked_mul(8)
            .ok_or_else(|| AdapterError::rejected("model artifact byte ceiling overflows"))?;
        let required_secrets = match profile.auth() {
            AuthMode::NoAuth => BTreeSet::new(),
            AuthMode::Bearer { secret } | AuthMode::AnthropicApiKey { secret } => {
                BTreeSet::from([secret.clone()])
            }
        };
        let authority_requirements = CapabilityExecutionRequirements {
            network_profiles: BTreeSet::from([network_profile]),
            network_destinations: BTreeSet::from([destination]),
            secrets: required_secrets,
            budget: AuthorityBudget {
                duration_ms: Some(limits.request_timeout_ms),
                invocations: Some(1),
                artifact_bytes: Some(artifact_bytes),
                units: Some(MAX_MODEL_OUTPUT_UNITS),
                concurrency: Some(1),
                ..AuthorityBudget::default()
            },
            ..CapabilityExecutionRequirements::default()
        };
        Ok(Self {
            capability,
            profile,
            client,
            secrets,
            data,
            active: Mutex::new(BTreeMap::new()),
            lifecycle: AtomicU8::new(Lifecycle::Created as u8),
            authority_requirements,
        })
    }

    fn execute_inner(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let lifecycle = self.lifecycle.load(Ordering::SeqCst);
        if lifecycle != Lifecycle::Started as u8 && lifecycle != Lifecycle::Draining as u8 {
            return Err(AdapterError::unavailable(
                "model endpoint adapter is not accepting exact work",
            ));
        }
        let context = invocation.context().ok_or_else(|| {
            AdapterError::rejected("model invocation requires durable execution context")
        })?;
        let request = invocation.request();
        if request.operation().as_str() != MODEL_GENERATE_OPERATION
            || request.provider_profile() != Some(self.profile.identity())
        {
            return Err(AdapterError::rejected(
                "model invocation does not match the exact operation/profile",
            ));
        }
        let manifest_ref = request.context_manifest().ok_or_else(|| {
            AdapterError::rejected("model invocation requires a frozen context manifest")
        })?;
        if manifest_ref.media_type() != Some(CONTEXT_MEDIA) {
            return Err(AdapterError::rejected(
                "context manifest media type is unsupported",
            ));
        }
        let limits = self.materialization_limits();
        let mut materialization = MaterializationLedger::new(limits);
        let manifest_bytes = self
            .data
            .read_artifact_bytes(context, manifest_ref, limits)
            .map_err(|error| AdapterError::rejected(error.to_string()))?;
        materialization.record(manifest_bytes.len())?;
        let manifest = ContextManifestDocument::from_json(&manifest_bytes)
            .map_err(|_| AdapterError::rejected("context manifest is malformed"))?;
        if manifest.body().run() != context.run()
            || manifest.body().revision() != context.revision()
            || manifest.body().node() != context.node()
            || manifest.body().execution() != context.execution()
            || manifest.body().attempt() != context.attempt()
        {
            return Err(AdapterError::rejected(
                "context manifest provenance does not match this exact attempt",
            ));
        }
        let manifest_text = std::str::from_utf8(&manifest_bytes)
            .map_err(|_| AdapterError::rejected("context manifest is not canonical UTF-8"))?;
        let context_parts = self.load_context_parts(
            context,
            request,
            manifest.body(),
            limits,
            &mut materialization,
        )?;
        let task = self.load_task(context, request, limits, &mut materialization)?;
        self.negotiate(&task, &context_parts)?;
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let mut active = self.active.lock().map_err(|_| {
                AdapterError::unavailable("model cancellation state is unavailable")
            })?;
            if active.contains_key(request.invocation()) {
                return Err(AdapterError::rejected("duplicate active model invocation"));
            }
            active.insert(request.invocation().clone(), cancelled.clone());
        }
        let _active_guard = ActiveInvocationGuard {
            active: &self.active,
            invocation: request.invocation().clone(),
        };
        let materialization = RefCell::new(materialization);
        let started = Instant::now();
        match self.profile.auth() {
            AuthMode::NoAuth => self.perform(ModelExecution {
                context,
                request,
                task: &task,
                context_manifest: manifest_text,
                context_parts: &context_parts,
                secret: None,
                cancelled: &cancelled,
                reporter,
                started,
                materialization: &materialization,
            }),
            AuthMode::Bearer { secret } | AuthMode::AnthropicApiKey { secret } => {
                match self.secrets.resolve(secret) {
                    Err(_) => Err(AdapterError::rejected(
                        "model endpoint secret reference is unavailable",
                    )),
                    Ok(secret) if secret.is_empty() => {
                        Err(AdapterError::rejected("model endpoint secret is empty"))
                    }
                    Ok(secret) => secret.expose(|bytes| {
                        self.perform(ModelExecution {
                            context,
                            request,
                            task: &task,
                            context_manifest: manifest_text,
                            context_parts: &context_parts,
                            secret: Some(bytes),
                            cancelled: &cancelled,
                            reporter,
                            started,
                            materialization: &materialization,
                        })
                    }),
                }
            }
        }
    }

    fn perform(&self, execution: ModelExecution<'_>) -> Result<(), AdapterError> {
        let ModelExecution {
            context,
            request,
            task,
            context_manifest,
            context_parts,
            secret,
            cancelled,
            reporter,
            started,
            materialization,
        } = execution;
        let load = |reference: &milkdrift_capability::ArtifactReference| {
            let bytes = self
                .data
                .read_artifact_bytes(context, reference, self.materialization_limits())
                .map_err(|_| HttpError::Policy("referenced model content is unavailable"))?;
            materialization
                .borrow_mut()
                .record(bytes.len())
                .map_err(|_| {
                    HttpError::Policy("referenced model content exceeds aggregate bounds")
                })?;
            Ok(bytes)
        };
        let wire = match self.profile.protocol() {
            ProviderProtocol::OpenAiCompatible { .. } => openai_compatible::request(
                task,
                self.profile.model(),
                context_manifest,
                context_parts,
                self.profile.provider_options(),
                load,
            ),
            ProviderProtocol::Anthropic { .. } => anthropic::request(
                task,
                self.profile.model(),
                context_manifest,
                context_parts,
                self.profile.provider_options(),
                load,
            ),
        }
        .map_err(|error| AdapterError::rejected(error.to_string()))?;
        let wire = serde_json::to_vec(&wire)
            .map_err(|_| AdapterError::rejected("model request encoding failed"))?;
        if u64::try_from(wire.len())
            .map_or(true, |size| size > self.profile.limits().max_request_bytes)
        {
            return Err(AdapterError::rejected(
                "encoded model request exceeds the endpoint request-body bound",
            ));
        }
        let url = self
            .profile
            .endpoint_url()
            .map_err(|_| AdapterError::rejected("endpoint URL is invalid"))?;
        let mut headers = http::headers(&self.profile, secret)
            .map_err(|error| AdapterError::rejected(error.to_string()))?;
        if let ProviderProtocol::Anthropic { version, .. } = self.profile.protocol() {
            headers.insert(
                HeaderName::from_static("anthropic-version"),
                HeaderValue::from_str(version).map_err(|_| {
                    AdapterError::rejected("Anthropic version is not a valid header value")
                })?,
            );
        }
        let response = self.client.post(url).headers(headers).body(wire).send();
        let response = match response {
            Ok(response) => response,
            Err(_) => {
                if cancelled.load(Ordering::SeqCst) {
                    return report_uncertain(
                        request,
                        reporter,
                        1,
                        ProviderFailure {
                            class: ErrorClass::Unknown,
                            retryable: false,
                            code: "model_cancellation_unconfirmed",
                            message: "model cancellation was requested without provider-side terminal evidence",
                        },
                        started,
                    );
                }
                return Err(AdapterError::external_failure(
                    "model endpoint transport failed after request entry",
                ));
            }
        };
        if !response.status().is_success() {
            let mapped = http::status_error(response.status());
            return report_failure(
                request,
                reporter,
                1,
                ProviderFailure {
                    class: mapped.class,
                    retryable: mapped.retryable,
                    code: mapped.code,
                    message: "model endpoint rejected the request",
                },
                started,
            );
        }
        let mut sequence = 1u64;
        let parsed = if task.streaming() {
            match self.profile.protocol() {
                ProviderProtocol::OpenAiCompatible { .. } => {
                    let mut state = openai_compatible::StreamState::new();
                    let result = http::read_sse(&self.profile, response, cancelled, |data| {
                        state.event(data, |fragment| {
                            report_fragment(
                                request,
                                reporter,
                                &mut sequence,
                                fragment,
                                self.profile.limits().max_fragment_bytes,
                            )
                        })
                    });
                    result.and_then(|()| state.complete(task.structured_output().is_some()))
                }
                ProviderProtocol::Anthropic { .. } => {
                    let mut state = anthropic::StreamState::new();
                    let result = http::read_sse(&self.profile, response, cancelled, |data| {
                        state.event(data, |fragment| {
                            report_fragment(
                                request,
                                reporter,
                                &mut sequence,
                                fragment,
                                self.profile.limits().max_fragment_bytes,
                            )
                        })
                    });
                    result.and_then(|()| state.complete())
                }
            }
        } else {
            http::read_json(&self.profile, response).and_then(|value| {
                match self.profile.protocol() {
                    ProviderProtocol::OpenAiCompatible { .. } => {
                        openai_compatible::response(&value, task.structured_output().is_some())
                    }
                    ProviderProtocol::Anthropic { .. } => anthropic::response(&value),
                }
            })
        };
        let response = match parsed {
            Ok(value) => value,
            Err(HttpError::Cancelled) => {
                return report_uncertain(
                    request,
                    reporter,
                    sequence,
                    ProviderFailure {
                        class: ErrorClass::Unknown,
                        retryable: false,
                        code: "model_cancellation_unconfirmed",
                        message: "model cancellation was requested without provider-side terminal evidence",
                    },
                    started,
                );
            }
            Err(error) => {
                return report_uncertain(
                    request,
                    reporter,
                    sequence,
                    ProviderFailure {
                        class: error.class(),
                        retryable: false,
                        code: error.code(),
                        message: "model response ended without trustworthy terminal evidence",
                    },
                    started,
                );
            }
        };
        self.publish_response(context, request, reporter, response, sequence, started)
    }

    fn load_context_parts(
        &self,
        context: &milkdrift_capability_host::AdapterExecutionContext,
        request: &InvocationRequest,
        manifest: &ContextManifest,
        limits: MaterializationLimits,
        materialization: &mut MaterializationLedger,
    ) -> Result<Vec<MaterializedContextPart>, AdapterError> {
        let mut expected = BTreeSet::new();
        let mut parts = Vec::new();
        for entry in manifest.entries() {
            if let ContextSource::DirectInput { name, reference } = entry.source() {
                let input = request
                    .inputs()
                    .iter()
                    .find(|input| input.name() == name && input.value() == reference)
                    .ok_or_else(|| {
                        AdapterError::rejected(
                            "direct context input contradicts the frozen manifest",
                        )
                    })?;
                let bytes = self
                    .data
                    .read_input_bytes(context, input, limits)
                    .map_err(|_| AdapterError::rejected("direct context content is unavailable"))?;
                materialization.record(bytes.len())?;
                verify_context_bytes(entry, &bytes)?;
                continue;
            }
            let name = format!(
                "{}{:04}",
                milkdrift_capability::CONTEXT_ITEM_INPUT_PREFIX,
                entry.ordinal()
            );
            expected.insert(name.clone());
            let input = request
                .inputs()
                .iter()
                .find(|input| input.name() == name)
                .ok_or_else(|| AdapterError::rejected("selected context input is missing"))?;
            let bytes = self
                .data
                .read_input_bytes(context, input, limits)
                .map_err(|_| AdapterError::rejected("selected context content is unavailable"))?;
            materialization.record(bytes.len())?;
            verify_context_bytes(entry, &bytes)?;
            let media_type = match entry.source() {
                ContextSource::Artifact { reference } => reference.media_type().as_str(),
                ContextSource::DirectInput { .. }
                | ContextSource::NodeExecution { .. }
                | ContextSource::Event { .. }
                | ContextSource::WorkspaceValue { .. } => "application/json",
            };
            let label = serde_json::to_string(&serde_json::json!({
                "ordinal": entry.ordinal(),
                "kind": entry.kind(),
                "semantic_roles": entry.semantic_roles(),
                "source": entry.source(),
                "content_digest": entry.content_digest(),
            }))
            .map_err(|_| AdapterError::rejected("context provenance encoding failed"))?;
            if media_type.starts_with("text/")
                || media_type == "application/json"
                || media_type.ends_with("+json")
            {
                let text = String::from_utf8(bytes).map_err(|_| {
                    AdapterError::rejected("selected textual context is not valid UTF-8")
                })?;
                parts.push(MaterializedContextPart::Text { label, text });
            } else if media_type.starts_with("image/") {
                parts.push(MaterializedContextPart::Image {
                    label,
                    media_type: media_type.to_owned(),
                    bytes,
                });
            } else {
                return Err(AdapterError::rejected(
                    "selected binary context has no safe provider mapping",
                ));
            }
        }
        if request.inputs().iter().any(|input| {
            input
                .name()
                .starts_with(milkdrift_capability::CONTEXT_ITEM_INPUT_PREFIX)
                && input.name() != milkdrift_model::CONTEXT_MANIFEST_INPUT_NAME
                && !expected.contains(input.name())
        }) {
            return Err(AdapterError::rejected(
                "invocation contains context outside the frozen manifest",
            ));
        }
        Ok(parts)
    }

    fn publish_response(
        &self,
        context: &milkdrift_capability_host::AdapterExecutionContext,
        request: &InvocationRequest,
        reporter: &dyn AdapterReporter,
        response: ModelResponse,
        mut sequence: u64,
        started: Instant,
    ) -> Result<(), AdapterError> {
        let limits = self.materialization_limits();
        let canonical = ModelResponseDocument::new(response.clone())
            .to_canonical_json()
            .map_err(|_| {
                AdapterError::external_failure("canonical model response encoding failed")
            })?;
        let mut outputs = vec![PreparedOutput {
            name: "model_response",
            media_type: RESPONSE_MEDIA,
            bytes: canonical,
        }];
        if !response.text().is_empty() {
            outputs.push(PreparedOutput {
                name: "final_text",
                media_type: "text/plain",
                bytes: response.text().as_bytes().to_vec(),
            });
        }
        if let Some(structured) = response.structured() {
            let bytes = serde_json::to_vec(structured.value())
                .map_err(|_| AdapterError::external_failure("structured output encoding failed"))?;
            outputs.push(PreparedOutput {
                name: "structured_output",
                media_type: STRUCTURED_MEDIA,
                bytes,
            });
        }
        if !response.tool_calls().is_empty() {
            let bytes = serde_json::to_vec(response.tool_calls())
                .map_err(|_| AdapterError::external_failure("tool call encoding failed"))?;
            outputs.push(PreparedOutput {
                name: "tool_calls",
                media_type: TOOL_CALLS_MEDIA,
                bytes,
            });
        }
        if !response.provider_metadata().is_empty() {
            let bytes = serde_json::to_vec(response.provider_metadata())
                .map_err(|_| AdapterError::external_failure("provider metadata encoding failed"))?;
            outputs.push(PreparedOutput {
                name: "provider_metadata",
                media_type: PROVIDER_METADATA_MEDIA,
                bytes,
            });
        }
        let mut output_ledger = MaterializationLedger::new(limits);
        if outputs
            .iter()
            .try_for_each(|output| output_ledger.record(output.bytes.len()))
            .is_err()
        {
            return report_failure(
                request,
                reporter,
                sequence,
                ProviderFailure {
                    class: ErrorClass::Adapter,
                    retryable: false,
                    code: "artifact_output_bounds",
                    message: "aggregate model output artifacts exceed the configured bound",
                },
                started,
            );
        }
        let mut published = Vec::with_capacity(outputs.len());
        for output in outputs {
            let reference = match self.data.publish_bytes(
                context,
                request,
                output.name,
                output.media_type,
                &output.bytes,
                limits,
            ) {
                Ok(value) => value,
                Err(_) => {
                    return report_failure(
                        request,
                        reporter,
                        sequence,
                        ProviderFailure {
                            class: ErrorClass::Adapter,
                            retryable: false,
                            code: "artifact_publication",
                            message: "model output artifact publication failed",
                        },
                        started,
                    );
                }
            };
            published.push((output.name, reference));
        }
        let output_refs = published
            .iter()
            .map(|(_, reference)| reference.clone())
            .collect::<Vec<_>>();
        for (name, reference) in published {
            reporter.invocation(
                InvocationEvent::new(
                    request.invocation().clone(),
                    sequence,
                    InvocationEventKind::Output {
                        name: name.to_owned(),
                        reference,
                    },
                )
                .map_err(|_| AdapterError::external_failure("invalid model output event"))?,
            )?;
            sequence = sequence
                .checked_add(1)
                .ok_or_else(|| AdapterError::external_failure("model report sequence overflow"))?;
        }
        let duration = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let usage = response.usage();
        let observed = UsageObservation::new(
            usage.input_units,
            usage.output_units,
            Some(duration),
            usage.cost_micros,
            usage.currency.clone(),
            BTreeMap::new(),
        )
        .map_err(|_| AdapterError::external_failure("invalid model usage observation"))?;
        let terminal = InvocationTerminal::new(
            TerminalStatus::Success,
            output_refs,
            None,
            Some(observed),
            SideEffectClass::Unknown,
        )
        .map_err(|_| AdapterError::external_failure("invalid model terminal event"))?;
        reporter.invocation(
            InvocationEvent::new(
                request.invocation().clone(),
                sequence,
                InvocationEventKind::Terminal { terminal },
            )
            .map_err(|_| AdapterError::external_failure("invalid model terminal event"))?,
        )
    }

    fn load_task(
        &self,
        context: &milkdrift_capability_host::AdapterExecutionContext,
        request: &InvocationRequest,
        limits: MaterializationLimits,
        materialization: &mut MaterializationLedger,
    ) -> Result<ModelTaskRequest, AdapterError> {
        let input = request
            .inputs()
            .iter()
            .find(|input| input.name() == MODEL_TASK_INPUT_NAME)
            .ok_or_else(|| AdapterError::rejected("model task input is missing"))?;
        let bytes = match input.value() {
            InvocationValueReference::Inline { value } => serde_json::to_vec(value.value())
                .map_err(|_| AdapterError::rejected("model task input cannot be encoded"))?,
            InvocationValueReference::Artifact { reference } => {
                let bytes = self
                    .data
                    .read_artifact_bytes(context, reference, limits)
                    .map_err(|_| AdapterError::rejected("model task artifact is unavailable"))?;
                materialization.record(bytes.len())?;
                bytes
            }
            InvocationValueReference::WorkspaceValue { .. } => {
                return Err(AdapterError::rejected(
                    "model task must be an inline or immutable artifact document",
                ));
            }
        };
        ModelTaskRequestDocument::from_json(&bytes)
            .map(|document| document.body().clone())
            .map_err(|_| AdapterError::rejected("model task contract is malformed"))
    }

    fn negotiate(
        &self,
        task: &ModelTaskRequest,
        context_parts: &[MaterializedContextPart],
    ) -> Result<(), AdapterError> {
        let features = self.profile.features();
        let require = |feature, reason| {
            if features.contains(&feature) {
                Ok(())
            } else {
                Err(AdapterError::rejected(reason))
            }
        };
        require(
            ModelFeature::SystemRole,
            "profile does not advertise the system role required for the frozen context manifest",
        )?;
        if context_parts
            .iter()
            .any(|part| matches!(part, MaterializedContextPart::Image { .. }))
        {
            require(
                ModelFeature::Images,
                "profile does not advertise selected context image mapping",
            )?;
        }
        if task.streaming() {
            require(
                ModelFeature::Streaming,
                "profile does not advertise streaming",
            )?;
        }
        if !task.tools().is_empty()
            || task
                .messages()
                .iter()
                .any(|message| message.role() == milkdrift_model::MessageRole::ToolResult)
        {
            require(
                ModelFeature::Tools,
                "profile does not advertise tool mapping",
            )?;
        }
        if task.structured_output().is_some() {
            require(
                ModelFeature::StructuredOutput,
                "profile does not advertise structured output",
            )?;
        }
        if task.reasoning().is_some() {
            require(
                ModelFeature::Reasoning,
                "profile does not advertise reasoning controls",
            )?;
        }
        for message in task.messages() {
            match message.role() {
                milkdrift_model::MessageRole::System => require(
                    ModelFeature::SystemRole,
                    "profile does not advertise system role",
                )?,
                milkdrift_model::MessageRole::Developer => require(
                    ModelFeature::DeveloperRole,
                    "profile does not advertise developer role",
                )?,
                _ => {}
            }
            for part in message.parts() {
                match part {
                    ContentPart::Image { .. } => {
                        require(ModelFeature::Images, "profile does not advertise images")?
                    }
                    ContentPart::File { .. } | ContentPart::Artifact { .. } => {
                        require(ModelFeature::Files, "profile does not advertise files")?
                    }
                    ContentPart::Text { .. } => {}
                }
            }
        }
        match task.session() {
            SessionSelection::Fresh => {}
            SessionSelection::ExplicitContinuation { .. } => {
                return Err(AdapterError::rejected(
                    "explicit continuation artifacts have no configured protocol mapping",
                ));
            }
            SessionSelection::ProviderManaged { .. } => {
                return Err(AdapterError::rejected(
                    "provider-managed sessions have no configured protocol mapping",
                ));
            }
        }
        Ok(())
    }

    fn materialization_limits(&self) -> MaterializationLimits {
        MaterializationLimits {
            max_files: 256,
            max_file_bytes: self.profile.limits().max_response_bytes,
            max_total_bytes: self.profile.limits().max_response_bytes.saturating_mul(4),
            max_path_bytes: 4096,
            max_directory_depth: 64,
            chunk_bytes: 1_048_576,
        }
    }
}

impl CapabilityAdapter for ModelEndpointAdapter {
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        self.authority_requirements.clone()
    }

    fn admission_envelope(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        Ok(InvocationAdmissionEnvelope::new(
            AdmissionBound::Unknown,
            AdmissionBound::Unknown,
            AdmissionBound::Bounded(self.profile.limits().max_response_bytes.saturating_mul(4)),
            AdmissionBound::Unknown,
        ))
    }

    fn start(&self) -> Result<(), AdapterError> {
        loop {
            let prior = self.lifecycle.load(Ordering::SeqCst);
            if prior == Lifecycle::Started as u8 {
                return Ok(());
            }
            if prior != Lifecycle::Created as u8 {
                return Err(AdapterError::rejected(
                    "model endpoint adapter cannot restart after drain or shutdown",
                ));
            }
            if self
                .lifecycle
                .compare_exchange(
                    prior,
                    Lifecycle::Started as u8,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                return Ok(());
            }
        }
    }
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        self.execute_inner(invocation, reporter)
    }
    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        let active = self
            .active
            .lock()
            .map_err(|_| AdapterError::unavailable("model cancellation state is unavailable"))?;
        let accepted = active.get(request.invocation()).is_some_and(|flag| {
            flag.store(true, Ordering::SeqCst);
            true
        });
        CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            accepted,
            false,
            Some(
                if accepted {
                    "request cancellation signalled; provider acknowledgement is not available"
                } else {
                    "invocation is not active"
                }
                .to_owned(),
            ),
        )
        .map_err(|_| AdapterError::external_failure("invalid cancellation acknowledgement"))
    }
    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        let load = u32::try_from(
            self.active
                .lock()
                .map_err(|_| AdapterError::unavailable("model state is unavailable"))?
                .len(),
        )
        .unwrap_or(u32::MAX);
        CapabilityObservation::new(
            self.capability.clone(),
            observed_at_unix_ms,
            self.lifecycle.load(Ordering::SeqCst) == Lifecycle::Started as u8,
            load,
            "configured endpoint; no discovery request performed",
        )
        .map_err(|_| AdapterError::unavailable("invalid model health observation"))
    }
    fn begin_drain(&self) -> Result<(), AdapterError> {
        loop {
            let prior = self.lifecycle.load(Ordering::SeqCst);
            if prior == Lifecycle::Draining as u8 {
                return Ok(());
            }
            if prior != Lifecycle::Started as u8 {
                return Err(AdapterError::rejected(
                    "model endpoint adapter must be started before drain",
                ));
            }
            if self
                .lifecycle
                .compare_exchange(
                    prior,
                    Lifecycle::Draining as u8,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                return Ok(());
            }
        }
    }
    fn shutdown(&self) -> Result<(), AdapterError> {
        self.lifecycle
            .store(Lifecycle::Stopped as u8, Ordering::SeqCst);
        if let Ok(active) = self.active.lock() {
            for flag in active.values() {
                flag.store(true, Ordering::SeqCst);
            }
        }
        Ok(())
    }
}

#[repr(u8)]
enum Lifecycle {
    Created = 0,
    Started = 1,
    Draining = 2,
    Stopped = 3,
}

fn network_destination(endpoint: &url::Url) -> Result<String, AdapterError> {
    let host = endpoint
        .host()
        .ok_or_else(|| AdapterError::rejected("model endpoint host is absent"))?;
    let host = match host {
        url::Host::Ipv6(address) => format!("[{address}]"),
        url::Host::Ipv4(address) => address.to_string(),
        url::Host::Domain(name) => name.to_owned(),
    };
    let port = endpoint
        .port_or_known_default()
        .ok_or_else(|| AdapterError::rejected("model endpoint port is absent"))?;
    Ok(format!("{host}:{port}"))
}

/// Creates the immutable capability descriptor corresponding exactly to a profile.
pub fn descriptor_for_profile(
    capability: CapabilityId,
    profile: &EndpointProfile,
) -> Result<CapabilityDescriptor, milkdrift_capability::ContractError> {
    let profile_bytes = profile
        .to_canonical_json()
        .map_err(|error| milkdrift_capability::ContractError::InvalidContract(error.to_string()))?;
    let protocol_family = match profile.protocol() {
        ProviderProtocol::OpenAiCompatible { .. } => "open_ai_compatible",
        ProviderProtocol::Anthropic { .. } => "anthropic",
    };
    let endpoint_origin = profile
        .redacted_origin()
        .map_err(|error| milkdrift_capability::ContractError::InvalidContract(error.to_string()))?;
    let provenance = BoundedJson::new(json!({
        "profile_digest": format!("b3_{}", blake3::hash(&profile_bytes)),
        "revision": profile.revision(),
        "protocol_family": protocol_family,
        "model_alias": profile.model(),
        "endpoint_origin": endpoint_origin,
    }))?;
    let input = SchemaContract::new(
        SchemaId::new("milkdrift.model-task")?,
        1,
        BoundedJson::new(json!({"type":"object"}))?,
    )?;
    let output = SchemaContract::new(
        SchemaId::new("milkdrift.model-response")?,
        1,
        BoundedJson::new(json!({"type":"object"}))?,
    )?;
    let mut streaming = BTreeSet::from([StreamingMode::None]);
    if profile.features().contains(&ModelFeature::Streaming) {
        streaming.insert(StreamingMode::OutputFragments);
    }
    let features = profile
        .features()
        .iter()
        .map(|feature| {
            let id = FeatureId::new(match feature {
                ModelFeature::Streaming => "model.streaming",
                ModelFeature::SystemRole => "model.role.system",
                ModelFeature::DeveloperRole => "model.role.developer",
                ModelFeature::Tools => "model.tools",
                ModelFeature::StructuredOutput => "model.structured_output",
                ModelFeature::Images => "model.images",
                ModelFeature::Files => "model.files",
                ModelFeature::Reasoning => "model.reasoning",
                ModelFeature::ProviderSessions => "model.provider_sessions",
            })?;
            Ok((id.clone(), FeatureContract::new(id, None)))
        })
        .collect::<Result<BTreeMap<_, _>, milkdrift_capability::ContractError>>()?;
    let operation = OperationContract::new(
        input,
        output,
        streaming,
        CancellationBehavior::BestEffort,
        IdempotencyBehavior::Unsupported,
        SideEffectClass::Unknown,
        features,
    )?;
    let locality = if profile.local_development() {
        Locality::Local
    } else {
        Locality::Remote
    };
    let trust = profile
        .trust_zones()
        .iter()
        .map(|zone| TrustZone::new(zone.clone()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    DescriptorBuilder::new(
        capability,
        profile.revision(),
        CapabilityCategory::Model,
        AdmissionConstraints::new(
            profile.max_concurrent(),
            profile.max_concurrent().saturating_mul(2),
        )?,
        locality,
    )
    .provider_profile(Some(profile.identity().clone()))
    .operations(BTreeMap::from([(
        OperationId::new(MODEL_GENERATE_OPERATION)?,
        operation,
    )]))
    .trust_zones(trust)
    .extensions(BTreeMap::from([(
        milkdrift_capability::ExtensionKey::new("org.milkdrift/model-profile")?,
        provenance,
    )]))
    .build()
}

fn report_fragment(
    request: &InvocationRequest,
    reporter: &dyn AdapterReporter,
    sequence: &mut u64,
    fragment: &str,
    max: u32,
) -> Result<(), HttpError> {
    let mut rest = fragment;
    while !rest.is_empty() {
        let piece = milkdrift_contracts::truncate_utf8(rest, max as usize);
        if piece.is_empty() {
            return Err(HttpError::MalformedResponse);
        }
        reporter
            .invocation(
                InvocationEvent::new(
                    request.invocation().clone(),
                    *sequence,
                    InvocationEventKind::Progress {
                        message: piece.to_owned(),
                        completed_units: None,
                        total_units: None,
                    },
                )
                .map_err(|_| HttpError::MalformedResponse)?,
            )
            .map_err(|_| HttpError::Transport)?;
        *sequence = sequence
            .checked_add(1)
            .ok_or(HttpError::MalformedResponse)?;
        rest = &rest[piece.len()..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> MaterializationLimits {
        MaterializationLimits {
            max_files: 2,
            max_file_bytes: 5,
            max_total_bytes: 8,
            max_path_bytes: 64,
            max_directory_depth: 4,
            chunk_bytes: 4,
        }
    }

    #[test]
    fn aggregate_materialization_ledger_enforces_count_bytes_and_exact_boundary() {
        let mut exact = MaterializationLedger::new(limits());
        assert!(exact.record(3).is_ok());
        assert!(exact.record(5).is_ok());
        assert!(exact.record(0).is_err());

        let mut bytes = MaterializationLedger::new(limits());
        assert!(bytes.record(4).is_ok());
        assert!(bytes.record(5).is_err());

        let mut file = MaterializationLedger::new(limits());
        assert!(file.record(6).is_err());
    }

    #[test]
    fn active_registration_is_removed_during_panic_unwind() -> Result<(), Box<dyn std::error::Error>>
    {
        let invocation = milkdrift_capability::InvocationId::new("invocation-panic-cleanup")?;
        let active = Mutex::new(BTreeMap::from([(
            invocation.clone(),
            Arc::new(AtomicBool::new(false)),
        )]));
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = ActiveInvocationGuard {
                active: &active,
                invocation,
            };
            std::panic::resume_unwind(Box::new("contained test panic"));
        }));
        assert!(unwind.is_err());
        assert!(
            active
                .lock()
                .map_err(|_| "active invocation test lock is poisoned")?
                .is_empty()
        );
        Ok(())
    }
}
