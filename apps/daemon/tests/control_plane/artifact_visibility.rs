//! Artifact authority must survive composition into every ordinary run view.
use super::{inputs, results, support::*};
use milkdrift_authority::Selection;
use milkdrift_control_protocol::{ArtifactMetadataRead, RunRead};
use serde_json::Value;
use std::collections::BTreeSet;

fn assert_hidden(value: &Value, hidden: &[ArtifactMetadataRead]) -> TestResult {
    let bytes = serde_json::to_string(value)?;
    for artifact in hidden {
        assert!(
            !bytes.contains(&artifact.artifact_id),
            "artifact identity leaked: {bytes}"
        );
        assert!(
            !bytes.contains(&artifact.digest),
            "artifact digest leaked: {bytes}"
        );
    }
    Ok(())
}

async fn views(client: &ControlClient, original: &RunRead) -> TestResult<Vec<Value>> {
    let run = &original.run_id;
    let mut values = vec![
        serde_json::to_value(client.run(run).await?)?,
        serde_json::to_value(client.run_result(run).await?)?,
        serde_json::to_value(
            client
                .timeline(
                    run,
                    &PageRequest {
                        cursor: None,
                        limit: 100,
                    },
                )
                .await?,
        )?,
    ];
    for node in &original.nodes {
        values.push(serde_json::to_value(
            client.node(run, &node.execution_id).await?,
        )?);
        if let Some(attempt) = &node.latest_attempt_id {
            values.push(serde_json::to_value(client.attempt(run, attempt).await?)?);
        }
    }
    let mut stream = client.subscribe("v1/runs/artifact-visibility/stream", None);
    let observation = tokio::time::timeout(Duration::from_secs(30), stream.next())
        .await?
        .ok_or("stream closed")??;
    assert!(matches!(observation.observation, Observation::RunStatus(_)));
    values.push(serde_json::to_value(observation)?);
    Ok(values)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_node_attempt_result_and_stream_filter_artifact_metadata() -> TestResult {
    let directory = tempfile::tempdir()?;
    let model = results::Responses::start(vec![
        results::prose("PRIVATE_DRAFT_SENTINEL_73819", "stop"),
        results::prose("PRIVATE_FINAL_SENTINEL_48370", "stop"),
    ])
    .await?;
    let mut config = super::authoring::model_configuration_document(&directory, model.address)?;
    let daemon = start(config.clone().validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let revision = inputs::workflow(&daemon.client).await?;
    let input = inputs::upload(&daemon.client, "visibility", b"visibility brief").await?;
    let input_metadata = daemon.client.artifact_metadata(&input.artifact_id).await?;
    let started = inputs::start_request("artifact-visibility", &revision, vec![input]);
    daemon.client.submit(&started).await?;
    let original = wait_for_run(
        &daemon.client,
        "artifact-visibility",
        Duration::from_secs(45),
        |run| run.terminal.is_some(),
    )
    .await?;
    assert_eq!(original.terminal.as_deref(), Some("succeeded"));
    let mut artifacts = BTreeMap::new();
    for node in &original.nodes {
        if let Some(attempt) = &node.latest_attempt_id {
            let read = daemon.client.attempt(&original.run_id, attempt).await?;
            for metadata in read
                .outputs
                .into_iter()
                .map(|output| output.artifact)
                .chain(read.context_manifest)
            {
                artifacts.insert(metadata.artifact_id.clone(), metadata);
            }
        }
    }
    assert!(artifacts.len() >= 4);
    let allowed = artifacts
        .values()
        .find(|artifact| artifact.content_type.starts_with("text/"))
        .ok_or("text output absent")?
        .clone();
    artifacts.insert(input_metadata.artifact_id.clone(), input_metadata);
    let manifests = artifacts
        .values()
        .filter(|artifact| artifact.content_type.contains("context-manifest"))
        .map(|artifact| ArtifactId::new(&artifact.artifact_id))
        .collect::<Result<BTreeSet<_>, _>>()?;
    assert!(!manifests.is_empty());
    daemon.stop().await?;
    control_inspection(
        &directory,
        &original,
        &artifacts.values().cloned().collect::<Vec<_>>(),
    )?;

    for (name, scope) in [
        ("none", ArtifactAuthorityScope::none()),
        (
            "manifests",
            ArtifactAuthorityScope::new(
                Selection::only(manifests)?,
                BTreeSet::from([ArtifactSensitivity::Restricted]),
            )?,
        ),
        (
            "one",
            ArtifactAuthorityScope::new(
                Selection::only(BTreeSet::from([ArtifactId::new(&allowed.artifact_id)?]))?,
                BTreeSet::from([ArtifactSensitivity::Restricted]),
            )?,
        ),
        (
            "public",
            ArtifactAuthorityScope::new(
                Selection::any(),
                BTreeSet::from([ArtifactSensitivity::Public]),
            )?,
        ),
    ] {
        let credential = format!("credential:visibility-{name}");
        let path = directory.path().join(format!("visibility-{name}.token"));
        fs::write(&path, format!("visibility-{name}-fixture"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        config
            .secret_sources
            .insert(credential.clone(), SecretSourceConfig::File { path });
        let mut authority = ActorGrantConfig::dangerous_administrator();
        authority.resources.artifacts = scope;
        config.actors.push(ActorBindingConfig {
            credential_ref: credential,
            actor: format!("reader:{name}"),
            grant_id: format!("grant:reader-{name}"),
            grant_revision: 1,
            revocation_generation: 0,
            preset: AuthorityPresetConfig::Observer,
            authority,
            enabled: true,
        });
    }
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    for name in ["none", "one", "public", "manifests"] {
        let reader = client(&daemon.endpoint, &format!("visibility-{name}-fixture"))?;
        let hidden = artifacts
            .values()
            .filter(|artifact| match name {
                "one" => artifact.artifact_id != allowed.artifact_id,
                "manifests" => !artifact.content_type.contains("context-manifest"),
                _ => true,
            })
            .cloned()
            .collect::<Vec<_>>();
        let responses = views(&reader, &original).await?;
        for response in &responses {
            assert_hidden(response, &hidden)?;
            let text = serde_json::to_string(response)?;
            assert!(!text.contains("PRIVATE_DRAFT_SENTINEL_73819"));
            if name != "one" {
                assert!(!text.contains("PRIVATE_FINAL_SENTINEL_48370"));
            }
        }
        for artifact in &hidden {
            assert!(
                reader
                    .artifact_metadata(&artifact.artifact_id)
                    .await
                    .is_err()
            );
        }
        if name == "one" {
            assert_eq!(
                reader.artifact_metadata(&allowed.artifact_id).await?,
                allowed
            );
            assert!(responses.iter().any(|value| {
                serde_json::to_string(value).is_ok_and(|value| value.contains(&allowed.artifact_id))
            }));
        }
    }
    assert!(daemon.client.submit(&started).await?.replayed);
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 2);
    daemon.stop().await
}

fn control_inspection(
    directory: &tempfile::TempDir,
    original: &RunRead,
    artifacts: &[ArtifactMetadataRead],
) -> TestResult {
    use milkdrift_authority::{
        AuthorityGrantBuilder, AuthorityOperation, GrantSetEvaluator, PolicyId,
    };
    use milkdrift_control::{
        ActorAuthorityContext, ControlCommand, ControlCommandDocument, ControlError, ControlId,
        ControlResult, ControlService, OptimisticGuard,
    };
    use milkdrift_persistence::{PageSize, Reason, TimestampMillis, WorkerId};
    use milkdrift_runtime::{
        CommandAuthorityClaim, DeterministicExecutor, ManualClock, RetryPolicy, RuntimeConfig,
        RuntimeService, SchedulerLimits, SequentialIdGenerator,
    };
    let store = Arc::new(RedbStore::open(directory.path().join("data"))?);
    let descriptor =
        milkdrift_capability::CapabilityDescriptorDocument::from_json(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/capability/tests/fixtures/descriptor-v1.json"
        )))?
        .body()
        .clone();
    for allow_metadata in [false, true] {
        let actor = milkdrift_authority::ActorRef::new("reader:control-visibility")?;
        let mut resources = ActorGrantConfig::dangerous_administrator().resources;
        if !allow_metadata {
            resources.artifacts = ArtifactAuthorityScope::none();
        }
        let grant = AuthorityGrantBuilder::new(
            milkdrift_authority::GrantId::new("grant:control-visibility")?,
            1,
            actor.clone(),
        )
        .operations(BTreeSet::from([
            AuthorityOperation::InspectRun,
            AuthorityOperation::InspectTimeline,
            AuthorityOperation::ReadArtifactMetadata,
        ]))
        .resources(resources)
        .build()?;
        let context = ActorAuthorityContext::new(
            actor.clone(),
            CommandAuthorityClaim::new(grant.identity().clone(), 1, grant.digest()?, 0)?,
        );
        let authority = Arc::new(GrantSetEvaluator::new(
            PolicyId::new("control-visibility")?,
            1,
            [grant],
            BTreeMap::new(),
        )?);
        let runtime = Arc::new(RuntimeService::open_closed_with_authority(
            store.clone(),
            Arc::new(DeterministicExecutor::new(descriptor.clone())),
            authority.clone(),
            Arc::new(ManualClock::new(1)),
            Arc::new(SequentialIdGenerator::new("visibility-reader", 1)?),
            RuntimeConfig::new(
                WorkerId::new("visibility-reader")?,
                actor,
                30_000,
                32,
                SchedulerLimits::new(8, 4, 2, 4)?,
                RetryPolicy::new(1, vec![], 1, 1, 0)?,
            )?,
        )?);
        let service = ControlService::new(store.clone(), store.clone(), runtime, authority);
        let command = |command| {
            ControlCommandDocument::new(
                ControlId::new("visibility-read")?,
                context.clone(),
                TimestampMillis::new(1),
                OptimisticGuard::default(),
                Reason::new("read authorized metadata")?,
                vec![],
                command,
            )
        };
        let run = RunId::new(&original.run_id)?;
        let read = service.execute(&command(ControlCommand::InspectRun { run: run.clone() })?)?;
        let value = serde_json::to_value(&read)?;
        if !allow_metadata {
            assert_hidden(&value, artifacts)?;
        }
        let mut after = None;
        let mut denied = false;
        let mut saw_artifacts = false;
        loop {
            match service.execute(&command(ControlCommand::InspectTimeline {
                run: run.clone(),
                after,
                limit: PageSize::new(100)?,
            })?) {
                Err(ControlError::AuthorizationDenied { .. }) => {
                    denied = true;
                    break;
                }
                Ok(ControlResult::Timeline { value }) => {
                    let serialized = serde_json::to_string(&value)?;
                    assert!(!serialized.contains("PRIVATE_DRAFT_SENTINEL_73819"));
                    saw_artifacts |= artifacts
                        .iter()
                        .any(|artifact| serialized.contains(&artifact.artifact_id));
                    if !allow_metadata {
                        assert_hidden(&serde_json::to_value(&value)?, artifacts)?;
                    }
                    after = value.next_sequence;
                    if after.is_none() {
                        break;
                    }
                }
                other => return Err(format!("unexpected history result: {other:?}").into()),
            }
        }
        assert_eq!(denied, !allow_metadata);
        assert_eq!(saw_artifacts, allow_metadata);
    }
    Ok(())
}
