//! Authorized run, node, attempt, context, and historical-provenance read ownership.

use super::*;

impl Owner {
    pub(super) fn run_read(
        &self,
        session: &ActorSession,
        run: &str,
    ) -> Result<RunRead, PublicFailure> {
        let run_id = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        self.authorize_run_read(session, run, AuthorityOperation::InspectRun, "read:run")?;
        let result = match self.inspect_control(
            session,
            ControlCommand::InspectRun { run: run_id },
            None,
            "run",
        ) {
            Err(error) if error.code == ErrorCode::Unauthorized => return Err(not_found()),
            result => result?,
        };
        let ControlResult::RunInspection { value } = result else {
            return Err(internal());
        };
        Ok(public_run(value))
    }

    pub(super) fn node_read(
        &self,
        session: &ActorSession,
        run: &str,
        execution: &str,
    ) -> Result<NodeRead, PublicFailure> {
        self.authorize_run_read(
            session,
            run,
            AuthorityOperation::InspectNodeExecution,
            "read:node-execution",
        )?;
        self.run_read(session, run)?
            .nodes
            .into_iter()
            .find(|node| node.execution_id == execution)
            .ok_or_else(not_found)
    }

    pub(super) fn attempt_read(
        &mut self,
        session: &ActorSession,
        run: &str,
        attempt: &str,
    ) -> Result<AttemptRead, PublicFailure> {
        self.authorize_run_read(
            session,
            run,
            AuthorityOperation::InspectAttempt,
            "read:attempt",
        )?;
        let current = self
            .run_read(session, run)?
            .nodes
            .into_iter()
            .find(|node| {
                node.latest_attempt
                    .as_ref()
                    .is_some_and(|value| value.attempt_id == attempt)
            })
            .and_then(|node| {
                node.latest_attempt
                    .map(|attempt| (node.node_id, node.revision_id, attempt))
            });
        let (node_id, revision_id, mut value) = match current {
            Some((node, revision, mut value)) => {
                if let Ok((_, _, historical)) = self.historical_attempt_read(run, attempt) {
                    value.peer_id = historical.peer_id;
                }
                (node, revision, value)
            }
            None => self.historical_attempt_read(run, attempt)?,
        };
        let Some(reference) = value.context_manifest.as_ref() else {
            return Ok(value);
        };
        let artifact = ArtifactId::new(reference.artifact_id.clone())
            .map_err(|error| invalid(&error.to_string()))?;
        artifacts::preauthorize_artifact_identity(
            session,
            &artifact,
            AuthorityOperation::ReadArtifactContent,
        )?;
        let metadata = self
            .store
            .metadata(&artifact)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let mut resources = RequestedResourceFacts::empty();
        resources.artifact = Some(artifact);
        resources.artifact_sensitivity = Some(metadata.sensitivity());
        let decision = self.evaluate_authority(
            session,
            AuthorityOperation::ReadArtifactContent,
            resources,
            "read:attempt-context-manifest",
        )?;
        self.record_security_decision(&decision)?;
        if !decision.is_allowed() {
            value.context_access = "denied".to_owned();
            return Ok(value);
        }
        let reference = milkdrift_capability::ArtifactReference::new(
            reference.artifact_id.clone(),
            reference.digest.clone(),
            Some(reference.content_type.clone()),
            Some(reference.size),
        )
        .map_err(|error| invalid(&error.to_string()))?;
        let manifest = milkdrift_runtime::read_context_manifest(
            self.store.as_ref(),
            &reference,
            ArtifactReadAuthority::Authorized {
                actor: session.actor.clone(),
                evidence: EvidenceId::new(format!("attempt-context:{}", &decision.digest()[..32]))
                    .map_err(public_persistence)?,
            },
        )
        .map_err(|error| PublicFailure::new(ErrorCode::Corruption, error.to_string(), false))?;
        if manifest.attempt().as_str() != attempt {
            return Err(PublicFailure::new(
                ErrorCode::Corruption,
                "context manifest is bound to another attempt",
                false,
            ));
        }
        let revision = parse_revision_id(&revision_id)?;
        let stored = self
            .store
            .revision(&revision)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        let policy = stored
            .semantic()
            .nodes()
            .get(
                &milkdrift_blueprint::NodeId::new(node_id)
                    .map_err(|error| invalid(&error.to_string()))?,
            )
            .and_then(|node| match node.kind() {
                milkdrift_blueprint::NodeKind::Task { config } => Some(config.context_policy()),
                _ => None,
            })
            .ok_or_else(not_found)?;
        const MAX_CONTEXT_READ_ITEMS: usize = 256;
        let truncated = manifest.entries().len() > MAX_CONTEXT_READ_ITEMS
            || manifest.omissions().len() > MAX_CONTEXT_READ_ITEMS;
        value.context = Some(ContextManifestRead {
            schema_version: manifest.schema_version(),
            digest: manifest.digest().as_str().to_owned(),
            policy: serde_json::to_value(policy).map_err(|_| internal())?,
            entries: manifest
                .entries()
                .iter()
                .take(MAX_CONTEXT_READ_ITEMS)
                .map(serde_json::to_value)
                .collect::<Result<_, _>>()
                .map_err(|_| internal())?,
            omissions: manifest
                .omissions()
                .iter()
                .take(MAX_CONTEXT_READ_ITEMS)
                .map(serde_json::to_value)
                .collect::<Result<_, _>>()
                .map_err(|_| internal())?,
            totals: serde_json::to_value(manifest.totals()).map_err(|_| internal())?,
            budget: serde_json::to_value(manifest.budget()).map_err(|_| internal())?,
            truncated,
        });
        value.context_access = "authorized".to_owned();
        Ok(value)
    }

    fn historical_attempt_read(
        &self,
        run: &str,
        attempt: &str,
    ) -> Result<(String, String, AttemptRead), PublicFailure> {
        let run_id = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let attempt_id =
            AttemptId::new(attempt.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let page_size = PageSize::new(256).map_err(public_persistence)?;
        let mut cursor = None;
        let mut current_revision = None::<RevisionId>;
        let mut execution_authority = None;
        let mut executions = BTreeMap::new();
        let mut retry_timer = None;
        let mut located = None::<(String, String, AttemptRead)>;
        loop {
            let query = EventPageQuery::new(run_id.clone(), cursor, page_size)
                .map_err(public_persistence)?;
            let page = self.store.events(&query).map_err(public_persistence)?;
            for event in page.events {
                let event_sequence = event.sequence().get();
                match event.kind() {
                    RunEventKind::ExecutionAuthorityEstablished { basis } => {
                        execution_authority = Some(public_execution_authority(basis));
                    }
                    RunEventKind::RunCreated { revision, .. }
                    | RunEventKind::RevisionPinned { revision, .. } => {
                        current_revision = Some(revision.clone());
                    }
                    RunEventKind::NodeBecameEligible {
                        node, execution, ..
                    } => {
                        if let Some(revision) = current_revision.as_ref() {
                            executions.insert(
                                execution.clone(),
                                (node.as_str().to_owned(), revision.as_str().to_owned()),
                            );
                        }
                    }
                    RunEventKind::NodeRetryScheduled {
                        execution,
                        next_attempt,
                        timer,
                        ..
                    } if next_attempt == &attempt_id => {
                        let (node, revision) = executions
                            .get(execution)
                            .cloned()
                            .ok_or_else(|| corruption("retry attempt has no owning execution"))?;
                        retry_timer = Some(timer.clone());
                        located = Some((
                            node,
                            revision,
                            empty_attempt_read(attempt, "awaiting_retry_timer"),
                        ));
                    }
                    RunEventKind::TimerFired { timer, .. }
                        if retry_timer.as_ref() == Some(timer) =>
                    {
                        if let Some((_, _, value)) = located.as_mut() {
                            value.state = "ready_to_schedule".to_owned();
                        }
                    }
                    RunEventKind::NodeScheduled {
                        node,
                        execution,
                        attempt: scheduled,
                        invocation,
                        request,
                        ..
                    } if scheduled == &attempt_id => {
                        let revision = executions
                            .get(execution)
                            .map(|(_, revision)| revision.clone())
                            .or_else(|| {
                                current_revision
                                    .as_ref()
                                    .map(|revision| revision.as_str().to_owned())
                            })
                            .ok_or_else(|| corruption("scheduled attempt has no revision"))?;
                        let mut value = empty_attempt_read(attempt, "scheduled");
                        value.execution_authority = execution_authority.clone();
                        value.invocation_id = Some(invocation.as_str().to_owned());
                        value.context_manifest =
                            request.context_manifest().map(public_invocation_artifact);
                        value.context_access = if value.context_manifest.is_some() {
                            "metadata_only".to_owned()
                        } else {
                            "absent".to_owned()
                        };
                        located = Some((node.as_str().to_owned(), revision, value));
                    }
                    RunEventKind::CapabilityResolutionDecisionRecorded {
                        attempt: resolved,
                        authorization,
                        ..
                    } if resolved == &attempt_id => {
                        if let Some((_, _, value)) = located.as_mut() {
                            value.resolution_authorization =
                                Some(public_authority_decision(authorization));
                            value.peer_id = authorization
                                .request()
                                .resources
                                .peer
                                .as_ref()
                                .map(|peer| peer.as_str().to_owned())
                                .or_else(|| {
                                    authorization
                                        .request()
                                        .provenance
                                        .peer
                                        .as_ref()
                                        .map(|peer| peer.as_str().to_owned())
                                });
                        }
                    }
                    RunEventKind::CapabilityEntryDecisionRecorded {
                        attempt: entered,
                        authorization,
                    } if entered == &attempt_id => {
                        if let Some((_, _, value)) = located.as_mut() {
                            value.claim_authorization =
                                Some(public_authority_decision(authorization));
                        }
                    }
                    RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                        attempt: entered,
                        authorization,
                    } if entered == &attempt_id => {
                        if let Some((_, _, value)) = located.as_mut() {
                            value.entry_authorization =
                                Some(public_authority_decision(authorization));
                        }
                    }
                    RunEventKind::CapabilityResolved {
                        attempt: resolved,
                        snapshot,
                        ..
                    } if resolved == &attempt_id => {
                        if let Some((_, _, value)) = located.as_mut() {
                            value.capability_id = Some(snapshot.capability().as_str().to_owned());
                            value.descriptor_revision = Some(snapshot.descriptor_revision());
                            value.capability_provenance =
                                Some(public_capability_provenance(snapshot));
                            value.provider_profile = snapshot
                                .provider_profile()
                                .map(|profile| profile.as_str().to_owned());
                        }
                    }
                    RunEventKind::LeaseGranted {
                        attempt: leased, ..
                    } if leased == &attempt_id => {
                        if let Some((_, _, value)) = located.as_mut() {
                            value.state = "leased".to_owned();
                        }
                    }
                    RunEventKind::NodeStarted {
                        attempt: started, ..
                    } if started == &attempt_id => {
                        if let Some((_, _, value)) = located.as_mut() {
                            value.state = "running".to_owned();
                        }
                    }
                    RunEventKind::NodeProgressRecorded {
                        attempt: progressed,
                        detail,
                        ..
                    } if progressed == &attempt_id => {
                        if let Some((_, _, value)) = located.as_mut() {
                            value.progress_observations =
                                value.progress_observations.saturating_add(1);
                            value.progress_bytes = value.progress_bytes.saturating_add(
                                u64::try_from(detail.as_str().len()).unwrap_or(u64::MAX),
                            );
                        }
                    }
                    RunEventKind::AttemptUsageRecorded {
                        attempt: measured,
                        usage,
                    } if measured == &attempt_id => {
                        if let Some((_, _, value)) = located.as_mut() {
                            value.usage = Some(public_attempt_usage(usage));
                        }
                    }
                    RunEventKind::NodeOutputPublished {
                        attempt: published,
                        report_sequence,
                        value: reference,
                        artifact: Some(artifact),
                        ..
                    } if published == &attempt_id => {
                        if let Some((_, _, value)) = located.as_mut() {
                            value
                                .outputs
                                .push(milkdrift_control_protocol::AttemptOutputRead {
                                    name: reference.key().as_str().to_owned(),
                                    report_sequence: Some(*report_sequence),
                                    publication_sequence: event_sequence,
                                    artifact: ArtifactMetadataRead {
                                        artifact_id: artifact.artifact().as_str().to_owned(),
                                        digest: artifact.digest().to_hex(),
                                        size: artifact.size_bytes(),
                                        content_type: artifact.media_type().as_str().to_owned(),
                                        disposition_name: None,
                                        sensitivity: "restricted".to_owned(),
                                    },
                                });
                        }
                    }
                    RunEventKind::NodeTerminal {
                        attempt: terminal,
                        outcome,
                        ..
                    } if terminal == &attempt_id => {
                        if let Some((_, _, value)) = located.as_mut() {
                            value.state = "terminal".to_owned();
                            value.terminal = Some(snake_debug(outcome));
                        }
                    }
                    RunEventKind::ExternalOutcomeUncertain {
                        attempt: uncertain, ..
                    } if uncertain == &attempt_id => {
                        if let Some((_, _, value)) = located.as_mut() {
                            value.state = "uncertain".to_owned();
                            value.uncertain = true;
                        }
                    }
                    RunEventKind::ExternalOutcomeRetained {
                        attempt: retained, ..
                    } if retained == &attempt_id => {
                        if let Some((_, _, value)) = located.as_mut() {
                            value.state = "retained".to_owned();
                            value.uncertain = true;
                        }
                    }
                    _ => {}
                }
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        located.ok_or_else(not_found)
    }

    pub(super) fn authorize_run_read(
        &self,
        session: &ActorSession,
        run: &str,
        operation: AuthorityOperation,
        boundary: &str,
    ) -> Result<AuthorityDecisionSnapshot, PublicFailure> {
        let run = RunId::new(run.to_owned()).map_err(|error| invalid(&error.to_string()))?;
        let mut resources = RequestedResourceFacts::empty();
        resources.run = Some(run.clone());
        match &session.grant.resources().workflow_run {
            WorkflowRunScope::Any => {}
            WorkflowRunScope::Workflow { workflow } => {
                resources.workflow = Some(workflow.clone());
            }
            WorkflowRunScope::Run {
                run: allowed,
                workflow,
            } => {
                if allowed != &run {
                    return Err(unauthorized());
                }
                resources.workflow = workflow.clone();
            }
        }
        let decision = self.authorize(session, operation, resources.clone(), boundary)?;
        let summary = self
            .store
            .run_summary(&run)
            .map_err(public_persistence)?
            .ok_or_else(not_found)?;
        if resources
            .workflow
            .as_ref()
            .is_some_and(|workflow| workflow != &summary.workflow)
        {
            return Err(not_found());
        }
        Ok(decision)
    }
}
