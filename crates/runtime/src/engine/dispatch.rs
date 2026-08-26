//! Runnable dispatch, durable leases, immutable request construction, and executor reports.

use super::support::{
    CommandPlan, DispatchOutcome, ResolvedInputValue, checked_timestamp_add,
    execution_branch_state, invocation_value_reference, stable_idempotency_key,
};
use super::{CommandExecution, MAX_DURABLE_INVOCATION_REQUEST_BYTES, RuntimeService};
use crate::projection::{
    AttemptState, BranchState, NodeExecutionState, RunLifecycle, RunProjection,
};
use crate::{AdmissionRequest, RunCommand, RunCommandDocument, RuntimeError, SystemTransition};
use milkdrift_blueprint::{BlueprintRevision, Node, NodeKind, ReducerStrategy};
use milkdrift_capability::{
    BoundedJson, ErrorClass, IdempotencyBehavior, IdempotencyKey, InputReference, InvocationId,
    InvocationRequest, SideEffectClass,
};
use milkdrift_persistence::{
    AtomicRunCommitOutcome, AttemptId, BoundedDetail, NodeExecutionId, PersistenceError, Reason,
    RunEventKind, RunnableIndexEntry, TimestampMillis,
};
use milkdrift_workspace::{ArtifactId, ArtifactSensitivity, RunId, ScopeKind, ScopeReference};
use std::collections::{BTreeMap, BTreeSet};
use tracing::{info, warn};

impl RuntimeService {
    pub(super) fn dispatch_runnable(
        &self,
        entry: &RunnableIndexEntry,
        now: TimestampMillis,
    ) -> Result<DispatchOutcome, RuntimeError> {
        // Serialize the exact durable admission snapshot and lease CAS, then release
        // before entering the potentially blocking executor boundary. This prevents
        // same-service oversubscription without suppressing concurrent cancellation.
        let scheduler_guard = self.scheduler_gate.lock().map_err(|_error| {
            RuntimeError::Scheduling("runtime scheduler coordination lock is poisoned".to_owned())
        })?;
        let projection = self.projection(&entry.run)?;
        if projection.sequence() < entry.through_sequence
            || projection.lifecycle() != RunLifecycle::Running
        {
            return Ok(DispatchOutcome::Deferred);
        }
        if self.runnable_executions(&projection)?.get(&entry.execution) != Some(&entry.eligible_at)
        {
            return Ok(DispatchOutcome::Deferred);
        }
        let execution = projection
            .node_executions()
            .get(&entry.execution)
            .ok_or_else(|| {
                RuntimeError::InvalidHistory("runnable execution is absent".to_owned())
            })?;
        if execution_branch_state(&projection, execution.execution())
            .is_some_and(|state| state != BranchState::Active)
        {
            return Ok(DispatchOutcome::Deferred);
        }
        let revision = self.revision_for_execution(&projection, &entry.execution)?;
        let node = revision
            .semantic()
            .nodes()
            .get(execution.node())
            .ok_or_else(|| RuntimeError::InvalidHistory("runnable node is absent".to_owned()))?;
        let requirement = match node.kind() {
            NodeKind::Task { config } => config.requirement().clone(),
            NodeKind::Reducer { config } => match config.strategy() {
                ReducerStrategy::Capability(operation) => {
                    milkdrift_capability::CapabilityRequirement::new(operation.clone())
                }
                ReducerStrategy::Collect | ReducerStrategy::First => {
                    return Ok(DispatchOutcome::Deferred);
                }
            },
            NodeKind::Branch { .. }
            | NodeKind::Fork { .. }
            | NodeKind::Join { .. }
            | NodeKind::Repeat { .. }
            | NodeKind::Wait { .. }
            | NodeKind::SignalWait { .. }
            | NodeKind::Subworkflow { .. }
            | NodeKind::Terminal { .. } => return Ok(DispatchOutcome::Deferred),
        };
        let branch = projection
            .scopes()
            .get(execution.scope())
            .and_then(|scope| match scope.kind() {
                ScopeKind::Branch { branch } => Some(branch.clone()),
                ScopeKind::RunRoot
                | ScopeKind::Iteration { .. }
                | ScopeKind::Subworkflow { .. } => None,
            });
        let admission = AdmissionRequest {
            run: entry.run.clone(),
            branch: branch.clone(),
            operation: requirement.operation().clone(),
        };
        let (usage, lease_revision) = self.admission_usage()?;
        if !self.config.scheduler_limits.allows(&admission, &usage) {
            return Ok(DispatchOutcome::Deferred);
        }
        let resolution = self.executor.resolve(&requirement, now.get())?;
        let contract = resolution.snapshot().operation_contract();
        let attempt = match execution.state() {
            NodeExecutionState::Eligible => self.next_attempt_id()?,
            NodeExecutionState::RetryPending(attempt)
                if projection
                    .attempts()
                    .get(attempt)
                    .is_some_and(|value| value.state() == &AttemptState::ReadyToSchedule) =>
            {
                attempt.clone()
            }
            NodeExecutionState::Scheduled(_)
            | NodeExecutionState::Running(_)
            | NodeExecutionState::RetryPending(_)
            | NodeExecutionState::Uncertain(_)
            | NodeExecutionState::CancelledBeforeDispatch
            | NodeExecutionState::RemovedProspectively(_)
            | NodeExecutionState::Terminal(_) => return Ok(DispatchOutcome::Deferred),
        };
        let invocation = self.next_invocation_id()?;
        let idempotency_key = match contract.idempotency() {
            IdempotencyBehavior::Unsupported => None,
            IdempotencyBehavior::CapabilityScoped | IdempotencyBehavior::ProviderProfileScoped => {
                Some(stable_idempotency_key(&entry.run, execution.execution())?)
            }
        };
        if let NodeExecutionState::RetryPending(retry_attempt) = execution.state() {
            let retry = projection
                .retries()
                .values()
                .find(|retry| retry.next_attempt() == retry_attempt)
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "retry-pending execution has no retry decision".to_owned(),
                    )
                })?;
            let previous = projection
                .attempts()
                .get(retry.previous_attempt())
                .ok_or_else(|| {
                    RuntimeError::InvalidHistory(
                        "retry decision has no previous attempt".to_owned(),
                    )
                })?;
            if previous.side_effect().is_some_and(|classification| {
                classification.side_effect() == SideEffectClass::IdempotentWrite
            }) {
                let same_resolution = previous
                    .capability()
                    .is_some_and(|capability| capability.snapshot() == resolution.snapshot());
                let same_key = previous.idempotency_key() == idempotency_key.as_ref();
                if !same_resolution || !same_key {
                    warn!(
                        run = %entry.run,
                        execution = %execution.execution(),
                        previous_attempt = %previous.attempt(),
                        "idempotent-write retry retained because capability resolution or key changed"
                    );
                    return Ok(DispatchOutcome::Deferred);
                }
            }
        }
        let request = match self.invocation_request(
            &revision,
            &projection,
            node,
            execution.scope(),
            execution.execution(),
            &attempt,
            invocation.clone(),
            resolution.snapshot().capability().clone(),
            resolution.snapshot().provider_profile().cloned(),
            idempotency_key.clone(),
        ) {
            Ok(request) => request,
            Err(RuntimeError::Scheduling(_)) => {
                self.commit_pre_dispatch_failure(
                    &entry.run,
                    now,
                    execution.execution(),
                    "immutable invocation input materialization failed",
                )?;
                return Ok(DispatchOutcome::PreDispatchFailed);
            }
            Err(error) => return Err(error),
        };
        let request_bytes = serde_json::to_vec(&request)
            .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
        if request_bytes.len() > MAX_DURABLE_INVOCATION_REQUEST_BYTES {
            self.commit_pre_dispatch_failure(
                &entry.run,
                now,
                execution.execution(),
                "immutable invocation request exceeds the durable event size budget",
            )?;
            return Ok(DispatchOutcome::PreDispatchFailed);
        }
        let lease = self.next_lease_id()?;
        let expires_at = checked_timestamp_add(now, self.config.lease_duration_ms)?;
        let schedule = CommandPlan {
            events: vec![
                RunEventKind::NodeScheduled {
                    node: node.id().clone(),
                    execution: execution.execution().clone(),
                    attempt: attempt.clone(),
                    invocation: invocation.clone(),
                    idempotency_key: idempotency_key.clone(),
                    request: request.clone(),
                },
                RunEventKind::CapabilityResolved {
                    execution: execution.execution().clone(),
                    attempt: attempt.clone(),
                    requirement: requirement.clone(),
                    snapshot: resolution.snapshot().clone(),
                },
                RunEventKind::SideEffectClassified {
                    attempt: attempt.clone(),
                    side_effect: contract.side_effect(),
                    idempotency: contract.idempotency(),
                    idempotency_key: idempotency_key.clone(),
                },
                RunEventKind::LeaseGranted {
                    lease: lease.clone(),
                    execution: execution.execution().clone(),
                    attempt: attempt.clone(),
                    worker: self.config.worker.clone(),
                    expires_at,
                },
            ],
            expected_lease_revision: Some(lease_revision),
            ..CommandPlan::default()
        };
        match self.commit_internal_plan(
            &entry.run,
            now,
            SystemTransition::ScheduleAndLease {
                attempt: attempt.clone(),
            },
            schedule,
        ) {
            Ok(_) => {}
            Err(RuntimeError::Persistence(PersistenceError::LeaseRevisionConflict { .. }))
            | Err(RuntimeError::Persistence(PersistenceError::SequenceConflict { .. })) => {
                return Ok(DispatchOutcome::Deferred);
            }
            Err(error) => return Err(error),
        }
        drop(scheduler_guard);
        info!(
            run = %entry.run,
            revision = %revision.id(),
            node = %node.id(),
            execution = %execution.execution(),
            attempt = %attempt,
            invocation = %invocation,
            lease = %lease,
            "durable lease committed for caller-owned effect execution"
        );
        Ok(DispatchOutcome::Dispatched)
    }

    fn commit_pre_dispatch_failure(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        execution: &NodeExecutionId,
        detail: &'static str,
    ) -> Result<(), RuntimeError> {
        let plan = CommandPlan::one(RunEventKind::NodePreDispatchFailed {
            execution: execution.clone(),
            error_class: ErrorClass::InvalidRequest,
            detail: Some(BoundedDetail::new(detail)?),
        });
        let _ = self.commit_internal_plan(
            run,
            occurred_at,
            SystemTransition::TerminalizePreDispatchFailure {
                execution: execution.clone(),
            },
            plan,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn invocation_request(
        &self,
        revision: &BlueprintRevision,
        projection: &RunProjection,
        node: &Node,
        occurrence_scope: &ScopeReference,
        execution: &NodeExecutionId,
        attempt: &AttemptId,
        invocation: InvocationId,
        capability: milkdrift_capability::CapabilityId,
        provider_profile: Option<milkdrift_capability::ProviderProfileRef>,
        idempotency_key: Option<IdempotencyKey>,
    ) -> Result<InvocationRequest, RuntimeError> {
        self.validate_projected_scope(projection, occurrence_scope, &[])?;
        let mut inputs = Vec::new();
        for (port, declaration) in node.data_inputs() {
            let resolved = match node.kind() {
                NodeKind::Reducer { config }
                    if matches!(config.strategy(), ReducerStrategy::Capability(_))
                        && port == config.input_port() =>
                {
                    let references = self.ordered_reducer_references(
                        revision,
                        projection,
                        node,
                        port,
                        occurrence_scope,
                        &[],
                    )?;
                    if references.len() < usize::from(config.minimum_items()) {
                        return Err(RuntimeError::Scheduling(format!(
                            "capability reducer {} requires at least {} collected inputs",
                            node.id(),
                            config.minimum_items()
                        )));
                    }
                    vec![ResolvedInputValue::Inline {
                        value: BoundedJson::new(serde_json::to_value(references)?)
                            .map_err(|error| RuntimeError::Scheduling(error.to_string()))?,
                        source: None,
                    }]
                }
                NodeKind::Task { .. }
                | NodeKind::Reducer { .. }
                | NodeKind::Branch { .. }
                | NodeKind::Fork { .. }
                | NodeKind::Join { .. }
                | NodeKind::Repeat { .. }
                | NodeKind::Wait { .. }
                | NodeKind::SignalWait { .. }
                | NodeKind::Subworkflow { .. }
                | NodeKind::Terminal { .. } => self.resolve_node_port_inputs(
                    revision,
                    projection,
                    node,
                    port,
                    occurrence_scope,
                    &[],
                )?,
            };
            if resolved.is_empty() {
                if declaration.is_required() {
                    return Err(RuntimeError::Scheduling(format!(
                        "required task input {}:{} is unresolved",
                        node.id(),
                        port
                    )));
                }
                continue;
            }
            if resolved.len() != 1 {
                return Err(RuntimeError::Scheduling(format!(
                    "task input {}:{} resolved to more than one exact value",
                    node.id(),
                    port
                )));
            }
            let resolved_value = resolved.into_iter().next().ok_or_else(|| {
                RuntimeError::InvalidHistory("resolved invocation input disappeared".to_owned())
            })?;
            let value = invocation_value_reference(resolved_value)?;
            inputs.push(
                InputReference::new(port.as_str().to_owned(), value)
                    .map_err(|error| RuntimeError::Scheduling(error.to_string()))?,
            );
        }
        let request = InvocationRequest::new(
            invocation,
            capability,
            match node.kind() {
                NodeKind::Task { config } => config.requirement().operation().clone(),
                NodeKind::Reducer { config } => match config.strategy() {
                    ReducerStrategy::Capability(operation) => operation.clone(),
                    ReducerStrategy::Collect | ReducerStrategy::First => {
                        return Err(RuntimeError::Scheduling(
                            "a deterministic reducer cannot build an invocation".to_owned(),
                        ));
                    }
                },
                NodeKind::Branch { .. }
                | NodeKind::Fork { .. }
                | NodeKind::Join { .. }
                | NodeKind::Repeat { .. }
                | NodeKind::Wait { .. }
                | NodeKind::SignalWait { .. }
                | NodeKind::Subworkflow { .. }
                | NodeKind::Terminal { .. } => {
                    return Err(RuntimeError::Scheduling(
                        "only task or capability-backed reducer nodes can build invocations"
                            .to_owned(),
                    ));
                }
            },
            provider_profile,
            idempotency_key,
            inputs,
            BTreeMap::new(),
        )
        .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
        let NodeKind::Task { config } = node.kind() else {
            return Ok(request);
        };
        if config.requirement().operation().as_str() != milkdrift_model::MODEL_GENERATE_OPERATION {
            return Ok(request);
        }
        let mut candidates = Vec::with_capacity(request.inputs().len());
        for input in request.inputs() {
            let selected_bytes = u64::try_from(
                serde_json::to_vec(input.value())
                    .map_err(|error| RuntimeError::Scheduling(error.to_string()))?
                    .len(),
            )
            .map_err(|_| RuntimeError::Scheduling("context input size overflow".to_owned()))?;
            let (artifact_bytes, sensitivity, authority, available) = match input.value() {
                milkdrift_capability::InvocationValueReference::Artifact { reference } => {
                    let metadata = self
                        .store
                        .metadata(
                            &ArtifactId::new(reference.identity())
                                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?,
                        )?
                        .filter(|metadata| {
                            reference.size_bytes() == Some(metadata.reference().size_bytes())
                                && reference.digest() == metadata.reference().digest().to_hex()
                                && reference.media_type()
                                    == Some(metadata.reference().media_type().as_str())
                        });
                    let sensitivity = metadata
                        .as_ref()
                        .map_or(ArtifactSensitivity::Restricted, |value| value.sensitivity());
                    (
                        reference.size_bytes().unwrap_or(0),
                        sensitivity,
                        milkdrift_model::AuthorityFact {
                            required: sensitivity != ArtifactSensitivity::Public,
                            authorized: metadata.is_some(),
                            authority_reference: None,
                        },
                        metadata.is_some(),
                    )
                }
                milkdrift_capability::InvocationValueReference::Inline { .. }
                | milkdrift_capability::InvocationValueReference::WorkspaceValue { .. } => (
                    0,
                    ArtifactSensitivity::Restricted,
                    milkdrift_model::AuthorityFact {
                        required: false,
                        authorized: true,
                        authority_reference: None,
                    },
                    true,
                ),
            };
            candidates.push(crate::ContextCandidate {
                kind: milkdrift_model::ContextSemanticKind::DirectInput,
                source: Some(milkdrift_model::ContextSource::DirectInput {
                    name: input.name().to_owned(),
                    reference: input.value().clone(),
                }),
                node: None,
                roles: BTreeSet::new(),
                scope: Some(occurrence_scope.clone()),
                exposed_across_scope: false,
                required: node
                    .data_inputs()
                    .iter()
                    .find(|(port, _)| port.as_str() == input.name())
                    .is_some_and(|(_, declaration)| declaration.is_required()),
                available,
                selected_bytes,
                selected_artifact_bytes: artifact_bytes,
                estimated_model_input_units: None,
                sensitivity,
                authority,
                artifact: None,
                causal_parents: Vec::new(),
            });
        }
        let visible_scopes = self
            .store
            .scope_lineage(occurrence_scope)?
            .into_iter()
            .map(|scope| scope.reference().clone())
            .collect();
        let run = projection.run_id().ok_or_else(|| {
            RuntimeError::InvalidHistory("run projection has no identity".to_owned())
        })?;
        let manifest = crate::CausalContextBuilder::build(crate::ContextBuildRequest {
            identity: crate::ContextBuildIdentity {
                run: run.clone(),
                revision: revision.id().clone(),
                node: node.id().clone(),
                execution: execution.clone(),
                attempt: attempt.clone(),
            },
            semantic: revision.semantic(),
            policy: config.context_policy(),
            visible_scopes,
            candidates,
        })
        .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
        let budget = projection.workspace_budget().ok_or_else(|| {
            RuntimeError::InvalidHistory("run has no workspace budget".to_owned())
        })?;
        let usage = self.store.workspace_usage(run)?;
        let manifest =
            crate::persist_context_manifest(self.store.as_ref(), &manifest, budget.clone(), usage)
                .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
        request
            .with_context_manifest(manifest)
            .map_err(|error| RuntimeError::Scheduling(error.to_string()))
    }

    pub(super) fn commit_internal_plan(
        &self,
        run: &RunId,
        occurred_at: TimestampMillis,
        transition: SystemTransition,
        plan: CommandPlan,
    ) -> Result<CommandExecution, RuntimeError> {
        let projection = self.projection(run)?;
        let reason = Reason::new(format!("internal runtime action: {}", transition.label()))?;
        let document = RunCommandDocument::new(
            self.next_command_id()?,
            run.clone(),
            self.config.internal_actor.clone(),
            projection.sequence(),
            occurred_at,
            reason,
            Vec::new(),
            RunCommand::SystemTransition { transition },
        )?;
        let receipt = document.receipt()?;
        let outcome = self.commit_accepted(&document, receipt, projection, plan, None)?;
        let (result, replayed) = match outcome {
            AtomicRunCommitOutcome::Committed(value) => (value, false),
            AtomicRunCommitOutcome::Replayed(value) => (value, true),
        };
        Ok(CommandExecution { result, replayed })
    }
}
