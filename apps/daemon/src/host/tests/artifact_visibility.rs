//! Exercise the public projection with a metadata-only grant and real stored metadata.
use super::*;
use crate::auth::ActorSession;
use crate::host::Owner;
use milkdrift_authority::{AuthorityGrant, GrantSetEvaluator, PolicyId};
use milkdrift_control::ActorAuthorityContext;
use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactStore, BeginArtifactPublication, WorkflowStore,
};
use milkdrift_runtime::CommandAuthorityClaim;
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
    ArtifactSensitivity, CausalId, CausalReference, ContentDigest, MediaType, RunId,
    WorkspaceBudget, WorkspaceUsage,
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
    workflow.task_control = milkdrift_control::TaskControlService::new(
        workflow.tasks.clone(),
        owner.authority.clone(),
        owner.clock.runtime_adapter(),
    );
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
            let read = metadata
                .iter()
                .map(|item| {
                    owner
                        .artifact_metadata(&session, item.reference().artifact().as_str())
                        .map_err(|error| error.message)
                })
                .collect::<Result<Vec<_>, _>>()?;
            assert_eq!(read.len(), 3);
            let serialized = serde_json::to_string(&read)?;
            assert!(!serialized.contains("PRIVATE_CONTENT_"));
            for (output, metadata) in read.iter().zip(&metadata) {
                assert_eq!(
                    *output,
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
            let public = owner
                .artifact_metadata(
                    &session,
                    metadata
                        .first()
                        .ok_or("public artifact missing")?
                        .reference()
                        .artifact()
                        .as_str(),
                )
                .map_err(|error| error.message)?;
            assert_eq!(public.sensitivity, "public");
            for restricted in metadata.iter().skip(1) {
                let refusal = owner
                    .artifact_metadata(&session, restricted.reference().artifact().as_str())
                    .err()
                    .ok_or("out-of-scope metadata must be refused")?;
                assert_eq!(
                    refusal.code,
                    milkdrift_control_protocol::ErrorCode::NotFound
                );
                let serialized = serde_json::to_string(&refusal)?;
                assert!(!serialized.contains(restricted.reference().artifact().as_str()));
                assert!(!serialized.contains(&restricted.reference().digest().to_hex()));
                assert!(!serialized.contains("PRIVATE_CONTENT_"));
            }
            Ok(())
        };
        check().map_err(|error| crate::host::read_model::invalid(&error.to_string()))
    })
    .await
    .map_err(|error| error.message)?;
    host.shutdown().await?;
    Ok(())
}

fn request() -> TestResult<milkdrift_control_protocol::tasks::CreateWorkflowRequest> {
    Ok(serde_json::from_value(json!({
        "request_id":"visibility-create", "title":"Receipt visibility", "description":"",
        "tasks":[{"alias":"future","definition":{
            "title":"Future work","description":"","instructions":"No entry without supplied text",
            "capability":{"id":"unconfigured","operation":"model.generate"},
            "inputs":{"brief":{"kind":"supplied","name":"brief","optional":false}},
            "dependencies":[],"required":true,"corrective_of":null,"paths":[]
        }}],"inputs":{},"finish_when_settled":false,"limits":null
    }))?)
}

#[tokio::test]
async fn retained_task_reply_rechecks_revocation_and_expiry_without_editing_receipt() -> TestResult
{
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
    let (session, before) = host
        .dispatch(false, move |owner| {
            let check = || -> TestResult<_> {
                narrow(owner, &mut session, false)?;
                let request = request()?;
                let canonical = serde_json::to_vec(&serde_json::to_value(&request)?)?;
                owner
                    .create_task_workflow(&session, request.clone(), &canonical)
                    .map_err(|error| error.message)?;
                let before = owner
                    .store
                    .workflow_receipt(session.actor.as_str(), &request.request_id)?
                    .ok_or("receipt absent")?;
                owner.authority = Arc::new(GrantSetEvaluator::new(
                    PolicyId::new("visibility-test")?,
                    1,
                    [session.grant.clone()],
                    BTreeMap::from([(session.grant.identity().clone(), 1)]),
                )?);
                let workflow = owner.workflow.as_mut().ok_or("workflow absent")?;
                workflow.task_control = milkdrift_control::TaskControlService::new(
                    workflow.tasks.clone(),
                    owner.authority.clone(),
                    owner.clock.runtime_adapter(),
                );
                assert!(
                    owner
                        .task_request_receipt(&session, &request.request_id)
                        .is_err()
                );
                assert!(
                    owner
                        .create_task_workflow(&session, request.clone(), &canonical)
                        .is_err()
                );
                assert_eq!(
                    owner
                        .store
                        .workflow_receipt(session.actor.as_str(), &request.request_id)?
                        .as_ref(),
                    Some(&before)
                );
                narrow(owner, &mut session, false)?;
                assert_eq!(
                    owner
                        .task_request_receipt(&session, &request.request_id)
                        .map_err(|error| error.message)?
                        .workflow_id,
                    before.workflow.as_str()
                );
                Ok((session, before))
            };
            check().map_err(|error| crate::host::read_model::invalid(&error.to_string()))
        })
        .await
        .map_err(|error| error.message)?;
    clock.set(102);
    host.dispatch(false, move |owner| {
        let check = || -> TestResult {
            assert!(
                owner
                    .task_request_receipt(&session, "visibility-create")
                    .is_err()
            );
            assert_eq!(
                owner
                    .store
                    .workflow_receipt(session.actor.as_str(), "visibility-create")?
                    .as_ref(),
                Some(&before)
            );
            Ok(())
        };
        check().map_err(|error| crate::host::read_model::invalid(&error.to_string()))
    })
    .await
    .map_err(|error| error.message)?;
    host.shutdown().await?;
    Ok(())
}
