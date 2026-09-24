//! Publish a governed method through the existing capability host and runtime command path.
//! The service retains no invocation ledger or execution thread. Its continuation reconstructs
//! the exact accepted child from the caller-owned association and observes that run's journal.
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};
mod authority;
mod result;
mod validation;
use milkdrift_authority::{
    AuthorityDecisionSnapshot, AuthorityEvaluator, CapabilityExecutionRequirements,
};
use milkdrift_capability::{
    CancellationAcknowledgement, CancellationRequest, CapabilityObservation, ErrorClass,
    InvocationAdmissionEnvelope, InvocationEvent, InvocationEventKind, InvocationFailure,
    InvocationTerminal, SideEffectClass, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter, CapabilityHost,
    InvocationDataAccess, MaterializationLimits, PreparedAdapterExecution,
    PublishedWorkflowContinuation,
};
use milkdrift_persistence::{
    CommandId, PageSize, PeerExecutionStore, Reason, RunOutcome, RunSequence, TimestampMillis,
    published::{
        PublishedInput, PublishedInvocationPlan, PublishedMethod, PublishedMethodRecord,
        PublishedMethodStore, PublishedServiceIdentity,
    },
};
use milkdrift_runtime::{
    BoundaryClock, ExecutorError, RunCommand, RunCommandDocument, RuntimeService, RuntimeStore,
};
use milkdrift_workspace::{
    RunId, ScopeId, ValueKey, WorkspaceScope, WorkspaceValue, WorkspaceValueEntry,
};

/// Publication and run storage must address the same durable host. Definitions, receipts and
/// public operations remain in their existing owners; this trait is only their composition port.
pub trait PublishedWorkflowStore:
    RuntimeStore
    + PublishedMethodStore
    + PeerExecutionStore
    + milkdrift_persistence::managed::ManagedResourceStore
{
}
impl<
    T: RuntimeStore
        + PublishedMethodStore
        + PeerExecutionStore
        + milkdrift_persistence::managed::ManagedResourceStore,
> PublishedWorkflowStore for T
{
}

/// Workflow-enabled composition's publication owner. Configured service identities are an explicit
/// allowlist for each public capability; knowing another grant's identity never authorizes its use.
pub struct PublishedWorkflowService {
    host: CapabilityHost,
    store: Arc<dyn PublishedWorkflowStore>,
    runtime: Arc<RuntimeService>,
    authority: Arc<dyn AuthorityEvaluator>,
    clock: Arc<dyn BoundaryClock>,
    data: Arc<dyn InvocationDataAccess>,
    services: BTreeMap<milkdrift_capability::CapabilityId, PublishedServiceIdentity>,
}

impl PublishedWorkflowService {
    /// Compose publication with ordinary owners and operator-approved service relationships.
    pub fn new(
        host: CapabilityHost,
        store: Arc<dyn PublishedWorkflowStore>,
        runtime: Arc<RuntimeService>,
        authority: Arc<dyn AuthorityEvaluator>,
        clock: Arc<dyn BoundaryClock>,
        data: Arc<dyn InvocationDataAccess>,
        services: BTreeMap<milkdrift_capability::CapabilityId, PublishedServiceIdentity>,
    ) -> Arc<Self> {
        Arc::new(Self {
            host,
            store,
            runtime,
            authority,
            clock,
            data,
            services,
        })
    }

    /// Validate and publish an immutable method, then advertise its invocable implementation.
    pub fn publish(
        self: &Arc<Self>,
        method: PublishedMethod,
        expected_previous_version: Option<u64>,
        authorization: &AuthorityDecisionSnapshot,
        request: &milkdrift_persistence::IntegrityDigest,
    ) -> Result<PublishedMethodRecord, crate::ControlError> {
        let host = &self.host;
        self.validate_method(&method, host)
            .map_err(publication_error)?;
        let current = self
            .current_decision(authorization.request().clone())
            .map_err(publication_error)?;
        let (_, record) = host.register_with_commit(
            method.capability_descriptor()?,
            Arc::new(PublishedMethodAdapter {
                owner: Arc::downgrade(self),
                method: method.clone(),
            }),
            Some(
                CapabilityObservation::new(
                    method.descriptor.identity().clone(),
                    self.clock
                        .now()
                        .map_err(failure)
                        .map_err(publication_error)?
                        .get(),
                    true,
                    0,
                    "published workflow owner available",
                )
                .map_err(failure)
                .map_err(publication_error)?,
            ),
            || {
                self.store
                    .publish_method(&method, expected_previous_version, &current, request)
                    .map_err(crate::ControlError::from)
            },
        )?;
        let current_record = self
            .store
            .published_method(
                method.descriptor.identity(),
                method.descriptor.descriptor_revision(),
            )?
            .ok_or_else(|| {
                crate::ControlError::InvalidContract(
                    "publication disappeared after commit".to_owned(),
                )
            })?;
        self.register(host, &current_record)
            .map_err(publication_error)?;
        Ok(record)
    }

    /// Close new selection while retaining exact accepted invocations and immutable definitions.
    pub fn retire(
        &self,
        capability: &milkdrift_capability::CapabilityId,
        generation: u64,
        expected_version: u64,
        authorization: &AuthorityDecisionSnapshot,
        request: &milkdrift_persistence::IntegrityDigest,
    ) -> Result<PublishedMethodRecord, crate::ControlError> {
        let host = &self.host;
        let current = self
            .current_decision(authorization.request().clone())
            .map_err(publication_error)?;
        let record = self.store.retire_method(
            capability,
            generation,
            expected_version,
            &current,
            request,
        )?;
        host.begin_drain(capability, generation)
            .map_err(|error| crate::ControlError::InvalidContract(error.to_string()))?;
        Ok(record)
    }

    /// Reconstruct every retained exact generation before recovery admits new calls.
    pub fn restore(self: &Arc<Self>) -> Result<(), ExecutorError> {
        let host = &self.host;
        let mut after = None;
        loop {
            let page = self
                .store
                .published_methods(
                    after.as_ref().map(|(id, generation)| (id, *generation)),
                    PageSize::new(128).map_err(failure)?,
                )
                .map_err(failure)?;
            if page.is_empty() {
                break;
            }
            for record in &page {
                self.register(host, record)?;
            }
            after = page.last().map(|record| {
                (
                    record.method.descriptor.identity().clone(),
                    record.method.descriptor.descriptor_revision(),
                )
            });
        }
        let now = self.clock.now().map_err(failure)?.get();
        let mut after = None;
        loop {
            let page = self
                .store
                .published_methods(
                    after.as_ref().map(|(id, generation)| (id, *generation)),
                    PageSize::new(128).map_err(failure)?,
                )
                .map_err(failure)?;
            if page.is_empty() {
                break;
            }
            for record in &page {
                host.refresh_health(
                    record.method.descriptor.identity(),
                    record.method.descriptor.descriptor_revision(),
                    now,
                )
                .map_err(failure)?;
            }
            after = page.last().map(|record| {
                (
                    record.method.descriptor.identity().clone(),
                    record.method.descriptor.descriptor_revision(),
                )
            });
        }
        let mut cursor = None;
        loop {
            let (plans, next) = self
                .store
                .published_local_page(cursor.as_ref(), PageSize::new(128).map_err(failure)?)
                .map_err(failure)?;
            for plan in &plans {
                host.retain_published_invocation(plan)?;
            }
            cursor = next;
            if cursor.is_none() {
                break;
            }
        }
        let mut cursor = None;
        loop {
            let (records, next) = self
                .store
                .published_serving_page(cursor.as_ref(), PageSize::new(128).map_err(failure)?)
                .map_err(failure)?;
            for record in records {
                if let Some(plan) = &record.published_invocation {
                    host.retain_published_invocation(plan)?;
                }
            }
            cursor = next;
            if cursor.is_none() {
                break;
            }
        }
        Ok(())
    }

    fn register(
        self: &Arc<Self>,
        host: &CapabilityHost,
        record: &PublishedMethodRecord,
    ) -> Result<(), ExecutorError> {
        let method = &record.method;
        let now = self.clock.now().map_err(failure)?.get();
        // Retained generations must remain recoverable even when current configuration cannot
        // execute them. Exact discovery reports unavailable; accepted cleanup keeps its owner.
        let available = self.validate_method(method, host).is_ok();
        host.register(
            method.capability_descriptor().map_err(failure)?,
            Arc::new(PublishedMethodAdapter {
                owner: Arc::downgrade(self),
                method: method.clone(),
            }),
            Some(
                CapabilityObservation::new(
                    method.descriptor.identity().clone(),
                    now,
                    available,
                    0,
                    if available {
                        "published workflow owner available"
                    } else {
                        "published implementation or service authority unavailable"
                    },
                )
                .map_err(failure)?,
            ),
        )
        .map_err(failure)?;
        if record.retired {
            host.begin_drain(
                method.descriptor.identity(),
                method.descriptor.descriptor_revision(),
            )
            .map_err(failure)?;
        }
        Ok(())
    }

    fn validate_method(
        &self,
        method: &PublishedMethod,
        host: &CapabilityHost,
    ) -> Result<(), ExecutorError> {
        method.validate().map_err(failure)?;
        if self.services.get(method.descriptor.identity()) != Some(&method.service) {
            return Err(rejected(
                "publication does not match an operator-approved service relationship",
            ));
        }
        if method.descriptor.operations().len() != 1
            || method
                .descriptor
                .operations()
                .keys()
                .next()
                .is_none_or(|op| op.as_str() != "method.invoke")
        {
            return Err(rejected("published methods expose exactly method.invoke"));
        }
        let revision = self
            .store
            .revision(&method.revision)
            .map_err(failure)?
            .ok_or_else(|| rejected("starting method is absent"))?;
        let agreement = revision
            .semantic()
            .agreement()
            .ok_or_else(|| rejected("published method requires a governing agreement"))?;
        if agreement.digest() != method.agreement
            || revision.semantic().interface().inputs().len() != method.inputs.len()
            || revision
                .semantic()
                .interface()
                .inputs()
                .keys()
                .any(|key| !method.inputs.contains_key(key.as_str()))
        {
            return Err(rejected(
                "publication differs from its agreement or declared workflow inputs",
            ));
        }
        self.validate_implementations(method, host)?;
        for output in method.outputs.values() {
            if !revision
                .semantic()
                .interface()
                .outputs()
                .keys()
                .any(|field| field.as_str() == output.field.as_str())
            {
                return Err(rejected(
                    "public result must name a declared terminal workflow output",
                ));
            }
        }
        Ok(())
    }

    fn prepare(
        &self,
        method: &PublishedMethod,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<PublishedInvocationPlan, ExecutorError> {
        let context = invocation
            .context()
            .ok_or_else(|| rejected("published method requires durable caller provenance"))?;
        let caller = context
            .entry_authorization()
            .ok_or_else(|| rejected("published method requires an accepted caller decision"))?
            .clone();
        let source = context
            .published_source()
            .ok_or_else(|| rejected("published caller owner is absent"))?;
        let now = self.clock.now().map_err(failure)?.get();
        if invocation.request().inputs().len() != method.inputs.len()
            || !invocation.request().extensions().is_empty()
        {
            return Err(rejected(
                "public input set differs from the published contract",
            ));
        }
        let mut ancestry = context.publication_ancestry().to_vec();
        if let milkdrift_persistence::published::PublishedInvocationSource::Local { run, .. } =
            &source
        {
            let parent = self.runtime.projection(run).map_err(failure)?;
            if let Some(parent_source) = parent.published_source() {
                let prior = self
                    .store
                    .published_invocation(parent_source)
                    .map_err(failure)?
                    .ok_or_else(|| rejected("parent publication association is absent"))?;
                let ancestor = prior.ancestor().map_err(failure)?;
                ancestry = prior.ancestry;
                ancestry.push(ancestor);
            }
        }
        if ancestry.len() >= usize::from(method.maximum_depth)
            || ancestry.iter().any(|ancestor| {
                ancestor.capability() == method.descriptor.identity()
                    || ancestry.len() >= usize::from(ancestor.maximum_depth())
            })
        {
            return Err(rejected(
                "publication cycle or nesting limit refused before child creation",
            ));
        }
        let identity = serde_json::to_vec(&(
            method.descriptor.identity(),
            method.descriptor.descriptor_revision(),
            &source,
            invocation.request().invocation(),
        ))
        .map_err(failure)?;
        let hash = blake3::hash(&identity).to_hex();
        let child = RunId::new(format!("published:{hash}")).map_err(failure)?;
        let root = WorkspaceScope::run_root(child.clone(), ScopeId::new("root").map_err(failure)?);
        let mut inputs = Vec::new();
        for input in invocation.request().inputs() {
            let rule = method
                .inputs
                .get(input.name())
                .ok_or_else(|| rejected("undeclared public input"))?;
            if matches!(
                source,
                milkdrift_persistence::published::PublishedInvocationSource::Local { .. }
            ) {
                self.authorize_local_input(&caller, input)?;
            }
            let value = match rule {
                PublishedInput::Choice { values } => {
                    let bytes = self
                        .data
                        .read_input_bytes(context, input, limits(65_536))
                        .map_err(failure)?;
                    let value = milkdrift_capability::BoundedJson::new(
                        serde_json::from_slice(&bytes).map_err(failure)?,
                    )
                    .map_err(failure)?;
                    if !values.contains(&value) {
                        return Err(rejected(
                            "public input value is outside the approved choices",
                        ));
                    }
                    WorkspaceValue::Json(value)
                }
                PublishedInput::Artifact {
                    media_type,
                    maximum_bytes,
                } => {
                    let reference = self
                        .data
                        .resolve_artifact_reference(context, input)
                        .map_err(failure)?;
                    if reference.media_type() != Some(media_type.as_str())
                        || reference
                            .size_bytes()
                            .is_none_or(|size| size > *maximum_bytes)
                    {
                        return Err(rejected(
                            "public artifact exceeds its type or size contract",
                        ));
                    }
                    self.data
                        .read_input_bytes(context, input, limits(*maximum_bytes))
                        .map_err(failure)?;
                    WorkspaceValue::Artifact(milkdrift_workspace::ArtifactReference::new(
                        milkdrift_workspace::ArtifactId::new(reference.identity())
                            .map_err(failure)?,
                        milkdrift_workspace::ContentDigest::from_hex(reference.digest())
                            .map_err(failure)?,
                        milkdrift_workspace::MediaType::new(media_type).map_err(failure)?,
                        reference
                            .size_bytes()
                            .ok_or_else(|| rejected("artifact size absent"))?,
                    ))
                }
            };
            inputs.push(WorkspaceValueEntry::initial(
                root.reference().clone(),
                ValueKey::new(input.name()).map_err(failure)?,
                value,
            ));
        }
        let revision = self
            .store
            .revision(&method.revision)
            .map_err(failure)?
            .ok_or_else(|| rejected("starting method is unavailable"))?;
        let command = |suffix: &str, sequence: RunSequence, action: RunCommand| {
            RunCommandDocument::new(
                CommandId::new(format!("published:{hash}:{suffix}")).map_err(failure)?,
                child.clone(),
                method.service.actor.clone(),
                sequence,
                TimestampMillis::new(now),
                Reason::new("accepted published workflow invocation").map_err(failure)?,
                Vec::new(),
                action,
            )
            .map_err(failure)
        };
        let create = command(
            "create",
            RunSequence::ZERO,
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: method.revision.clone(),
                root_scope: root,
                workspace_budget: method.workspace_budget.clone(),
                inputs,
            },
        )?;
        let start = command("start", RunSequence::new(2), RunCommand::StartRun)?;
        let plan = PublishedInvocationPlan {
            schema_version: 1,
            source,
            invocation: invocation.request().invocation().clone(),
            request: invocation.request().clone(),
            capability: method.descriptor.identity().clone(),
            generation: method.descriptor.descriptor_revision(),
            method_digest: method.digest().map_err(failure)?,
            allowance:
                milkdrift_persistence::ControllerAccountDeclaration::for_published_invocation(
                    child.clone(),
                    invocation.request().invocation().clone(),
                    method.digest().map_err(failure)?,
                    method.allowance.clone(),
                )
                .map_err(failure)?,
            child_run: child,
            create_command: String::from_utf8(create.to_canonical_json().map_err(failure)?)
                .map_err(failure)?,
            start_command: String::from_utf8(start.to_canonical_json().map_err(failure)?)
                .map_err(failure)?,
            cancel_command: CommandId::new(format!("published:{hash}:cancel")).map_err(failure)?,
            service: method.service.clone(),
            caller,
            maximum_depth: method.maximum_depth,
            ancestry,
            deadline_unix_ms: now
                .checked_add(method.maximum_duration_ms)
                .ok_or_else(|| rejected("published deadline overflow"))?,
        };
        plan.validate().map_err(failure)?;
        self.authorize_future_start(&plan)?;
        Ok(plan)
    }
}

impl PublishedWorkflowContinuation for PublishedWorkflowService {
    fn continue_invocation(
        &self,
        plan: &PublishedInvocationPlan,
        cancel: bool,
        next_sequence: u64,
    ) -> Result<Option<InvocationEvent>, ExecutorError> {
        let method = self
            .store
            .published_method(&plan.capability, plan.generation)
            .map_err(failure)?
            .ok_or_else(|| rejected("accepted published implementation is unavailable"))?
            .method;
        if method.digest().map_err(failure)? != plan.method_digest || method.service != plan.service
        {
            return Err(rejected("accepted publication identity changed"));
        }
        if self
            .store
            .published_invocation(&plan.source)
            .map_err(failure)?
            .as_ref()
            != Some(plan)
        {
            return Err(rejected(
                "continuation differs from its authoritative acceptance",
            ));
        }
        let cancelled = cancel
            || self.services.get(&plan.capability) != Some(&plan.service)
            || self.clock.now().map_err(failure)?.get() >= plan.deadline_unix_ms;
        let existing = self.runtime.projection(&plan.child_run).map_err(failure)?;
        if cancelled
            || existing.lifecycle()
                == milkdrift_runtime::RunLifecycle::Terminal(RunOutcome::Cancelled)
        {
            if existing.lifecycle() == milkdrift_runtime::RunLifecycle::Uncreated {
                return terminal_event(
                    plan,
                    next_sequence,
                    TerminalStatus::Cancelled,
                    SideEffectClass::None,
                    Vec::new(),
                    "cancelled before internal creation",
                    zero_usage(&method.allowance)?,
                );
            }
            self.runtime
                .cancel_published_run(plan, self.store.as_ref())
                .map_err(failure)?;
        } else {
            match self.authorize_future_start(plan) {
                Ok(()) => self
                    .runtime
                    .arrange_published_run(plan, self.store.as_ref())
                    .map_err(failure)?,
                Err(ExecutorError::BoundaryBeforeEntry(_))
                    if existing.lifecycle() == milkdrift_runtime::RunLifecycle::Uncreated =>
                {
                    return terminal_event(
                        plan,
                        next_sequence,
                        TerminalStatus::Rejected,
                        SideEffectClass::None,
                        Vec::new(),
                        "service authority refused before internal creation",
                        zero_usage(&method.allowance)?,
                    );
                }
                Err(ExecutorError::BoundaryBeforeEntry(_)) => {
                    self.runtime
                        .cancel_published_run(plan, self.store.as_ref())
                        .map_err(failure)?;
                }
                Err(error) => return Err(error),
            }
        }
        let projection = self.runtime.projection(&plan.child_run).map_err(failure)?;
        let Some(terminal) = projection.terminal() else {
            return Ok(None);
        };
        if let Some(usage) = self
            .store
            .managed_use(&milkdrift_persistence::managed::managed_use_id(
                &plan.invocation,
            ))
            .map_err(failure)?
            && matches!(
                usage.phase,
                milkdrift_persistence::managed::ManagedUsePhase::Suspended { .. }
            )
        {
            // A workflow terminal is not a stop proof. Keep the public operation and lifetime
            // hold owned until the resource owner has settled the exact child writer.
            return Ok(None);
        }
        let status = match terminal.outcome() {
            RunOutcome::Succeeded
                if !cancelled
                    && projection
                        .accepted_agreement()
                        .is_some_and(|binding| binding.agreement_digest() == method.agreement) =>
            {
                TerminalStatus::Success
            }
            RunOutcome::Cancelled => TerminalStatus::Cancelled,
            _ if cancelled => TerminalStatus::Cancelled,
            _ => TerminalStatus::Failure,
        };
        let outputs = if status == TerminalStatus::Success {
            let (event, outputs) =
                self.public_outputs(&method, plan, &projection, next_sequence)?;
            if let Some(event) = event {
                return Ok(Some(event));
            }
            outputs
        } else {
            Vec::new()
        };
        terminal_event(
            plan,
            next_sequence,
            status,
            method
                .descriptor
                .operation(plan.request.operation())
                .ok_or_else(|| rejected("published operation disappeared"))?
                .side_effect(),
            outputs,
            "internal method did not satisfy its accepted agreement",
            self.settled_usage(plan)?,
        )
    }
}

struct PublishedMethodAdapter {
    owner: Weak<PublishedWorkflowService>,
    method: PublishedMethod,
}
impl CapabilityAdapter for PublishedMethodAdapter {
    fn maximum_pending_workflows(&self) -> Option<u32> {
        Some(self.method.maximum_outstanding)
    }
    fn accepts_direct_inputs(&self) -> bool {
        true
    }
    fn prepare(
        self: Arc<Self>,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<PreparedAdapterExecution, AdapterError> {
        let owner = self
            .owner
            .upgrade()
            .ok_or_else(|| AdapterError::unavailable("publication owner unavailable"))?;
        owner
            .validate_method(&self.method, &owner.host)
            .map_err(|error| AdapterError::unavailable(error.to_string()))?;
        let plan = owner
            .prepare(&self.method, invocation)
            .map_err(|e| AdapterError::rejected(e.to_string()))?;
        Ok(PreparedAdapterExecution::published(
            self.admission_envelope(invocation)?,
            plan,
        ))
    }
    fn admission_envelope(
        &self,
        _: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        self.method
            .admission_envelope()
            .map_err(|error| AdapterError::rejected(error.to_string()))
    }
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        CapabilityExecutionRequirements {
            budget: milkdrift_authority::AuthorityBudget {
                artifact_bytes: self
                    .method
                    .admission_envelope()
                    .ok()
                    .and_then(|envelope| envelope.artifact_bytes().bounded().copied()),
                units: Some(
                    self.method
                        .allowance
                        .input_units()
                        .saturating_add(self.method.allowance.output_units()),
                ),
                duration_ms: Some(self.method.maximum_duration_ms),
                invocations: Some(1),
                concurrency: Some(1),
                cost_minor: self
                    .method
                    .allowance
                    .currency()
                    .as_ref()
                    .map(|_| self.method.allowance.cost_micros().saturating_add(9999) / 10000),
            },
            ..CapabilityExecutionRequirements::default()
        }
    }
    fn start(&self) -> Result<(), AdapterError> {
        Ok(())
    }
    fn execute(
        &self,
        _: &AdapterInvocation<'_>,
        _: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        Err(AdapterError::rejected(
            "published work requires its durable continuation owner",
        ))
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
            Some("cancellation belongs to the durable invocation owner".to_owned()),
        )
        .map_err(|e| AdapterError::rejected(e.to_string()))
    }
    fn health(&self, now: u64) -> Result<CapabilityObservation, AdapterError> {
        CapabilityObservation::new(
            self.method.descriptor.identity().clone(),
            now,
            self.owner
                .upgrade()
                .is_some_and(|owner| owner.validate_method(&self.method, &owner.host).is_ok()),
            0,
            "published workflow owner",
        )
        .map_err(|e| AdapterError::rejected(e.to_string()))
    }
    fn begin_drain(&self) -> Result<(), AdapterError> {
        Ok(())
    }
    fn shutdown(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}
fn limits(bytes: u64) -> MaterializationLimits {
    MaterializationLimits {
        max_files: 128,
        max_file_bytes: bytes,
        max_total_bytes: bytes,
        max_path_bytes: 4096,
        max_directory_depth: 32,
        chunk_bytes: 65_536,
    }
}
fn rejected(message: &str) -> ExecutorError {
    ExecutorError::BoundaryBeforeEntry(message.to_owned())
}
fn failure(error: impl std::fmt::Display) -> ExecutorError {
    ExecutorError::Boundary(error.to_string())
}

fn terminal_event(
    plan: &PublishedInvocationPlan,
    next_sequence: u64,
    status: TerminalStatus,
    side_effect: SideEffectClass,
    outputs: Vec<milkdrift_capability::ArtifactReference>,
    detail: &str,
    usage: Option<milkdrift_capability::UsageObservation>,
) -> Result<Option<InvocationEvent>, ExecutorError> {
    let invocation_failure = matches!(status, TerminalStatus::Failure | TerminalStatus::Rejected)
        .then(|| {
            InvocationFailure::new(
                ErrorClass::Adapter,
                false,
                "published_method_failed",
                detail,
                None,
            )
        })
        .transpose()
        .map_err(failure)?;
    let terminal = InvocationTerminal::new(status, outputs, invocation_failure, usage, side_effect)
        .map_err(failure)?;
    Ok(Some(
        InvocationEvent::new(
            plan.invocation.clone(),
            next_sequence,
            InvocationEventKind::Terminal { terminal },
        )
        .map_err(failure)?,
    ))
}

fn zero_usage(
    budget: &milkdrift_persistence::ControllerResourceBudget,
) -> Result<Option<milkdrift_capability::UsageObservation>, ExecutorError> {
    usage_observation(
        milkdrift_persistence::ControllerResourceTotals::default(),
        budget,
    )
}
fn usage_observation(
    totals: milkdrift_persistence::ControllerResourceTotals,
    budget: &milkdrift_persistence::ControllerResourceBudget,
) -> Result<Option<milkdrift_capability::UsageObservation>, ExecutorError> {
    Ok(Some(
        milkdrift_capability::UsageObservation::new(
            Some(totals.input_units()),
            Some(totals.output_units()),
            None,
            budget.currency().as_ref().map(|_| totals.cost_micros()),
            budget
                .currency()
                .as_ref()
                .map(|currency| currency.as_str().to_owned()),
            BTreeMap::new(),
        )
        .map_err(failure)?
        .with_nested_work(milkdrift_capability::NestedWorkUsage::new(
            milkdrift_capability::InvocationCounts::new(
                totals.process_admissions(),
                totals.model_admissions(),
            ),
            totals.artifact_bytes(),
        )),
    ))
}
impl PublishedWorkflowService {
    fn settled_usage(
        &self,
        plan: &PublishedInvocationPlan,
    ) -> Result<Option<milkdrift_capability::UsageObservation>, ExecutorError> {
        let account = self
            .store
            .controller_account(plan.allowance.account())
            .map_err(failure)?
            .ok_or_else(|| rejected("internal terminal run lost its allowance"))?;
        if account.declaration() != &plan.allowance {
            return Err(rejected("internal allowance identity changed"));
        }
        // Absence is deliberate: unresolved usage must retain the outer reservation. A terminal
        // workflow outcome is not evidence that all external effects were measured or stopped.
        if account.blocked().is_some() || !account.reservations().is_empty() {
            return Ok(None);
        }
        usage_observation(account.settled(), account.declaration().budget())
    }
}

fn publication_error(error: ExecutorError) -> crate::ControlError {
    crate::ControlError::InvalidContract(error.to_string())
}

impl From<milkdrift_capability_host::HostError> for crate::ControlError {
    fn from(error: milkdrift_capability_host::HostError) -> Self {
        Self::InvalidContract(error.to_string())
    }
}
