//! Exercise the public projection with a metadata-only grant and real stored metadata.
use super::*;
use crate::auth::ActorSession;
use crate::host::{Owner, read_model::public_attempt};
use milkdrift_authority::{AuthorityGrant, GrantSetEvaluator, PolicyId};
use milkdrift_control::{ActorAuthorityContext, AttemptInspection};
use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactStore, AttemptId, BeginArtifactPublication, RunSequence,
};
use milkdrift_runtime::{AttemptState, CommandAuthorityClaim};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
    ArtifactSensitivity, CausalId, CausalReference, ContentDigest, MediaType, RunId, ScopeId,
    ScopeReference, ValueKey, ValueVersion, WorkspaceBudget, WorkspaceUsage,
    WorkspaceValueReference,
};
use serde_json::json;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn narrow(owner: &mut Owner, session: &mut ActorSession, metadata_only: bool) -> TestResult {
    let mut wire = serde_json::to_value(&session.grant)?;
    *wire.get_mut("valid_until").ok_or("validity absent")? = json!(101);
    if metadata_only {
        wire.get_mut("operations")
            .ok_or("operations absent")?
            .as_array_mut()
            .ok_or("operations absent")?
            .retain(|value| value != "read_artifact_content");
    }
    let grant = AuthorityGrant::from_json(&serde_json::to_vec(&wire)?)?;
    session.context = ActorAuthorityContext::new(
        session.actor.clone(),
        CommandAuthorityClaim::new(
            grant.identity().clone(),
            grant.revision(),
            grant.digest()?,
            grant.revocation_generation(),
        )?,
    );
    session.grant = grant.clone();
    owner.authority = Arc::new(GrantSetEvaluator::new(
        PolicyId::new("visibility-test")?,
        1,
        [grant],
        BTreeMap::new(),
    )?);
    let workflow = owner.workflow.as_mut().ok_or("workflow absent")?;
    workflow.control = Arc::new(milkdrift_control::ControlService::new(
        owner.store.clone(),
        owner.store.clone(),
        workflow.runtime.clone(),
        owner.authority.clone(),
    ));
    Ok(())
}

fn publish(
    owner: &Owner,
    identity: &str,
    sensitivity: ArtifactSensitivity,
) -> TestResult<ArtifactMetadata> {
    let bytes = format!("PRIVATE_CONTENT_{identity}");
    let metadata = ArtifactMetadata::new(
        ArtifactReference::new(
            ArtifactId::new(identity)?,
            ContentDigest::for_bytes(bytes.as_bytes()),
            MediaType::new("text/plain")?,
            bytes.len() as u64,
        ),
        sensitivity,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("visibility-fixture")?,
            },
            vec![],
        )?,
    )?;
    let publication = BeginArtifactPublication::new(
        ArtifactPublicationId::new(format!("publish:{identity}"))?,
        RunId::new(format!("run:{identity}"))?,
        metadata.clone(),
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    owner.store.begin_publication(&publication)?;
    owner
        .store
        .write_chunk(publication.publication(), 0, bytes.as_bytes())?;
    owner.store.commit_publication(publication.publication())?;
    Ok(metadata)
}

fn attempt(metadata: &[ArtifactMetadata]) -> TestResult<AttemptInspection> {
    let outputs = metadata.iter().map(|metadata| {
        serde_json::from_value(json!({
            "report_sequence":1,
            "value":WorkspaceValueReference::new(ScopeReference::new(RunId::new("visibility")?, ScopeId::new("root")?), ValueKey::new(metadata.reference().artifact().as_str())?, ValueVersion::new(1)?),
            "artifact":metadata.reference(), "sequence":RunSequence::new(5)
        })).map_err(Into::into)
    }).collect::<TestResult<Vec<_>>>()?;
    Ok(AttemptInspection {
        attempt: AttemptId::new("visibility-attempt")?,
        invocation: None,
        state: AttemptState::Running,
        capability: None,
        requirement: None,
        execution_authority: None,
        resolution_authorization: None,
        claim_authorization: None,
        entry_authorization: None,
        context_manifest: None,
        context_manifest_denied: false,
        side_effect: None,
        outputs,
        progress: vec![],
        usage: None,
        terminal: None,
        late_terminal_evidence: None,
        external_outcome: None,
    })
}

#[tokio::test]
async fn metadata_only_projection_preserves_actual_sensitivities_and_refuses_bytes() -> TestResult {
    let (_root, host) = queue_test_host()?;
    let mut session = host
        .auth
        .authenticate(b"queue-test-token")
        .ok_or("session absent")?;
    host.dispatch(false, move |owner| {
        let mut check = || -> TestResult {
            narrow(owner, &mut session, true)?;
            let metadata = [
                ArtifactSensitivity::Public,
                ArtifactSensitivity::Internal,
                ArtifactSensitivity::Restricted,
            ]
            .into_iter()
            .enumerate()
            .map(|(index, sensitivity)| publish(owner, &format!("visibility-{index}"), sensitivity))
            .collect::<TestResult<Vec<_>>>()?;
            let read = public_attempt(attempt(&metadata)?, owner, &session)
                .map_err(|error| error.message)?;
            assert_eq!(read.outputs.len(), 3);
            let serialized = serde_json::to_string(&read)?;
            assert!(!serialized.contains("PRIVATE_CONTENT_"));
            for (output, metadata) in read.outputs.iter().zip(&metadata) {
                assert_eq!(
                    output.artifact,
                    crate::host::read_model::public_artifact_metadata(metadata)
                );
                assert!(
                    owner
                        .artifact_range(
                            &session,
                            metadata.reference().artifact().as_str(),
                            0,
                            100,
                            "metadata-only-test"
                        )
                        .is_err()
                );
            }
            let mut wire = serde_json::to_value(&session.grant)?;
            let scope = milkdrift_authority::ArtifactAuthorityScope::new(
                milkdrift_authority::Selection::any(),
                std::collections::BTreeSet::from([ArtifactSensitivity::Public]),
            )?;
            *wire
                .pointer_mut("/resources/artifacts")
                .ok_or("artifact scope absent")? = serde_json::to_value(scope)?;
            session.grant = AuthorityGrant::from_json(&serde_json::to_vec(&wire)?)?;
            narrow(owner, &mut session, true)?;
            let public = public_attempt(attempt(&metadata)?, owner, &session)
                .map_err(|error| error.message)?;
            assert_eq!(public.outputs.len(), 1);
            assert_eq!(
                public
                    .outputs
                    .first()
                    .ok_or("public output absent")?
                    .artifact
                    .sensitivity,
                "public"
            );
            let serialized = serde_json::to_string(&public)?;
            for metadata in metadata
                .iter()
                .filter(|metadata| metadata.sensitivity() != ArtifactSensitivity::Public)
            {
                assert!(!serialized.contains(metadata.reference().artifact().as_str()));
                assert!(!serialized.contains(&metadata.reference().digest().to_hex()));
            }
            Ok(())
        };
        check().map_err(|error| crate::host::invalid(&error.to_string()))
    })
    .await
    .map_err(|error| error.message)?;
    host.shutdown().await?;
    Ok(())
}

fn request(
    identity: &str,
    command: milkdrift_control_protocol::Command,
) -> milkdrift_control_protocol::CommandRequest {
    milkdrift_control_protocol::CommandRequest {
        protocol: milkdrift_control_protocol::ProtocolVersion::CURRENT,
        command_id: identity.into(),
        expected_sequence: None,
        expected_revision: None,
        reason: "retained response disclosure regression".into(),
        evidence: vec![],
        command,
    }
}

#[tokio::test]
async fn retained_definition_reply_rechecks_revocation_and_expiry_without_editing_receipt()
-> TestResult {
    use milkdrift_control_protocol::{BlueprintDraft, Command, ErrorCode};
    use milkdrift_persistence::{ApplicationCommandStore, CommandId};
    let (root, initial) = queue_test_host()?;
    initial.shutdown().await?;
    let clock = Arc::new(ControlledDaemonClock::new(100));
    let host = DaemonHost::start_with_clock(
        owner_test_config(root.path(), &root.path().join("operator.token"), 32)?,
        clock.clone(),
    )?;
    let mut session = host
        .auth
        .authenticate(b"queue-test-token")
        .ok_or("session absent")?;
    let (session, command, before) = host
        .dispatch(false, move |owner| {
            let check = || -> TestResult<_> {
                narrow(owner, &mut session, false)?;
                let document = serde_json::from_slice(include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../crates/blueprint/tests/fixtures/revision-v3.json"
                )))?;
                let imported = crate::host::receipts::execute(
                    owner,
                    &session,
                    request("visibility-import", Command::ImportBlueprint { document }),
                )
                .map_err(|error| error.message)?;
                let revision = imported
                    .value
                    .get("revision_id")
                    .ok_or("revision absent")?
                    .as_str()
                    .ok_or("revision absent")?
                    .to_owned();
                let mut command = request(
                    "visibility-open",
                    Command::ConstructBlueprint {
                        draft: BlueprintDraft {
                            workflow_id: "golden".into(),
                            base_revision: Some(revision.clone()),
                            mutations: vec![],
                        },
                        store: false,
                    },
                );
                command.expected_revision = Some(revision);
                crate::host::receipts::execute(owner, &session, command.clone())
                    .map_err(|error| error.message)?;
                let before = owner
                    .store
                    .application_command_receipt(
                        &session.actor,
                        &CommandId::new(&command.command_id)?,
                    )?
                    .ok_or("receipt absent")?;
                owner.authority = Arc::new(GrantSetEvaluator::new(
                    PolicyId::new("visibility-test")?,
                    1,
                    [session.grant.clone()],
                    BTreeMap::from([(session.grant.identity().clone(), 1)]),
                )?);
                let workflow = owner.workflow.as_mut().ok_or("workflow absent")?;
                workflow.control = Arc::new(milkdrift_control::ControlService::new(
                    owner.store.clone(),
                    owner.store.clone(),
                    workflow.runtime.clone(),
                    owner.authority.clone(),
                ));
                let denied = crate::host::receipts::execute(owner, &session, command.clone())
                    .err()
                    .ok_or("revoked replay disclosed")?;
                assert_eq!(denied.code, ErrorCode::NotFound);
                let after = owner
                    .store
                    .application_command_receipt(
                        &session.actor,
                        &CommandId::new(&command.command_id)?,
                    )?
                    .ok_or("receipt absent")?;
                assert_eq!(before, after);
                narrow(owner, &mut session, false)?;
                assert!(
                    crate::host::receipts::execute(owner, &session, command.clone())
                        .map_err(|error| error.message)?
                        .replayed
                );
                Ok((session, command, before))
            };
            check().map_err(|error| crate::host::invalid(&error.to_string()))
        })
        .await
        .map_err(|error| error.message)?;
    clock.set(102);
    host.dispatch(false, move |owner| {
        let check = || -> TestResult {
            let denied = crate::host::receipts::execute(owner, &session, command.clone())
                .err()
                .ok_or("expired replay disclosed")?;
            assert_eq!(denied.code, ErrorCode::NotFound);
            assert_eq!(
                owner
                    .store
                    .application_command_receipt(
                        &session.actor,
                        &CommandId::new(&command.command_id)?
                    )?
                    .as_ref(),
                Some(&before)
            );
            Ok(())
        };
        check().map_err(|error| crate::host::invalid(&error.to_string()))
    })
    .await
    .map_err(|error| error.message)?;
    host.shutdown().await?;
    Ok(())
}
