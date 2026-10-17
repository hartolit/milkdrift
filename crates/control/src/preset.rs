use std::collections::{BTreeMap, BTreeSet};

use milkdrift_authority::{
    ActorRef, ArtifactAuthorityScope, AuthorityBudget, AuthorityGrant, AuthorityGrantBuilder,
    AuthorityOperation, BoundaryTimeMillis, CapabilityAuthorityScope, DaemonAuthorityScope,
    GrantId, NetworkScope, PeerAuthorityScope, ResourceScope, WorkflowRunScope,
    WorkspaceAuthorityScope,
};
use serde::{Deserialize, Serialize};

use crate::ControlError;

/// Convenient role names that expand into ordinary immutable grants.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityPreset {
    /// Invoke and observe public capabilities within explicit scope, without workflow administration.
    Invoker,
    /// Inspect only the exact state permitted by resource scope.
    Observer,
    /// Inspect future definitions; task edits require a stronger scoped role.
    Advisor,
    /// Edit future task definitions and request stop within exact scope.
    Supervisor,
    /// Create and execute tasks and administer permitted hosts and daemon names within explicit scope.
    Controller,
    /// Create and execute tasks under strict caller-supplied ceilings.
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
            AdministerCapabilities, AdministerPeer, Apply, Cancel, CancelCapability, CreateRun,
            Inspect, InspectCapabilityHealth, InspectDaemonHealth, InspectOwnAuthority,
            InspectPeer, InspectProviderProfile, InspectRun, InvokeCapability, ListCapabilities,
            NegotiateControlProtocol, PublishArtifact, ReadArtifactContent, ReadArtifactMetadata,
            ReadCapabilityOutput, ReadReadiness, RenameDaemon, StartRun,
        };
        let reads = [
            Inspect,
            InspectRun,
            ListCapabilities,
            InspectCapabilityHealth,
            InspectProviderProfile,
            ReadArtifactMetadata,
            ReadArtifactContent,
            ReadCapabilityOutput,
            NegotiateControlProtocol,
            ReadReadiness,
            InspectDaemonHealth,
            InspectOwnAuthority,
            InspectPeer,
        ];
        match self {
            Self::Invoker => [
                Inspect,
                ListCapabilities,
                InspectCapabilityHealth,
                InspectProviderProfile,
                ReadCapabilityOutput,
                ReadArtifactMetadata,
                ReadArtifactContent,
                PublishArtifact,
                InvokeCapability,
                CancelCapability,
                NegotiateControlProtocol,
                ReadReadiness,
                InspectOwnAuthority,
            ]
            .into_iter()
            .collect(),
            Self::Observer | Self::Advisor => reads.into_iter().collect(),
            Self::Supervisor => reads.into_iter().chain([Apply, Cancel]).collect(),
            Self::Controller => reads
                .into_iter()
                .chain([
                    CreateRun,
                    StartRun,
                    Apply,
                    Cancel,
                    InvokeCapability,
                    CancelCapability,
                    PublishArtifact,
                    AdministerPeer,
                    AdministerCapabilities,
                    RenameDaemon,
                ])
                .collect(),
            Self::Autonomous => reads
                .into_iter()
                .chain([
                    CreateRun,
                    StartRun,
                    Apply,
                    Cancel,
                    InvokeCapability,
                    CancelCapability,
                    PublishArtifact,
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
    ///
    /// # Errors
    /// Returns authority validation errors for the chosen grant revision, operations, resource scopes,
    /// budget, validity interval, or revocation facts. The template itself grants no authority.
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
