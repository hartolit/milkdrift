//! Run authority establishment and prospective workflow-envelope validation.

use std::collections::{BTreeSet, VecDeque};

use milkdrift_authority::{
    AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityExecutionProvenance,
    AuthorityOperation, BoundaryTimeMillis, CapabilityAuthorityScope, DecisionId,
    ExecutionAuthorityBasis, RequestedResourceFacts,
};
use milkdrift_blueprint::{NodeKind, ReducerStrategy, RevisionId};
use milkdrift_persistence::RunEventKind;

use super::RuntimeService;
use super::support::CommandPlan;
use crate::{RunCommand, RunCommandDocument, RunProjection, RuntimeError};

const MAX_AUTHORITY_REVISION_WALK: usize = 512;

pub(super) struct ExecutionAuthorityError {
    pub(super) error: RuntimeError,
    pub(super) decision: Option<Box<AuthorityDecisionSnapshot>>,
}

impl From<RuntimeError> for ExecutionAuthorityError {
    fn from(error: RuntimeError) -> Self {
        Self {
            error,
            decision: None,
        }
    }
}

impl From<AuthorityError> for ExecutionAuthorityError {
    fn from(error: AuthorityError) -> Self {
        Self::from(RuntimeError::from(error))
    }
}

impl RuntimeService {
    pub(super) fn bind_execution_authority(
        &self,
        document: &RunCommandDocument,
        projection: &RunProjection,
        authorization: Option<&AuthorityDecisionSnapshot>,
        plan: &mut CommandPlan,
    ) -> Result<(), ExecutionAuthorityError> {
        match document.command() {
            RunCommand::StartRun => {
                if projection.execution_authority().is_some() {
                    self.validate_revision_authority(
                        projection.execution_authority().ok_or_else(|| {
                            RuntimeError::InvalidHistory(
                                "started child run lost inherited execution authority".to_owned(),
                            )
                        })?,
                        projection.revision().ok_or_else(|| {
                            RuntimeError::InvalidHistory("created run has no revision".to_owned())
                        })?,
                        document.issued_at().get(),
                    )?;
                    return Ok(());
                }
                let decision = authorization.ok_or_else(|| {
                    RuntimeError::InvalidCommand(
                        "external run start requires an authority decision".to_owned(),
                    )
                })?;
                let workflow = projection.workflow().ok_or_else(|| {
                    RuntimeError::InvalidHistory("created run has no workflow".to_owned())
                })?;
                let revision = projection.revision().ok_or_else(|| {
                    RuntimeError::InvalidHistory("created run has no revision".to_owned())
                })?;
                let basis = ExecutionAuthorityBasis::from_start_decision(
                    decision,
                    workflow.clone(),
                    document.run_id().clone(),
                    revision.clone(),
                )?;
                self.validate_revision_authority(&basis, revision, document.issued_at().get())?;
                plan.events
                    .insert(0, RunEventKind::ExecutionAuthorityEstablished { basis });
            }
            RunCommand::RequestRevisionAdoption { revision, .. } => {
                let basis = projection.execution_authority().ok_or_else(|| {
                    RuntimeError::InvalidTransition(
                        "revision adoption requires established run execution authority".to_owned(),
                    )
                })?;
                self.validate_revision_authority(basis, revision, document.issued_at().get())?;
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn validate_revision_authority(
        &self,
        basis: &ExecutionAuthorityBasis,
        root: &RevisionId,
        evaluated_at_unix_ms: u64,
    ) -> Result<(), ExecutionAuthorityError> {
        let mut pending = VecDeque::from([root.clone()]);
        let mut visited = BTreeSet::new();
        while let Some(revision_id) = pending.pop_front() {
            if !visited.insert(revision_id.clone()) {
                continue;
            }
            if visited.len() > MAX_AUTHORITY_REVISION_WALK {
                return Err(RuntimeError::InvalidTransition(
                    "workflow authority validation exceeded the bounded revision graph".to_owned(),
                )
                .into());
            }
            let revision = self.load_validated_revision(&revision_id, None)?;
            for node in revision.semantic().nodes().values() {
                let requirement = match node.kind() {
                    NodeKind::Task { config } => Some(config.requirement().clone()),
                    NodeKind::Reducer { config } => match config.strategy() {
                        ReducerStrategy::Capability(operation) => Some(
                            milkdrift_capability::CapabilityRequirement::new(operation.clone()),
                        ),
                        ReducerStrategy::Collect | ReducerStrategy::First => None,
                    },
                    NodeKind::Subworkflow { reference } => {
                        pending.push_back(reference.revision().clone());
                        None
                    }
                    NodeKind::Repeat { config } => {
                        pending.push_back(config.body().revision().clone());
                        None
                    }
                    NodeKind::Branch { .. }
                    | NodeKind::Fork { .. }
                    | NodeKind::Join { .. }
                    | NodeKind::Wait { .. }
                    | NodeKind::SignalWait { .. }
                    | NodeKind::Terminal { .. } => None,
                };
                let Some(requirement) = requirement else {
                    continue;
                };
                let envelope = CapabilityAuthorityScope::requirement_envelope(&requirement)?;
                let mut resources = RequestedResourceFacts::empty();
                resources.capability = requirement.exact_capability().cloned();
                resources.capability_operation = Some(requirement.operation().clone());
                resources.provider_profile = requirement.provider_profile_ref().cloned();
                resources.execution_trust_class = requirement.execution_trust_class();
                resources.side_effect = requirement.maximum_side_effect_class();
                resources.capability_envelope = Some(envelope);
                let identity = format!(
                    "{}:{}:{}:requirement",
                    basis.digest(),
                    revision_id,
                    node.id(),
                );
                let digest = blake3::hash(identity.as_bytes());
                let request = basis.request(
                    DecisionId::new(format!("decision:{digest}"))?,
                    AuthorityOperation::InvokeCapability,
                    resources,
                    AuthorityBudget {
                        invocations: Some(1),
                        concurrency: Some(1),
                        ..AuthorityBudget::default()
                    },
                    BoundaryTimeMillis::new(evaluated_at_unix_ms),
                    AuthorityExecutionProvenance {
                        revision: Some(revision_id.clone()),
                        node: Some(node.id().clone()),
                        ..AuthorityExecutionProvenance::default()
                    },
                );
                let decision = self.authority.evaluate(&request)?;
                if !decision.is_allowed() {
                    return Err(ExecutionAuthorityError {
                        error: RuntimeError::AuthorizationDenied {
                            decision: decision.digest().to_owned(),
                            reasons: decision.reason_codes().to_vec(),
                        },
                        decision: Some(Box::new(decision)),
                    });
                }
            }
        }
        Ok(())
    }
}
