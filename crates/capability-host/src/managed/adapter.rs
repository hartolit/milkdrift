use super::{ManagedError, ManagedResources};
use crate::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter, InvocationDataAccess,
    MaterializationLimits,
};
use milkdrift_authority::CapabilityExecutionRequirements;
use milkdrift_capability::managed::{ManagedAction, ManagedRequest, ManagedTarget};
use milkdrift_capability::{
    AdmissionBound, AdmissionConstraints, AdmissionUnit, BoundedJson, CancellationAcknowledgement,
    CancellationBehavior, CancellationRequest, CapabilityCategory, CapabilityDescriptor,
    CapabilityId, CapabilityObservation, DescriptorBuilder, ExecutionTrustClass,
    IdempotencyBehavior, InvocationAdmissionEnvelope, InvocationEvent, InvocationEventKind,
    InvocationFailure, InvocationTerminal, InvocationValueReference, Locality, OperationContract,
    OperationId, SchemaContract, SchemaId, SideEffectClass, StreamingMode, TerminalStatus,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const CAPABILITY: &str = "milkdrift.resources";
const MAX_RESULT: u64 = 1_048_576;

/// In-process adapter routing direct and workflow calls to the same resource owner as administration.
pub struct ManagedLifecycleAdapter {
    owner: Arc<ManagedResources>,
    data: Arc<dyn InvocationDataAccess>,
}
impl ManagedLifecycleAdapter {
    /// Compose lifecycle semantics with the existing ordinary artifact publication owner.
    #[must_use]
    pub fn new(owner: Arc<ManagedResources>, data: Arc<dyn InvocationDataAccess>) -> Self {
        Self { owner, data }
    }
}

/// The resource-management operation itself grants no raw platform access. The semantic owner
/// separately checks the exact installation and requested operation beneath every caller.
pub fn managed_lifecycle_descriptor()
-> Result<CapabilityDescriptor, milkdrift_capability::ContractError> {
    let schema = SchemaContract::new(
        SchemaId::new("milkdrift.managed.command")?,
        2,
        BoundedJson::new(serde_json::json!({"type":"object"}))?,
    )?;
    let contract = OperationContract::new(
        schema.clone(),
        schema,
        BTreeSet::from([StreamingMode::None]),
        CancellationBehavior::Unsupported,
        IdempotencyBehavior::CapabilityScoped,
        SideEffectClass::IdempotentWrite,
        BTreeMap::new(),
    )?;
    DescriptorBuilder::new(
        CapabilityId::new(CAPABILITY)?,
        2,
        CapabilityCategory::Tool,
        AdmissionConstraints::new(8, 0)?,
        Locality::Local,
    )
    .execution_trust(ExecutionTrustClass::Unspecified)
    .operations(BTreeMap::from([
        (OperationId::new("resource.manage")?, contract.clone()),
        (
            OperationId::new("resource.evaluate_candidate")?,
            contract.clone(),
        ),
        (OperationId::new("resource.publish_candidate")?, contract),
    ]))
    .build()
}

enum Parsed {
    Ready(ManagedRequest),
    Evaluation(ManagedTarget, milkdrift_capability::InputReference),
    Publication(ManagedTarget, milkdrift_capability::InputReference),
}
fn parse(
    invocation: &AdapterInvocation<'_>,
    data: &dyn InvocationDataAccess,
) -> Result<Parsed, AdapterError> {
    let operation = invocation.request().operation().as_str();
    let input = |name: &str| {
        invocation
            .request()
            .inputs()
            .iter()
            .find(|i| i.name() == name)
            .ok_or_else(|| AdapterError::rejected(format!("managed {name} input is required")))
    };
    let name = if operation == "resource.manage" {
        "request"
    } else {
        "target"
    };
    let value = match input(name)?.value() {
        InvocationValueReference::Inline { value } => value.value().clone(),
        _ => {
            let context = invocation.context().ok_or_else(|| {
                AdapterError::rejected(
                    "referenced managed target/request requires authorized execution context",
                )
            })?;
            let bytes = data
                .read_input_bytes(
                    context,
                    input(name)?,
                    MaterializationLimits {
                        max_files: 1,
                        max_file_bytes: 65_536,
                        max_total_bytes: 65_536,
                        max_path_bytes: 256,
                        max_directory_depth: 8,
                        chunk_bytes: 16_384,
                    },
                )
                .map_err(failure)?;
            milkdrift_contracts::parse_json_without_duplicates(&bytes).map_err(failure)?
        }
    };
    if operation == "resource.manage" {
        let request: ManagedRequest = serde_json::from_value(value).map_err(failure)?;
        request.validate().map_err(failure)?;
        return Ok(Parsed::Ready(request));
    }
    let target: ManagedTarget = serde_json::from_value(value).map_err(failure)?;
    target.request(ManagedAction::Inspect {}).map_err(failure)?;
    match operation {
        "resource.evaluate_candidate" | "resource.publish_candidate" => {
            let selected = input(if operation == "resource.evaluate_candidate" {
                "candidate"
            } else {
                "evaluation"
            })?;
            if matches!(selected.value(), InvocationValueReference::Inline { .. }) {
                return Err(AdapterError::rejected(
                    "protected operation needs a selected immutable artifact",
                ));
            }
            Ok(if operation == "resource.evaluate_candidate" {
                Parsed::Evaluation(target, selected.clone())
            } else {
                Parsed::Publication(target, selected.clone())
            })
        }
        _ => Err(AdapterError::rejected("unsupported managed operation")),
    }
}

fn failure(e: impl std::fmt::Display) -> AdapterError {
    AdapterError::rejected(e.to_string())
}
impl CapabilityAdapter for ManagedLifecycleAdapter {
    fn accepts_direct_inputs(&self) -> bool {
        true
    }
    fn admission_envelope(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        parse(invocation, self.data.as_ref())?;
        Ok(InvocationAdmissionEnvelope::new(
            AdmissionUnit::Unknown,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::Bounded(MAX_RESULT),
            AdmissionBound::NotApplicable,
        ))
    }
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        CapabilityExecutionRequirements::default()
    }
    fn start(&self) -> Result<(), AdapterError> {
        Ok(())
    }
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let parsed = parse(invocation, self.data.as_ref())?;
        let context = invocation
            .context()
            .ok_or_else(|| AdapterError::rejected("durable execution context is required"))?;
        let request = match parsed {
            Parsed::Ready(request) => request,
            Parsed::Evaluation(target, selected) => {
                let candidate = self
                    .data
                    .resolve_artifact_reference(context, &selected)
                    .map_err(failure)?;
                target
                    .request(ManagedAction::Evaluate { candidate })
                    .map_err(failure)?
            }
            Parsed::Publication(target, reference) => {
                let bytes = self
                    .data
                    .read_input_bytes(
                        context,
                        &reference,
                        MaterializationLimits {
                            max_files: 1,
                            max_file_bytes: 32_768,
                            max_total_bytes: 32_768,
                            max_path_bytes: 256,
                            max_directory_depth: 8,
                            chunk_bytes: 32_768,
                        },
                    )
                    .map_err(failure)?;
                let value =
                    milkdrift_contracts::parse_json_without_duplicates(&bytes).map_err(failure)?;
                let response: milkdrift_capability::managed::ManagedResponse =
                    serde_json::from_value(value).map_err(failure)?;
                let evidence: milkdrift_workspace::CandidateEvaluation = serde_json::from_value(
                    response
                        .evaluation
                        .ok_or_else(|| AdapterError::rejected("evaluation result has no identity"))?
                        .value()
                        .clone(),
                )
                .map_err(failure)?;
                evidence.validate().map_err(failure)?;
                // Only the identity is selected from this untrusted input. The resource owner reads
                // the actual observation from its private journal and rechecks all applicability.
                target
                    .request(ManagedAction::Publish {
                        evaluation: evidence.identity,
                    })
                    .map_err(failure)?
            }
        };
        let caller = context
            .entry_authorization()
            .ok_or_else(|| AdapterError::rejected("final entry authority is required"))?
            .request();
        let response = match self.owner.execute(caller, &request) {
            Ok(response) => response,
            Err(error) => {
                // A known refusal with no accepted intent is terminal. Once an intent exists,
                // even a later registry conflict may follow a completed effect: retain uncertainty.
                if matches!(
                    error,
                    ManagedError::Rejected(_)
                        | ManagedError::Unauthorized
                        | ManagedError::Conflict(_)
                ) && self
                    .owner
                    .store
                    .managed_receipt(caller.actor.as_str(), &request.command)
                    .map_err(failure)?
                    .is_none()
                {
                    let class = if matches!(error, ManagedError::Unauthorized) {
                        milkdrift_capability::ErrorClass::Authorization
                    } else {
                        milkdrift_capability::ErrorClass::InvalidRequest
                    };
                    return reporter.invocation(
                        InvocationEvent::new(
                            invocation.request().invocation().clone(),
                            1,
                            InvocationEventKind::Terminal {
                                terminal: InvocationTerminal::new(
                                    TerminalStatus::Rejected,
                                    Vec::new(),
                                    Some(
                                        InvocationFailure::new(
                                            class,
                                            false,
                                            "managed_operation_rejected",
                                            super::bounded(&error.to_string()),
                                            None,
                                        )
                                        .map_err(failure)?,
                                    ),
                                    None,
                                    SideEffectClass::None,
                                )
                                .map_err(failure)?,
                            },
                        )
                        .map_err(failure)?,
                    );
                }
                return Err(failure(error));
            }
        };
        let bytes = serde_json::to_vec(&response).map_err(failure)?;
        let artifact = self
            .data
            .publish_bytes(
                context,
                invocation.request(),
                "resource_result",
                "application/vnd.milkdrift.managed+json",
                &bytes,
                MaterializationLimits {
                    max_files: 1,
                    max_file_bytes: MAX_RESULT,
                    max_total_bytes: MAX_RESULT,
                    max_path_bytes: 256,
                    max_directory_depth: 8,
                    chunk_bytes: 262_144,
                },
            )
            .map_err(failure)?;
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                1,
                InvocationEventKind::Output {
                    name: "resource_result".to_owned(),
                    reference: artifact,
                },
            )
            .map_err(failure)?,
        )?;
        if let ManagedAction::Publish { evaluation } = &request.action {
            let record = self
                .owner
                .store
                .managed_installation(&request.installation)
                .map_err(failure)?;
            if record.as_ref().is_none_or(|r| {
                r.pending.is_some()
                    || r.removed
                    || !r.desired_running
                    || r.current
                        .as_ref()
                        .and_then(|s| s.protection.as_ref())
                        .and_then(|p| p.evidence.as_ref())
                        .is_none_or(|e| &e.identity != evaluation)
            }) {
                return Err(AdapterError::external_failure(
                    "protected publication is accepted but has no verified completion; inspect its exact transition",
                ));
            }
        }
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                2,
                InvocationEventKind::Terminal {
                    terminal: InvocationTerminal::new(
                        TerminalStatus::Success,
                        Vec::new(),
                        None,
                        None,
                        SideEffectClass::IdempotentWrite,
                    )
                    .map_err(failure)?,
                },
            )
            .map_err(failure)?,
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
                "accepted resource intent survives its invoking client; inspect or recover it"
                    .to_owned(),
            ),
        )
        .map_err(failure)
    }
    fn health(&self, now: u64) -> Result<CapabilityObservation, AdapterError> {
        CapabilityObservation::new(
            CapabilityId::new(CAPABILITY).map_err(failure)?,
            now,
            true,
            0,
            "managed resource owner available",
        )
        .map_err(failure)
    }
    fn begin_drain(&self) -> Result<(), AdapterError> {
        Ok(())
    }
    fn shutdown(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}
