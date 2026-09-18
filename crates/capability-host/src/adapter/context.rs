use super::DirectInputSelection;
use milkdrift_authority::{AuthorityDecisionSnapshot, ExecutionAuthorityBasis};
use milkdrift_blueprint::{NodeId, RevisionId};
use milkdrift_persistence::{AttemptId, ControllerReservationId, NodeExecutionId};
use milkdrift_workspace::RunId;

/// Exact local or originating workflow coordinates; direct work has none.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowExecutionContext {
    run: RunId,
    revision: RevisionId,
    node: NodeId,
    execution: NodeExecutionId,
    attempt: AttemptId,
}

impl WorkflowExecutionContext {
    /// Originating durable run.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }
    /// Immutable workflow revision.
    #[must_use]
    pub const fn revision(&self) -> &RevisionId {
        &self.revision
    }
    /// Semantic node identity.
    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }
    /// Logical execution identity.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }
    /// Exact execution attempt.
    #[must_use]
    pub const fn attempt(&self) -> &AttemptId {
        &self.attempt
    }
}

/// How the owner selected the inputs supplied to this adapter.
#[derive(Clone, Debug, PartialEq)]
pub enum AdapterInputSelection {
    /// A causal selection whose manifest must match these exact coordinates.
    Workflow(WorkflowExecutionContext),
    /// Only the accepted caller-supplied inputs, without a workflow-history lookup.
    Direct(DirectInputSelection),
}

/// Exact input selection and durable authority supplied to materializing adapters.
#[derive(Clone, Debug, PartialEq)]
pub struct AdapterExecutionContext {
    selection: AdapterInputSelection,
    authority: Option<ExecutionAuthorityBasis>,
    resolution_authorization: Option<AuthorityDecisionSnapshot>,
    entry_authorization: Option<AuthorityDecisionSnapshot>,
    controller_reservation: Option<ControllerReservationId>,
    peer_artifacts: Option<(
        milkdrift_workspace::ArtifactOwner,
        milkdrift_workspace::WorkspaceBudget,
    )>,
}

impl AdapterExecutionContext {
    /// Constructs exact durable provenance for an already validated execution dispatch.
    #[must_use]
    pub const fn new(
        run: RunId,
        revision: RevisionId,
        node: NodeId,
        execution: NodeExecutionId,
        attempt: AttemptId,
    ) -> Self {
        Self {
            selection: AdapterInputSelection::Workflow(WorkflowExecutionContext {
                run,
                revision,
                node,
                execution,
                attempt,
            }),
            authority: None,
            resolution_authorization: None,
            entry_authorization: None,
            controller_reservation: None,
            peer_artifacts: None,
        }
    }

    /// Constructs a direct preparation context after the serving owner has authorized inputs.
    /// Output publication remains unavailable until a durable serving entry binds its owner.
    #[must_use]
    pub const fn direct(selection: DirectInputSelection) -> Self {
        Self {
            selection: AdapterInputSelection::Direct(selection),
            authority: None,
            resolution_authorization: None,
            entry_authorization: None,
            controller_reservation: None,
            peer_artifacts: None,
        }
    }

    /// Exact input-selection meaning; callers must handle direct and workflow work explicitly.
    #[must_use]
    pub const fn selection(&self) -> &AdapterInputSelection {
        &self.selection
    }

    /// Originating workflow coordinates, if this work came from a workflow.
    #[must_use]
    pub const fn workflow(&self) -> Option<&WorkflowExecutionContext> {
        match &self.selection {
            AdapterInputSelection::Workflow(workflow) => Some(workflow),
            AdapterInputSelection::Direct(_) => None,
        }
    }

    /// Frozen caller-supplied selection for direct work.
    #[must_use]
    pub const fn direct_selection(&self) -> Option<&DirectInputSelection> {
        match &self.selection {
            AdapterInputSelection::Direct(direct) => Some(direct),
            AdapterInputSelection::Workflow(_) => None,
        }
    }

    pub(crate) fn from_dispatch(
        dispatch: &milkdrift_runtime::ExecutionDispatch,
        controller_reservation: Option<&ControllerReservationId>,
    ) -> Self {
        Self {
            selection: AdapterInputSelection::Workflow(WorkflowExecutionContext {
                run: dispatch.run().clone(),
                revision: dispatch.revision().clone(),
                node: dispatch.node().clone(),
                execution: dispatch.execution().clone(),
                attempt: dispatch.attempt().clone(),
            }),
            authority: Some(dispatch.execution_authority().clone()),
            resolution_authorization: Some(dispatch.resolution_authorization().clone()),
            entry_authorization: Some(dispatch.entry_authorization().clone()),
            controller_reservation: controller_reservation.cloned(),
            peer_artifacts: None,
        }
    }

    /// Binds artifact publication to the serving host's durably entered peer execution.
    /// The accepted request owns this allowance; the originating run is not a local run.
    pub(crate) fn with_serving_execution(
        mut self,
        record: &milkdrift_persistence::PeerExecutionRecord,
    ) -> Result<Self, crate::InvocationDataError> {
        let origin = record.request.authorization.origin();
        let selection_matches = match (&self.selection, &origin) {
            (
                AdapterInputSelection::Workflow(workflow),
                milkdrift_peer_protocol::InvocationOrigin::Workflow { provenance },
            ) => {
                provenance.run == workflow.run.as_str()
                    && provenance.revision == workflow.revision.as_str()
                    && provenance.node == workflow.node.as_str()
                    && provenance.execution == workflow.execution.as_str()
                    && provenance.attempt == workflow.attempt.as_str()
            }
            (
                AdapterInputSelection::Direct(selection),
                milkdrift_peer_protocol::InvocationOrigin::Direct,
            ) => selection.validate_request(&record.request.request).is_ok(),
            _ => false,
        };
        if !matches!(
            record.phase,
            milkdrift_persistence::PeerExecutionPhase::Entered { .. }
        ) || record.caller != record.request.authorization.caller()
            || !selection_matches
            || self.authority.is_some()
            || self.peer_artifacts.is_some()
        {
            return Err(crate::InvocationDataError::Rejected(
                "peer publication requires the exact entered execution".to_owned(),
            ));
        }
        record
            .request
            .validate()
            .map_err(|error| crate::InvocationDataError::Rejected(error.to_string()))?;
        let limits = &record.request.limits;
        let input_bytes = record
            .request
            .input_artifact_bytes()
            .map_err(|error| crate::InvocationDataError::Rejected(error.to_string()))?;
        let remaining = limits
            .artifact_bytes
            .checked_sub(input_bytes)
            .ok_or_else(|| {
                crate::InvocationDataError::Rejected("peer artifact allowance exhausted".to_owned())
            })?;
        let owner = milkdrift_workspace::ArtifactOwner::HostInvocation {
            host: record.request.authorization.host().clone(),
            invocation: crate::serving::prepared::serving_invocation(record)
                .map_err(|error| crate::InvocationDataError::Rejected(error.to_string()))?,
        };
        let budget = milkdrift_workspace::WorkspaceBudget::new(
            0,
            0,
            0,
            u64::from(limits.observations),
            remaining,
            remaining,
        )
        .map_err(|error| crate::InvocationDataError::Rejected(error.to_string()))?;
        self.peer_artifacts = Some((owner, budget));
        Ok(self)
    }

    pub(crate) fn publication_owner(
        &self,
    ) -> Result<milkdrift_workspace::ArtifactOwner, crate::InvocationDataError> {
        if let Some((owner, _)) = &self.peer_artifacts {
            return Ok(owner.clone());
        }
        self.workflow()
            .map(|workflow| (&workflow.run).into())
            .ok_or_else(|| {
                crate::InvocationDataError::Rejected(
                    "direct publication requires a durably entered serving execution".to_owned(),
                )
            })
    }

    pub(crate) fn publication_namespace(&self) -> Result<&str, crate::InvocationDataError> {
        match self.peer_artifacts.as_ref().map(|(owner, _)| owner) {
            Some(milkdrift_workspace::ArtifactOwner::HostInvocation { invocation, .. }) => {
                Ok(invocation.as_str())
            }
            _ => self
                .workflow()
                .map(|workflow| workflow.run.as_str())
                .ok_or_else(|| {
                    crate::InvocationDataError::Rejected(
                        "direct publication requires a durably entered serving execution"
                            .to_owned(),
                    )
                }),
        }
    }

    pub(crate) fn peer_artifact_budget(&self) -> Option<&milkdrift_workspace::WorkspaceBudget> {
        self.peer_artifacts.as_ref().map(|(_, budget)| budget)
    }

    /// Exact controller reservation committed before this adapter entry, when controlled.
    #[must_use]
    pub const fn controller_reservation(&self) -> Option<&ControllerReservationId> {
        self.controller_reservation.as_ref()
    }

    /// Frozen actor/grant/policy basis inherited from run acceptance.
    #[must_use]
    pub const fn authority(&self) -> Option<&ExecutionAuthorityBasis> {
        self.authority.as_ref()
    }

    /// Exact decision that allowed this capability generation to be selected.
    #[must_use]
    pub const fn resolution_authorization(&self) -> Option<&AuthorityDecisionSnapshot> {
        self.resolution_authorization.as_ref()
    }

    /// Fresh decision committed immediately before adapter entry.
    #[must_use]
    pub const fn entry_authorization(&self) -> Option<&AuthorityDecisionSnapshot> {
        self.entry_authorization.as_ref()
    }
}
