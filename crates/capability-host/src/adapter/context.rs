use super::DirectInputSelection;
use milkdrift_authority::{AuthorityDecisionSnapshot, ExecutionAuthorityBasis};
use milkdrift_blueprint::{NodeId, RevisionId};
use milkdrift_persistence::{AttemptId, NodeExecutionId};
use milkdrift_workspace::RunId;

/// Immutable task-origin coordinates; remote origins never own a local workflow.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowExecutionContext {
    run: RunId,
    revision: RevisionId,
    node: NodeId,
    execution: NodeExecutionId,
    attempt: AttemptId,
}
impl WorkflowExecutionContext {
    /// Owning workflow identity.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }
    /// Frozen definition identity carried by the authority protocol.
    #[must_use]
    pub const fn revision(&self) -> &RevisionId {
        &self.revision
    }
    /// Task identity carried by the authority protocol.
    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }
    /// Exact logical execution identity.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }
    /// Exact accepted attempt identity.
    #[must_use]
    pub const fn attempt(&self) -> &AttemptId {
        &self.attempt
    }
}

/// Frozen selected bytes and references; no adapter performs a workflow-history lookup.
#[derive(Clone, Debug, PartialEq)]
pub enum AdapterInputSelection {
    /// Work with an immutable originating workflow/task identity.
    Task {
        /// Originating task coordinates.
        workflow: WorkflowExecutionContext,
        /// Exact accepted request inputs.
        selection: DirectInputSelection,
    },
    /// Independently accepted work without workflow coordinates.
    Direct(DirectInputSelection),
}

/// Frozen selection, authority and output ownership supplied by an execution owner.
#[derive(Clone, Debug, PartialEq)]
pub struct AdapterExecutionContext {
    task_artifact_budget: Option<milkdrift_workspace::WorkspaceBudget>,
    task_output_bytes: Option<u64>,
    pub(crate) task_paths: Vec<milkdrift_blueprint::TaskPathAccess>,
    pub(crate) task_tools: Vec<milkdrift_blueprint::TaskTool>,
    pub(crate) maximum_tool_calls: u32,
    pub(crate) task_publication_invocation: Option<milkdrift_capability::InvocationId>,
    selection: AdapterInputSelection,
    authority: Option<ExecutionAuthorityBasis>,
    resolution_authorization: Option<AuthorityDecisionSnapshot>,
    entry_authorization: Option<AuthorityDecisionSnapshot>,
    peer_artifacts: Option<(
        milkdrift_workspace::ArtifactOwner,
        milkdrift_workspace::WorkspaceBudget,
    )>,
}

impl AdapterExecutionContext {
    /// Capture exact inputs alongside independently validated originating coordinates.
    /// Publication additionally requires an accepted task dispatch or durable serving entry.
    #[must_use]
    pub fn originated(
        run: RunId,
        revision: RevisionId,
        node: NodeId,
        execution: NodeExecutionId,
        attempt: AttemptId,
        selection: DirectInputSelection,
    ) -> Self {
        let mut context = Self::direct(selection.clone());
        context.selection = AdapterInputSelection::Task {
            workflow: WorkflowExecutionContext {
                run,
                revision,
                node,
                execution,
                attempt,
            },
            selection,
        };
        context
    }
    /// A serving preparation context whose caller has authorized these exact inputs.
    #[must_use]
    pub const fn direct(selection: DirectInputSelection) -> Self {
        Self {
            task_artifact_budget: None,
            task_output_bytes: None,
            task_paths: Vec::new(),
            task_tools: Vec::new(),
            maximum_tool_calls: 0,
            task_publication_invocation: None,
            selection: AdapterInputSelection::Direct(selection),
            authority: None,
            resolution_authorization: None,
            entry_authorization: None,
            peer_artifacts: None,
        }
    }
    /// Frozen selected inputs and their optional originating coordinates.
    #[must_use]
    pub const fn selection(&self) -> &AdapterInputSelection {
        &self.selection
    }
    /// Originating workflow coordinates when present.
    #[must_use]
    pub const fn workflow(&self) -> Option<&WorkflowExecutionContext> {
        match &self.selection {
            AdapterInputSelection::Task { workflow, .. } => Some(workflow),
            AdapterInputSelection::Direct(_) => None,
        }
    }
    /// Exact request inputs, with no causal-history expansion.
    #[must_use]
    pub const fn direct_selection(&self) -> Option<&DirectInputSelection> {
        Some(match &self.selection {
            AdapterInputSelection::Task { selection, .. }
            | AdapterInputSelection::Direct(selection) => selection,
        })
    }
    /// Materialization facts from the runtime's immutable task dispatch.
    ///
    /// # Errors
    /// Refuses a request whose complete selected input document exceeds the task bound.
    pub fn from_dispatch(
        dispatch: &milkdrift_runtime::ExecutionDispatch,
    ) -> Result<Self, crate::InvocationDataError> {
        let selection = DirectInputSelection::new(
            dispatch.request(),
            milkdrift_blueprint::MAX_TASK_DEFINITION_BYTES as u64,
        )?;
        let mut context = Self::originated(
            dispatch.run().clone(),
            dispatch.revision().clone(),
            dispatch.node().clone(),
            dispatch.execution().clone(),
            dispatch.attempt().clone(),
            selection,
        );
        context.task_artifact_budget = dispatch.task_artifact_budget().cloned();
        context.task_output_bytes = dispatch.task_output_bytes();
        context.task_paths = dispatch.native_path_claims().to_vec();
        context.task_tools = dispatch.task_tools().to_vec();
        context.maximum_tool_calls = dispatch.maximum_tool_calls();
        context.task_publication_invocation = Some(dispatch.request().invocation().clone());
        context.authority = Some(dispatch.execution_authority().clone());
        context.resolution_authorization = Some(dispatch.resolution_authorization().clone());
        context.entry_authorization = Some(dispatch.entry_authorization().clone());
        Ok(context)
    }
    pub(crate) fn with_serving_preparation(
        mut self,
        record: &milkdrift_persistence::PeerExecutionRecord,
    ) -> Self {
        self.entry_authorization = Some(record.authority.clone());
        self
    }
    pub(crate) fn with_serving_execution(
        mut self,
        record: &milkdrift_persistence::PeerExecutionRecord,
    ) -> Result<Self, crate::InvocationDataError> {
        let selected = self
            .direct_selection()
            .is_some_and(|selection| selection.validate_request(&record.request.request).is_ok());
        let origin_matches = match (self.workflow(), record.request.authorization.origin()) {
            (None, milkdrift_peer_protocol::InvocationOrigin::Direct) => true,
            (
                Some(workflow),
                milkdrift_peer_protocol::InvocationOrigin::Workflow { provenance },
            ) => {
                provenance.run == workflow.run.as_str()
                    && provenance.revision == workflow.revision.as_str()
                    && provenance.node == workflow.node.as_str()
                    && provenance.execution == workflow.execution.as_str()
                    && provenance.attempt == workflow.attempt.as_str()
            }
            _ => false,
        };
        if !matches!(
            record.phase,
            milkdrift_persistence::PeerExecutionPhase::Entered { .. }
                | milkdrift_persistence::PeerExecutionPhase::CancellationRequested {
                    evidence: Some(_),
                    ..
                }
        ) || record.caller != record.request.authorization.caller()
            || !selected
            || !origin_matches
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
        self.entry_authorization = record
            .phase
            .entry_evidence()
            .map(|evidence| evidence.authority.clone());
        self.peer_artifacts = Some((owner, budget));
        Ok(self)
    }
    pub(crate) fn publication_owner(
        &self,
    ) -> Result<milkdrift_workspace::ArtifactOwner, crate::InvocationDataError> {
        if let Some((owner, _)) = &self.peer_artifacts {
            return Ok(owner.clone());
        }
        if self.task_artifact_budget.is_some()
            && self.task_output_bytes.is_some()
            && let Some(workflow) = self.workflow()
        {
            return Ok((&workflow.run).into());
        }
        Err(crate::InvocationDataError::Rejected(
            "output publication requires a durably entered task or serving execution".to_owned(),
        ))
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
                        "publication namespace is absent".to_owned(),
                    )
                }),
        }
    }
    pub(crate) fn peer_artifact_budget(&self) -> Option<&milkdrift_workspace::WorkspaceBudget> {
        self.peer_artifacts.as_ref().map(|(_, budget)| budget)
    }
    pub(crate) fn task_artifact_budget(&self) -> Option<&milkdrift_workspace::WorkspaceBudget> {
        self.task_artifact_budget.as_ref()
    }
    pub(crate) const fn task_output_bytes(&self) -> Option<u64> {
        self.task_output_bytes
    }
    /// Frozen actor/grant/policy basis of local task acceptance.
    #[must_use]
    pub const fn authority(&self) -> Option<&ExecutionAuthorityBasis> {
        self.authority.as_ref()
    }
    /// Decision permitting this exact capability selection.
    #[must_use]
    pub const fn resolution_authorization(&self) -> Option<&AuthorityDecisionSnapshot> {
        self.resolution_authorization.as_ref()
    }
    /// Fresh decision committed immediately before external entry.
    #[must_use]
    pub const fn entry_authorization(&self) -> Option<&AuthorityDecisionSnapshot> {
        self.entry_authorization.as_ref()
    }

    pub(crate) fn for_model_request(
        &self,
        request: &milkdrift_capability::InvocationRequest,
    ) -> Result<Self, crate::InvocationDataError> {
        let selection = DirectInputSelection::new(
            request,
            milkdrift_blueprint::MAX_TASK_DEFINITION_BYTES as u64,
        )?;
        let mut context = self.clone();
        context.selection = match self.workflow() {
            Some(workflow) => AdapterInputSelection::Task {
                workflow: workflow.clone(),
                selection,
            },
            None => AdapterInputSelection::Direct(selection),
        };
        Ok(context)
    }
}
