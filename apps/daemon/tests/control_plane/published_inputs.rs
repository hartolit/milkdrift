//! A published brief retains the ordinary uploaded-artifact contract and model input path.
use super::{
    inputs::{ModelFixture, workflow_with_draft},
    support::*,
};
use milkdrift_blueprint::{AdaptationScope, GoverningAgreement};
use milkdrift_capability::{
    AdmissionConstraints, CapabilityCategory, DescriptorBuilder, InvocationCounts, Locality,
};
use milkdrift_control::PublicationDraft;
use milkdrift_peer_protocol::{DirectInvocationRequest, InvocationAcceptance};
use milkdrift_persistence::published::{PublishedInput, PublishedOutput};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn published_brief_uses_uploaded_text_and_declared_result() -> TestResult {
    let directory = TempDir::new()?;
    let model = ModelFixture::start().await?;
    let mut config = super::authoring::model_configuration_document(&directory, model.address)?;
    // Controlled accounts require explicit billing and token bounds, even for this loopback fixture.
    let profile_path = &config.adapters.model_profiles[0].profile;
    let mut profile: serde_json::Value = serde_json::from_slice(&fs::read(profile_path)?)?;
    profile["billing"] = json!({"type":"unbilled","source":"controlled loopback fixture"});
    profile["limits"]["max_response_bytes"] = json!(8192);
    profile["token_limits"] = json!({"type":"byte_bpe","template_tokens_per_message":0,"template_tokens_per_request":0,"maximum_input_tokens":262144,"maximum_output_tokens":512,"output_control":"max_tokens","source":"bounded encoded request including selected context metadata; controlled fixture reports 20 input and 10 output tokens"});
    fs::write(profile_path, serde_json::to_vec(&profile)?)?;
    config.actors[0].authority.resources.capability =
        CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown);
    config.runtime.controller_activation = milkdrift_daemon::ControllerActivation::Enabled;
    config.runtime.effect_threads = 1;
    config.runtime.effect_queue = 1;
    config.serving.worker_threads = 1;
    config.serving.clients.execution_limits.nested_invocations = Some(InvocationCounts::new(4, 2));
    config.runtime.publication_services.insert(
        CapabilityId::new("method:notes")?,
        milkdrift_authority::GrantId::new("grant:integration-controller")?,
    );
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    let source = workflow_with_draft(&daemon.client, "repair.draft").await?;
    let read = daemon.client.revision(&source).await?;
    let (_, base) = BlueprintRevisionDocument::from_json(&serde_json::to_vec(&read.document)?)?;
    let NodeKind::Task { config } = base
        .semantic()
        .nodes()
        .get(&NodeId::new("repair.draft")?)
        .ok_or("draft")?
        .kind()
    else {
        return Err("model task".into());
    };
    let agreement = GoverningAgreement::seal(
        NodeId::new("agreement")?,
        &base,
        AdaptationScope::new("repair.".into(), 1, vec![config.requirement().clone()])?,
        format!("b3_{}", "1".repeat(64)),
    )?;
    let governed = base.revise(
        base.id(),
        MutationBatch::new(vec![Mutation::SetAgreement {
            agreement: Some(agreement),
        }])?,
        AuthorRef::new("human:operator")?,
        "review protected checks and published notes",
    )?;
    daemon
        .client
        .submit(&request(
            "seal",
            None,
            Command::ImportBlueprint {
                document: serde_json::from_slice(
                    &BlueprintRevisionDocument::new(&governed).to_canonical_json()?,
                )?,
            },
        ))
        .await?;
    let discovery = daemon.client.execution_discovery().await?;
    let model_descriptor = &discovery
        .catalog
        .entries
        .iter()
        .find(|entry| entry.descriptor.identity().as_str() == "writing-model")
        .ok_or("model")?
        .descriptor;
    let contract = model_descriptor
        .operation(&OperationId::new("model.generate")?)
        .ok_or("operation")?
        .clone();
    let draft = PublicationDraft {
        descriptor: DescriptorBuilder::new(CapabilityId::new("method:notes")?, 1, CapabilityCategory::Tool, AdmissionConstraints::new(1, 1)?, Locality::Local)
            .operations(BTreeMap::from([(OperationId::new("method.invoke")?, contract)])).build()?,
        documentation: "Draft and review notes using the supplied brief; acceptance establishes complete prose, not editorial quality.".into(),
        revision: governed.id().clone(),
        service_grant: milkdrift_authority::GrantId::new("grant:integration-controller")?,
        inputs: BTreeMap::from([("brief".into(), PublishedInput::Artifact { media_type: "text/plain".into(), maximum_bytes: 4096 })]),
        outputs: BTreeMap::from([("notes".into(), PublishedOutput { field: milkdrift_workspace::ValueKey::new("notes")?, media_type: "text/plain".into(), maximum_bytes: 4096 })]),
        workspace_budget: WorkspaceBudget::new(128, 65536, 1048576, 128, 1048576, 16777216)?,
        allowance: milkdrift_persistence::ControllerResourceBudget::new(0, None, 524288, 2048, 4194304, 4, 2)?,
        maximum_outstanding: 1, maximum_depth: 2, maximum_duration_ms: 60000,
    };
    let prepared = daemon
        .client
        .submit(&request(
            "prepare-notes",
            None,
            Command::PrepareMethod {
                document: serde_json::to_value(draft)?,
            },
        ))
        .await?;
    daemon
        .client
        .submit(&request(
            "publish-notes",
            None,
            Command::PublishMethod {
                document: prepared.value,
                expected_previous_version: None,
            },
        ))
        .await?;
    fs::write(
        directory.path().join("brief.txt"),
        "Harbor Host 1.4 release brief",
    )?;
    cli_ok(
        &daemon,
        &directory,
        "prepare-brief",
        &[
            "invocation",
            "prepare",
            "method:notes",
            "method.invoke",
            "--host",
            "host:local",
            "--request-id",
            "notes-call",
            "--input",
            "brief=brief.txt",
            "--output",
            "call.json",
        ],
    )?;
    let call: DirectInvocationRequest =
        serde_json::from_slice(&fs::read(directory.path().join("call.json"))?)?;
    assert!(matches!(
        call.request.inputs()[0].value(),
        milkdrift_capability::InvocationValueReference::Artifact { .. }
    ));
    let mut inline = serde_json::to_value(&call)?;
    inline["request_id"] = json!("inline-refused");
    inline["request"]["invocation"] = json!("inline-refused");
    // The publication accepts an artifact brief, never inline text substituted by a client.
    inline["request"]["inputs"][0]["value"] = json!({"type":"inline","value":"Harbor Host 1.4"});
    let inline: DirectInvocationRequest = serde_json::from_value(inline)?;
    let refused = daemon.client.invoke(&inline).await?;
    // Serving admission may accept before the publication validates its method-specific contract.
    if let InvocationAcceptance::Accepted { execution, .. } = refused {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let page = daemon
                .client
                .invocation_observations(&execution, 0, 64)
                .await?;
            if let Some(terminal) = page
                .observations
                .iter()
                .find_map(|event| event.event.kind().terminal())
            {
                assert_ne!(
                    terminal.status(),
                    milkdrift_capability::TerminalStatus::Success
                );
                break;
            }
            if tokio::time::Instant::now() > deadline {
                return Err("inline refusal stalled".into());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    assert!(
        model
            .requests
            .lock()
            .map_err(|_| "fixture lock")?
            .is_empty()
    );
    let InvocationAcceptance::Accepted { execution, .. } = daemon.client.invoke(&call).await?
    else {
        return Err("call refused".into());
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let page = daemon
            .client
            .invocation_observations(&execution, 0, 64)
            .await?;
        if let Some(terminal) = page
            .observations
            .iter()
            .find_map(|event| event.event.kind().terminal())
        {
            if terminal.status() != milkdrift_capability::TerminalStatus::Success {
                let runs = daemon
                    .client
                    .runs(
                        None,
                        Some("release-notes"),
                        &PageRequest {
                            limit: 8,
                            cursor: None,
                        },
                    )
                    .await?;
                for run in runs.items {
                    let state = daemon.client.run(&run.run_id).await?;
                    for node in state.nodes {
                        if let Some(attempt) = node.latest_attempt_id {
                            eprintln!(
                                "internal attempt: {:?}",
                                daemon.client.attempt(&run.run_id, &attempt).await?
                            );
                        }
                    }
                }
            }
            assert_eq!(
                terminal.status(),
                milkdrift_capability::TerminalStatus::Success,
                "{terminal:?}"
            );
            assert_eq!(terminal.outputs().len(), 1);
            let output = daemon
                .client
                .invocation_output(&execution, terminal.outputs()[0].identity(), 0, 4096)
                .await?;
            assert_eq!(output.bytes, b"Harbor revised release notes");
            break;
        }
        if tokio::time::Instant::now() > deadline {
            return Err(format!("published model stalled: {page:?}").into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 2);
    daemon.stop().await
}
