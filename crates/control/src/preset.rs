use std::collections::{BTreeMap, BTreeSet};

use milkdrift_authority::{
    ActorRef, ArtifactAuthorityScope, AuthorityBudget, AuthorityGrant, AuthorityGrantBuilder,
    AuthorityOperation, BoundaryTimeMillis, CapabilityAuthorityScope, DaemonAuthorityScope,
    GrantId, LayoutAuthorityScope, NetworkScope, PeerAuthorityScope, ResourceScope,
    WorkflowRunScope, WorkspaceAuthorityScope,
};
use serde::{Deserialize, Serialize};

use crate::ControlError;

/// Convenient role names that expand into ordinary immutable grants.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityPreset {
    /// Inspect only the exact state permitted by resource scope.
    Observer,
    /// Inspect and submit prospective proposals.
    Advisor,
    /// Pause, resume, retry/cancel, approve, and apply within exact scope.
    Supervisor,
    /// Apply broader prospective revisions within exact scope and budgets.
    Controller,
    /// Repeat controller operations under strict caller-supplied ceilings.
    Autonomous,
}

impl AuthorityPreset {
    /// Expands this name into a grant template without granting any resource wildcard implicitly.
    #[must_use]
    pub fn template(
        self,
        identity: GrantId,
        revision: u64,
        actor: ActorRef,
        workflow_run: WorkflowRunScope,
        capability: CapabilityAuthorityScope,
        budget: AuthorityBudget,
    ) -> GrantTemplate {
        GrantTemplate {
            preset: self,
            identity,
            revision,
            actor,
            workflow_run,
            capability,
            resources: None,
            budget,
            valid_from: BoundaryTimeMillis::new(0),
            valid_until: BoundaryTimeMillis::new(u64::MAX),
            revocation_generation: 0,
        }
    }

    fn operations(self) -> BTreeSet<AuthorityOperation> {
        use AuthorityOperation::{
            AdministerPeer, Apply, Approve, Cancel, CreateRun, DeliverSignal, ImportBlueprint,
            Inspect, InspectAttempt, InspectCapabilityHealth, InspectDaemonHealth,
            InspectNodeExecution, InspectOwnAuthority, InspectPeer, InspectProposal,
            InspectProviderProfile, InspectRevision, InspectRun, InspectTimeline, InvokeCapability,
            ListCapabilities, NegotiateControlProtocol, Pause, Propose, ReadArtifactContent,
            ReadArtifactMetadata, ReadLayout, ReadReadiness, ReadWorkspaceValue, Resume, Retry,
            StartRun, Terminate, ValidateBlueprint, WriteLayout,
        };
        let reads = [
            Inspect,
            ValidateBlueprint,
            InspectRevision,
            InspectRun,
            InspectTimeline,
            InspectNodeExecution,
            InspectAttempt,
            InspectProposal,
            ListCapabilities,
            InspectCapabilityHealth,
            InspectProviderProfile,
            ReadArtifactMetadata,
            ReadArtifactContent,
            ReadWorkspaceValue,
            ReadLayout,
            NegotiateControlProtocol,
            ReadReadiness,
            InspectDaemonHealth,
            InspectOwnAuthority,
            InspectPeer,
        ];
        match self {
            Self::Observer => reads.into_iter().collect(),
            Self::Advisor => reads.into_iter().chain([Propose]).collect(),
            Self::Supervisor => reads
                .into_iter()
                .chain([
                    Propose,
                    Approve,
                    Apply,
                    Pause,
                    Resume,
                    Cancel,
                    Retry,
                    DeliverSignal,
                    WriteLayout,
                ])
                .collect(),
            Self::Controller => reads
                .into_iter()
                .chain([
                    Propose,
                    Approve,
                    Apply,
                    ImportBlueprint,
                    CreateRun,
                    StartRun,
                    InvokeCapability,
                    Pause,
                    Resume,
                    Cancel,
                    Retry,
                    DeliverSignal,
                    Terminate,
                    WriteLayout,
                    AdministerPeer,
                ])
                .collect(),
            Self::Autonomous => reads
                .into_iter()
                .chain([
                    Propose,
                    Approve,
                    Apply,
                    ImportBlueprint,
                    CreateRun,
                    StartRun,
                    InvokeCapability,
                    Pause,
                    Resume,
                    Cancel,
                    Retry,
                    DeliverSignal,
                    WriteLayout,
                ])
                .collect(),
        }
    }
}

/// Immutable inputs used to build one ordinary authority grant revision.
#[derive(Clone, Debug)]
pub struct GrantTemplate {
    preset: AuthorityPreset,
    identity: GrantId,
    revision: u64,
    actor: ActorRef,
    workflow_run: WorkflowRunScope,
    capability: CapabilityAuthorityScope,
    resources: Option<ResourceScope>,
    budget: AuthorityBudget,
    valid_from: BoundaryTimeMillis,
    valid_until: BoundaryTimeMillis,
    revocation_generation: u64,
}

impl GrantTemplate {
    /// Replaces the complete ordinary resource scope, including path/network/secret facts.
    #[must_use]
    pub fn resources(mut self, resources: ResourceScope) -> Self {
        self.resources = Some(resources);
        self
    }

    /// Narrows or replaces the inclusive validity interval.
    #[must_use]
    pub const fn validity(
        mut self,
        valid_from: BoundaryTimeMillis,
        valid_until: BoundaryTimeMillis,
    ) -> Self {
        self.valid_from = valid_from;
        self.valid_until = valid_until;
        self
    }

    /// Sets the exact revocation generation expected by this grant revision.
    #[must_use]
    pub const fn revocation_generation(mut self, generation: u64) -> Self {
        self.revocation_generation = generation;
        self
    }

    /// Publishes the template as a normal immutable authority grant.
    pub fn build(self) -> Result<AuthorityGrant, ControlError> {
        let resources = match self.resources {
            Some(resources) => resources,
            None => ResourceScope {
                workflow_run: self.workflow_run,
                capability: self.capability,
                filesystem: Vec::new(),
                network: NetworkScope::new(BTreeSet::new(), BTreeSet::new())?,
                secrets: BTreeSet::new(),
                artifacts: ArtifactAuthorityScope::none(),
                layouts: LayoutAuthorityScope::none(),
                peers: PeerAuthorityScope::none(),
                daemon: DaemonAuthorityScope::default(),
                workspace: WorkspaceAuthorityScope::none(),
            },
        };
        Ok(
            AuthorityGrantBuilder::new(self.identity, self.revision, self.actor)
                .operations(self.preset.operations())
                .resources(resources)
                .budget(self.budget)
                .validity(self.valid_from, self.valid_until)
                .revocation_generation(self.revocation_generation)
                .extensions(BTreeMap::new())
                .build()?,
        )
    }
}
