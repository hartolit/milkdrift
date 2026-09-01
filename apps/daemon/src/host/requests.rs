//! Typed request surface for work serialized through the daemon owner.

use super::{
    ActorSession, ArtifactContentRead, ArtifactMetadataRead, AttemptRead, AuthorityOperation,
    BTreeSet, CapabilityRead, CommandAccepted, CommandRequest, Cursor, DaemonHost, LayoutDocument,
    NodeRead, Page, ProposalRead, PublicFailure, PublicRevisionSummary, RequestedResourceFacts,
    RevisionDiffRead, RevisionRead, RunRead, StreamAuthority, TimelineEntry,
};

impl DaemonHost {
    pub(crate) async fn authorize_version(
        &self,
        session: ActorSession,
    ) -> Result<(), PublicFailure> {
        self.dispatch(false, move |owner| {
            owner
                .authorize(
                    &session,
                    AuthorityOperation::NegotiateControlProtocol,
                    RequestedResourceFacts::empty(),
                    "read:version",
                )
                .map(drop)
        })
        .await
    }

    pub(crate) async fn authorize_health(
        &self,
        session: ActorSession,
    ) -> Result<(), PublicFailure> {
        self.dispatch(false, move |owner| {
            let mut resources = RequestedResourceFacts::empty();
            resources.daemon_detailed_health = true;
            owner
                .authorize(
                    &session,
                    AuthorityOperation::InspectDaemonHealth,
                    resources,
                    "read:health",
                )
                .map(drop)
        })
        .await
    }

    pub(crate) async fn authorize_readiness(
        &self,
        session: ActorSession,
    ) -> Result<(), PublicFailure> {
        self.dispatch(false, move |owner| {
            let mut resources = RequestedResourceFacts::empty();
            resources.daemon_readiness = true;
            owner
                .authorize(
                    &session,
                    AuthorityOperation::ReadReadiness,
                    resources,
                    "read:readiness",
                )
                .map(drop)
        })
        .await
    }

    pub(crate) async fn own_authority(
        &self,
        session: ActorSession,
    ) -> Result<milkdrift_control_protocol::AuthorityRead, PublicFailure> {
        self.dispatch(false, move |owner| {
            let mut resources = RequestedResourceFacts::empty();
            resources.daemon_own_authority = true;
            owner.authorize(
                &session,
                AuthorityOperation::InspectOwnAuthority,
                resources,
                "read:own-authority",
            )?;
            let operations = session
                .grant
                .operations()
                .iter()
                .filter_map(|operation| serde_json::to_value(operation).ok())
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect();
            Ok(milkdrift_control_protocol::AuthorityRead {
                actor: session.actor.as_str().to_owned(),
                grant_id: session.grant.identity().as_str().to_owned(),
                grant_revision: session.grant.revision(),
                revocation_generation: session.grant.revocation_generation(),
                operations,
            })
        })
        .await
    }

    pub(crate) async fn visible_peers(
        &self,
        session: ActorSession,
    ) -> Result<BTreeSet<String>, PublicFailure> {
        self.dispatch(false, move |owner| {
            let mut visible = BTreeSet::new();
            for peer in owner.peer_registries.keys() {
                let mut resources = RequestedResourceFacts::empty();
                resources.peer = Some(peer.clone());
                if owner
                    .evaluate_authority(
                        &session,
                        AuthorityOperation::InspectPeer,
                        resources,
                        "read:peers",
                    )?
                    .is_allowed()
                {
                    visible.insert(peer.as_str().to_owned());
                }
            }
            Ok(visible)
        })
        .await
    }

    pub(crate) async fn authorize_peer_read(
        &self,
        session: ActorSession,
        peer: String,
    ) -> Result<(), PublicFailure> {
        self.dispatch(false, move |owner| {
            owner
                .authorize_peer(
                    &session,
                    &peer,
                    AuthorityOperation::InspectPeer,
                    "read:peer",
                )
                .map(drop)
        })
        .await
    }

    pub(crate) async fn authorize_peer_administration(
        &self,
        session: ActorSession,
        peer: String,
    ) -> Result<(), PublicFailure> {
        self.dispatch(false, move |owner| {
            let decision = owner.authorize_peer(
                &session,
                &peer,
                AuthorityOperation::AdministerPeer,
                "command:administer-peer",
            )?;
            owner.record_security_decision(&decision)
        })
        .await
    }

    pub(crate) async fn authorize_stream(
        &self,
        session: ActorSession,
        stream: StreamAuthority,
    ) -> Result<String, PublicFailure> {
        self.dispatch(false, move |owner| {
            let decision = match stream {
                StreamAuthority::Run(run) => owner.authorize_run_read(
                    &session,
                    &run,
                    AuthorityOperation::InspectTimeline,
                    "stream:run",
                )?,
                StreamAuthority::Capabilities => {
                    owner.authorize(
                        &session,
                        AuthorityOperation::ListCapabilities,
                        RequestedResourceFacts::empty(),
                        "stream:capabilities",
                    )?;
                    owner.authorize(
                        &session,
                        AuthorityOperation::InspectCapabilityHealth,
                        RequestedResourceFacts::empty(),
                        "stream:capability-health",
                    )?;
                    owner.authorize(
                        &session,
                        AuthorityOperation::InspectProviderProfile,
                        RequestedResourceFacts::empty(),
                        "stream:provider-profile",
                    )?
                }
                StreamAuthority::Health => {
                    let mut resources = RequestedResourceFacts::empty();
                    resources.daemon_detailed_health = true;
                    owner.authorize(
                        &session,
                        AuthorityOperation::InspectDaemonHealth,
                        resources,
                        "stream:daemon-health",
                    )?
                }
            };
            Ok(decision.digest().to_owned())
        })
        .await
    }

    pub(crate) async fn command(
        &self,
        session: ActorSession,
        request: CommandRequest,
    ) -> Result<CommandAccepted, PublicFailure> {
        self.dispatch(false, move |owner| owner.command(&session, request))
            .await
    }

    pub(crate) async fn revision(
        &self,
        session: ActorSession,
        revision: String,
    ) -> Result<RevisionRead, PublicFailure> {
        self.dispatch(false, move |owner| owner.revision(&session, &revision))
            .await
    }

    pub(crate) async fn revisions(
        &self,
        session: ActorSession,
        workflow: Option<String>,
        cursor: Option<Cursor>,
        limit: u32,
    ) -> Result<Page<PublicRevisionSummary>, PublicFailure> {
        self.dispatch(false, move |owner| {
            owner.revisions(&session, workflow.as_deref(), cursor.as_ref(), limit)
        })
        .await
    }

    pub(crate) async fn revision_diff(
        &self,
        session: ActorSession,
        from: String,
        to: String,
    ) -> Result<RevisionDiffRead, PublicFailure> {
        self.dispatch(false, move |owner| {
            owner.revision_diff(&session, &from, &to)
        })
        .await
    }

    pub(crate) async fn run(
        &self,
        session: ActorSession,
        run: String,
    ) -> Result<RunRead, PublicFailure> {
        self.dispatch(false, move |owner| owner.run_read(&session, &run))
            .await
    }

    pub(crate) async fn node(
        &self,
        session: ActorSession,
        run: String,
        execution: String,
    ) -> Result<NodeRead, PublicFailure> {
        self.dispatch(false, move |owner| {
            owner.node_read(&session, &run, &execution)
        })
        .await
    }

    pub(crate) async fn attempt(
        &self,
        session: ActorSession,
        run: String,
        attempt: String,
    ) -> Result<AttemptRead, PublicFailure> {
        self.dispatch(false, move |owner| {
            owner.attempt_read(&session, &run, &attempt)
        })
        .await
    }

    pub(crate) async fn runs(
        &self,
        session: ActorSession,
        state: Option<String>,
        workflow: Option<String>,
        cursor: Option<Cursor>,
        limit: u32,
    ) -> Result<Page<RunRead>, PublicFailure> {
        self.dispatch(false, move |owner| {
            owner.runs(
                &session,
                state.as_deref(),
                workflow.as_deref(),
                cursor.as_ref(),
                limit,
            )
        })
        .await
    }

    pub(crate) async fn timeline(
        &self,
        session: ActorSession,
        run: String,
        cursor: Option<Cursor>,
        limit: u32,
    ) -> Result<Page<TimelineEntry>, PublicFailure> {
        self.dispatch(false, move |owner| {
            owner.timeline(&session, &run, cursor.as_ref(), limit)
        })
        .await
    }

    pub(crate) async fn proposals(
        &self,
        session: ActorSession,
        run: String,
        cursor: Option<Cursor>,
        limit: u32,
    ) -> Result<Page<ProposalRead>, PublicFailure> {
        self.dispatch(false, move |owner| {
            owner.proposals(&session, &run, cursor.as_ref(), limit)
        })
        .await
    }

    pub(crate) async fn proposal(
        &self,
        session: ActorSession,
        run: String,
        proposal: String,
        revision: String,
    ) -> Result<ProposalRead, PublicFailure> {
        self.dispatch(false, move |owner| {
            owner.proposal(&session, &run, &proposal, &revision)
        })
        .await
    }

    pub(crate) async fn capabilities(
        &self,
        session: ActorSession,
    ) -> Result<Vec<CapabilityRead>, PublicFailure> {
        self.dispatch(false, move |owner| owner.capabilities(&session))
            .await
    }

    pub(crate) async fn artifact_metadata(
        &self,
        session: ActorSession,
        artifact: String,
    ) -> Result<ArtifactMetadataRead, PublicFailure> {
        self.dispatch(false, move |owner| {
            owner.artifact_metadata(&session, &artifact)
        })
        .await
    }

    pub(crate) async fn artifact_range(
        &self,
        session: ActorSession,
        artifact: String,
        offset: u64,
        maximum: u32,
        evidence: String,
    ) -> Result<ArtifactContentRead, PublicFailure> {
        self.dispatch(false, move |owner| {
            owner.artifact_range(&session, &artifact, offset, maximum, &evidence)
        })
        .await
    }

    pub(crate) async fn layout(
        &self,
        session: ActorSession,
        workflow: String,
        revision: String,
    ) -> Result<LayoutDocument, PublicFailure> {
        self.dispatch(false, move |owner| {
            owner.layout(&session, &workflow, &revision)
        })
        .await
    }
}
