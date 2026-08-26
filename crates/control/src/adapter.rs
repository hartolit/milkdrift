use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use milkdrift_capability::{
    AdmissionConstraints, ArtifactReference, BoundedJson, CancellationAcknowledgement,
    CancellationBehavior, CancellationRequest, CapabilityCategory, CapabilityDescriptor,
    CapabilityId, CapabilityObservation, DescriptorBuilder, ErrorClass, ExtensionKey, FeatureId,
    IdempotencyBehavior, InvocationEvent, InvocationEventKind, InvocationFailure,
    InvocationTerminal, InvocationValueReference, Locality, OperationContract, OperationId,
    SchemaContract, SchemaId, SideEffectClass, StreamingMode, TerminalStatus, TrustZone,
};
use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter,
};
use milkdrift_contracts::{JsonLimits, canonical_json_bytes};
use serde::{Deserialize, Serialize};

use crate::{
    ActorAuthorityContext, ControlCommand, ControlCommandDocument, ControlError, ControlResult,
    ControlService, WORKFLOW_APPLY_OPERATION, WORKFLOW_INSPECT_OPERATION, WORKFLOW_PAUSE_OPERATION,
    WORKFLOW_PROPOSE_OPERATION, WORKFLOW_RESUME_OPERATION, WORKFLOW_RETRY_OPERATION,
    WORKFLOW_SIGNAL_OPERATION,
};

const CONTROL_CAPABILITY_ID: &str = "milkdrift-workflow-control";
const CONTROL_REQUEST_INPUT: &str = "milkdrift.control_request";
const AUTHORITY_CONTEXT_INPUT: &str = "milkdrift.authority_context";

/// Safe opaque lookup identity for caller-authenticated authority context.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(transparent)]
pub struct AuthorityContextRef(String);

impl AuthorityContextRef {
    /// Constructs a bounded safe reference that contains no grant or credential value.
    pub fn new(value: impl Into<String>) -> Result<Self, ControlError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 192
            || !value.is_ascii()
            || !value.as_bytes()[0].is_ascii_alphanumeric()
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
            })
        {
            return Err(ControlError::InvalidIdentity {
                kind: "AuthorityContextRef",
                reason: "must contain 1..=192 safe ASCII identity bytes".to_owned(),
            });
        }
        Ok(Self(value))
    }

    /// Returns the opaque lookup reference.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Trusted resolver that maps an opaque safe reference to immutable actor/grant context.
pub trait AuthorityContextResolver: Send + Sync {
    /// Resolves context outside model-controlled text and artifacts.
    fn resolve(
        &self,
        reference: &AuthorityContextRef,
    ) -> Result<ActorAuthorityContext, ControlError>;
}

/// Application port that publishes canonical control results as ordinary artifacts.
pub trait ControlResultSink: Send + Sync {
    /// Publishes exact canonical result bytes and returns their immutable reference.
    fn publish(
        &self,
        invocation: &AdapterInvocation<'_>,
        bytes: &[u8],
    ) -> Result<ArtifactReference, ControlError>;
}

/// Concrete in-process workflow-control adapter for `milkdrift-capability-host`.
pub struct WorkflowControlAdapter {
    service: Arc<ControlService>,
    contexts: Arc<dyn AuthorityContextResolver>,
    results: Arc<dyn ControlResultSink>,
}

impl WorkflowControlAdapter {
    /// Constructs an adapter with explicit authority-context and artifact-publication ports.
    #[must_use]
    pub fn new(
        service: Arc<ControlService>,
        contexts: Arc<dyn AuthorityContextResolver>,
        results: Arc<dyn ControlResultSink>,
    ) -> Self {
        Self {
            service,
            contexts,
            results,
        }
    }

    fn execute_control(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<ControlResult, ControlError> {
        let request_value = inline_input(invocation, CONTROL_REQUEST_INPUT)?;
        let context_value = inline_input(invocation, AUTHORITY_CONTEXT_INPUT)?;
        let context_ref = context_value.as_str().ok_or_else(|| {
            ControlError::InvalidContract(
                "authority context input must be a string reference".to_owned(),
            )
        })?;
        let context = self
            .contexts
            .resolve(&AuthorityContextRef::new(context_ref)?)?;
        let bytes = serde_json::to_vec(request_value)?;
        let document = ControlCommandDocument::from_json(&bytes)?;
        if document.context() != &context {
            return Err(ControlError::InvalidContract(
                "model-provided actor/grant context does not match the trusted reference"
                    .to_owned(),
            ));
        }
        ensure_operation_matches(invocation.request().operation(), document.command())?;
        self.service.execute(&document)
    }

    fn report_failure(
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
        error: &ControlError,
    ) -> Result<(), AdapterError> {
        let class = if matches!(error, ControlError::AuthorizationDenied { .. }) {
            ErrorClass::Authorization
        } else {
            ErrorClass::InvalidRequest
        };
        let failure = InvocationFailure::new(
            class,
            false,
            "workflow_control_rejected",
            bounded_message(&error.to_string()),
            None,
        )
        .map_err(|failure| AdapterError::rejected(failure.to_string()))?;
        let terminal = InvocationTerminal::new(
            TerminalStatus::Rejected,
            Vec::new(),
            Some(failure),
            None,
            SideEffectClass::None,
        )
        .map_err(|failure| AdapterError::rejected(failure.to_string()))?;
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                1,
                InvocationEventKind::Terminal { terminal },
            )
            .map_err(|failure| AdapterError::rejected(failure.to_string()))?,
        )
    }
}

impl CapabilityAdapter for WorkflowControlAdapter {
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let result = match self.execute_control(invocation) {
            Ok(result) => result,
            Err(error) => {
                return Self::report_failure(invocation, reporter, &error);
            }
        };
        let bytes = canonical_json_bytes(
            &result,
            JsonLimits {
                maximum_depth: 72,
                maximum_string_bytes: 65_536,
                maximum_key_bytes: 192,
                maximum_container_items: 4_096,
            },
        )
        .map_err(|error| AdapterError::rejected(format!("{error:?}")))?;
        let reference = self
            .results
            .publish(invocation, &bytes)
            .map_err(|error| AdapterError::rejected(error.to_string()))?;
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                1,
                InvocationEventKind::Output {
                    name: "control_result".to_owned(),
                    reference,
                },
            )
            .map_err(|error| AdapterError::rejected(error.to_string()))?,
        )?;
        let terminal = InvocationTerminal::new(
            TerminalStatus::Success,
            Vec::new(),
            None,
            None,
            SideEffectClass::IdempotentWrite,
        )
        .map_err(|error| AdapterError::rejected(error.to_string()))?;
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                2,
                InvocationEventKind::Terminal { terminal },
            )
            .map_err(|error| AdapterError::rejected(error.to_string()))?,
        )
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            false,
            false,
            Some(
                "workflow control commands are synchronous and idempotently replayable".to_owned(),
            ),
        )
        .map_err(|error| AdapterError::rejected(error.to_string()))
    }

    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        CapabilityObservation::new(
            CapabilityId::new(CONTROL_CAPABILITY_ID)
                .map_err(|error| AdapterError::rejected(error.to_string()))?,
            observed_at_unix_ms,
            true,
            0,
            "workflow control service is available",
        )
        .map_err(|error| AdapterError::rejected(error.to_string()))
    }
}

/// Builds the immutable descriptor for the in-process workflow-control adapter.
pub fn workflow_control_descriptor() -> Result<CapabilityDescriptor, ControlError> {
    let input_schema = SchemaContract::new(
        SchemaId::new("workflow.control_request")?,
        1,
        BoundedJson::new(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": [CONTROL_REQUEST_INPUT, AUTHORITY_CONTEXT_INPUT],
            "properties": {
                CONTROL_REQUEST_INPUT: { "type": "object" },
                AUTHORITY_CONTEXT_INPUT: { "type": "string", "maxLength": 192 }
            }
        }))?,
    )?;
    let output_schema = SchemaContract::new(
        SchemaId::new("workflow.control_result")?,
        1,
        BoundedJson::new(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["control_result"],
            "properties": {
                "control_result": { "type": "string", "description": "immutable artifact reference" }
            }
        }))?,
    )?;
    let operations = [
        (WORKFLOW_INSPECT_OPERATION, SideEffectClass::ReadOnly),
        (WORKFLOW_PROPOSE_OPERATION, SideEffectClass::IdempotentWrite),
        (WORKFLOW_PAUSE_OPERATION, SideEffectClass::IdempotentWrite),
        (WORKFLOW_RESUME_OPERATION, SideEffectClass::IdempotentWrite),
        (WORKFLOW_APPLY_OPERATION, SideEffectClass::IdempotentWrite),
        (WORKFLOW_RETRY_OPERATION, SideEffectClass::IdempotentWrite),
        (WORKFLOW_SIGNAL_OPERATION, SideEffectClass::IdempotentWrite),
    ]
    .into_iter()
    .map(|(name, side_effect)| {
        Ok((
            OperationId::new(name)?,
            OperationContract::new(
                input_schema.clone(),
                output_schema.clone(),
                BTreeSet::from([StreamingMode::OutputFragments]),
                CancellationBehavior::Unsupported,
                IdempotencyBehavior::CapabilityScoped,
                side_effect,
                BTreeMap::from([(
                    FeatureId::new("authority.scoped")?,
                    milkdrift_capability::FeatureContract::new(
                        FeatureId::new("authority.scoped")?,
                        None,
                    ),
                )]),
            )?,
        ))
    })
    .collect::<Result<BTreeMap<_, _>, ControlError>>()?;
    let authority_map = BoundedJson::new(serde_json::json!({
        WORKFLOW_INSPECT_OPERATION: "inspect",
        WORKFLOW_PROPOSE_OPERATION: "propose",
        WORKFLOW_PAUSE_OPERATION: "pause",
        WORKFLOW_RESUME_OPERATION: "resume",
        WORKFLOW_APPLY_OPERATION: "apply",
        WORKFLOW_RETRY_OPERATION: "retry",
        WORKFLOW_SIGNAL_OPERATION: "deliver_signal"
    }))?;
    Ok(DescriptorBuilder::new(
        CapabilityId::new(CONTROL_CAPABILITY_ID)?,
        1,
        CapabilityCategory::Custom(FeatureId::new("workflow.control")?),
        AdmissionConstraints::new(16, 64)?,
        Locality::Local,
    )
    .operations(operations)
    .trust_zones(BTreeSet::from([TrustZone::new("milkdrift-control")?]))
    .labels(BTreeSet::from([
        "audited".to_owned(),
        "authority-scoped".to_owned(),
        "workflow-control".to_owned(),
    ]))
    .extensions(BTreeMap::from([(
        ExtensionKey::new("org.milkdrift/authority-operations")?,
        authority_map,
    )]))
    .build()?)
}

fn inline_input<'a>(
    invocation: &'a AdapterInvocation<'_>,
    name: &str,
) -> Result<&'a serde_json::Value, ControlError> {
    invocation
        .request()
        .inputs()
        .iter()
        .find(|input| input.name() == name)
        .and_then(|input| match input.value() {
            InvocationValueReference::Inline { value } => Some(value.value()),
            _ => None,
        })
        .ok_or_else(|| {
            ControlError::InvalidContract(format!(
                "workflow-control invocation requires inline input {name}"
            ))
        })
}

fn ensure_operation_matches(
    operation: &OperationId,
    command: &ControlCommand,
) -> Result<(), ControlError> {
    let expected = match command {
        ControlCommand::InspectRun { .. }
        | ControlCommand::InspectRevision { .. }
        | ControlCommand::InspectTimeline { .. }
        | ControlCommand::QueryProposal { .. } => WORKFLOW_INSPECT_OPERATION,
        ControlCommand::SubmitProposal { .. } => WORKFLOW_PROPOSE_OPERATION,
        ControlCommand::PauseRun { .. } => WORKFLOW_PAUSE_OPERATION,
        ControlCommand::ResumeRun { .. } => WORKFLOW_RESUME_OPERATION,
        ControlCommand::ApproveProposal { .. }
        | ControlCommand::RejectProposal { .. }
        | ControlCommand::ApplyProposal { .. }
        | ControlCommand::CreateRun { .. }
        | ControlCommand::StartRun { .. }
        | ControlCommand::RequestCancellation { .. } => WORKFLOW_APPLY_OPERATION,
        ControlCommand::ResolveExternalWork { .. } => WORKFLOW_RETRY_OPERATION,
        ControlCommand::Signal { .. } => WORKFLOW_SIGNAL_OPERATION,
    };
    if operation.as_str() != expected {
        return Err(ControlError::InvalidContract(format!(
            "operation {} cannot execute this control command; expected {expected}",
            operation.as_str()
        )));
    }
    Ok(())
}

fn bounded_message(value: &str) -> String {
    if value.len() <= 4_096 {
        return value.to_owned();
    }
    let mut end = 4_096;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
