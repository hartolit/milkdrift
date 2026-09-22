use super::ManagedResources;
use crate::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter, InvocationDataAccess,
    MaterializationLimits,
};
use milkdrift_authority::CapabilityExecutionRequirements;
use milkdrift_capability::managed::ManagedRequest;
use milkdrift_capability::{
    AdmissionBound, AdmissionConstraints, AdmissionUnit, BoundedJson, CancellationAcknowledgement,
    CancellationBehavior, CancellationRequest, CapabilityCategory, CapabilityDescriptor,
    CapabilityId, CapabilityObservation, DescriptorBuilder, ExecutionTrustClass,
    IdempotencyBehavior, InvocationAdmissionEnvelope, InvocationEvent, InvocationEventKind,
    InvocationTerminal, InvocationValueReference, Locality, OperationContract, OperationId,
    SchemaContract, SchemaId, SideEffectClass, StreamingMode, TerminalStatus,
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
        1,
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
        1,
        CapabilityCategory::Tool,
        AdmissionConstraints::new(8, 0)?,
        Locality::Local,
    )
    .execution_trust(ExecutionTrustClass::Unspecified)
    .operations(BTreeMap::from([(
        OperationId::new("resource.manage")?,
        contract,
    )]))
    .build()
}

fn parse(invocation: &AdapterInvocation<'_>) -> Result<ManagedRequest, AdapterError> {
    if invocation.request().operation().as_str() != "resource.manage" {
        return Err(AdapterError::rejected("unsupported managed operation"));
    }
    let input = invocation
        .request()
        .inputs()
        .iter()
        .find(|i| i.name() == "request")
        .ok_or_else(|| AdapterError::rejected("managed request input is required"))?;
    let InvocationValueReference::Inline { value } = input.value() else {
        return Err(AdapterError::rejected(
            "managed request must be bounded inline JSON",
        ));
    };
    let request: ManagedRequest = serde_json::from_value(value.value().clone()).map_err(failure)?;
    request.validate().map_err(failure)?;
    Ok(request)
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
        parse(invocation)?;
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
        let request = parse(invocation)?;
        let context = invocation
            .context()
            .ok_or_else(|| AdapterError::rejected("durable execution context is required"))?;
        let caller = context
            .entry_authorization()
            .ok_or_else(|| AdapterError::rejected("final entry authority is required"))?
            .request();
        let response = self.owner.execute(caller, &request).map_err(failure)?;
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
