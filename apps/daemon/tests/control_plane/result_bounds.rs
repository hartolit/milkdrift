//! Real admitted attempts can each fit their reader while their combined context does not.
use super::{
    results::{Responses, prose},
    support::*,
};
use milkdrift_blueprint::{
    BindingSource, DataPort, FieldId, InterfaceField, PathSelector, SchemaRef, WorkflowInterface,
};
use milkdrift_capability::{BoundedJson, SchemaId};
use milkdrift_control_protocol::{
    MAX_DOCUMENT_BYTES, MAX_REQUEST_ID_BYTES, ResponseEnvelope, RunResultRead,
};
use milkdrift_persistence::RevisionStore;
use serde_json::json;

fn definition() -> TestResult<BlueprintRevision> {
    let (_, legacy) = BlueprintRevisionDocument::from_json(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/authoring/legacy-workflow.json"
    )))?;
    let template = legacy
        .semantic()
        .nodes()
        .get(&NodeId::new("draft")?)
        .ok_or("model task")?;
    let mut mutations = vec![];
    let artifact = SchemaRef::new(SchemaId::new("milkdrift.artifact-reference")?, 1)?;
    for index in 0..6 {
        let mut wire = serde_json::to_value(template)?;
        *wire.get_mut("id").ok_or("id")? = json!(format!("step-{index}"));
        wire.get_mut("data_inputs")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("inputs")?
            .retain(|name, _| name == "milkdrift.model_task");
        let mut node: Node = serde_json::from_value(wire)?;
        if index > 0 {
            node = node.with_control_input(PortId::new("in")?)?;
        }
        for input in 0..7 {
            node = node.with_data_input(
                PortId::new(format!("evidence-{input}"))?,
                DataPort::input(
                    SchemaRef::new(SchemaId::new("fixture.text")?, 1)?,
                    true,
                    Some(BindingSource::Literal {
                        value: BoundedJson::new(json!("x".repeat(32_768)))?,
                    }),
                )?,
            )?;
        }
        mutations.push(Mutation::AddNode { node });
        mutations.push(Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new(format!("next-{index}"))?,
                EdgeKind::Control,
                NodeId::new(format!("step-{index}"))?,
                PortId::new("out")?,
                NodeId::new(if index == 5 {
                    "done".into()
                } else {
                    format!("step-{}", index + 1)
                })?,
                PortId::new("in")?,
            ),
        });
    }
    let done = Node::new(
        NodeId::new("done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_data_input(
        PortId::new("result")?,
        DataPort::input(
            artifact.clone(),
            true,
            Some(BindingSource::NodeOutput {
                node: NodeId::new("step-5")?,
                port: PortId::new("final_text")?,
                path: PathSelector::new(vec![])?,
            }),
        )?,
    )?;
    mutations.push(Mutation::AddNode { node: done });
    mutations.push(Mutation::AddEdge {
        edge: Edge::new(
            EdgeId::new("result")?,
            EdgeKind::Data,
            NodeId::new("step-5")?,
            PortId::new("final_text")?,
            NodeId::new("done")?,
            PortId::new("result")?,
        ),
    });
    mutations.push(Mutation::SetInterface {
        interface: WorkflowInterface::new(
            [],
            [(FieldId::new("result")?, InterfaceField::required(artifact))],
        )?,
    });
    let revision = BlueprintRevision::genesis(
        WorkflowId::new("aggregate-bound")?,
        MutationBatch::new(mutations)?,
        AuthorRef::new("human:fixture")?,
        "Six separately bounded context manifests",
    )?;
    Ok(BlueprintRevisionDocument::from_json(
        &BlueprintRevisionDocument::new(&revision).to_canonical_json()?,
    )?
    .1)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn combined_result_omits_large_detail_but_keeps_exact_navigation_and_outputs() -> TestResult {
    let directory = tempfile::tempdir()?;
    let model = Responses::start(vec![prose(&"\"\\".repeat(4096), "stop"); 6]).await?;
    let config = super::authoring::model_configuration_document(&directory, model.address)?;
    let profile_path = &config
        .adapters
        .model_profiles
        .first()
        .ok_or("model profile")?
        .profile;
    let mut profile: serde_json::Value = serde_json::from_slice(&fs::read(profile_path)?)?;
    // This fixture admits the selected context to the loopback provider. Production
    // endpoint limits stay operator configured and are unchanged by the correction.
    *profile
        .pointer_mut("/limits/max_request_bytes")
        .ok_or("request bound")? = json!(1_048_576);
    fs::write(profile_path, serde_json::to_vec(&profile)?)?;
    let revision = definition()?;
    // Seed an ordinary legal immutable revision through its storage owner. Its full
    // definition exceeds one control request; this does not bypass runtime admission.
    {
        let store = RedbStore::open(&config.data_root)?;
        store.put_revision(&revision)?;
    }
    let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
    daemon
        .client
        .submit(&request(
            "start-aggregate",
            None,
            Command::StartRun {
                run_id: "aggregate".into(),
                workflow_id: "aggregate-bound".into(),
                revision_id: revision.id().to_string(),
                inputs: vec![],
            },
        ))
        .await?;
    let timeout = Duration::from_secs(60);
    let snapshot = super::diagnostics::FailureSnapshot::new(timeout);
    // Poll the durable terminal index while execution is pending. A full run read
    // repeatedly authorizes every accumulated artifact; that work obscures this
    // fixture's purpose, which is to inspect the complete result after execution.
    let state = tokio::time::timeout(timeout, async {
        loop {
            let page = daemon
                .client
                .runs(
                    Some("terminal"),
                    Some("aggregate-bound"),
                    &PageRequest {
                        limit: 1,
                        cursor: None,
                    },
                )
                .await?;
            if let Some(state) = page.items.into_iter().find(|run| run.run_id == "aggregate") {
                return Ok::<_, ClientError>(state);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    let state = match state {
        Ok(state) => state?,
        Err(_) => {
            let fixture_requests = model.requests.lock().map_err(|_| "requests lock")?.len();
            return Err(snapshot
                .capture(
                    &daemon.client,
                    Some("aggregate"),
                    Some("aggregate-bound"),
                    None,
                    Some(fixture_requests),
                    "aggregate run did not reach its indexed terminal before the deadline",
                )
                .await
                .into());
        }
    };
    assert_eq!(
        state.terminal.as_deref(),
        Some("succeeded"),
        "{:?}",
        state
            .nodes
            .iter()
            .filter_map(|node| node
                .latest_attempt
                .as_ref()
                .map(|attempt| (&node.node_id, &attempt.terminal_detail)))
            .collect::<Vec<_>>()
    );
    let result = daemon.client.run_result("aggregate").await?;
    assert!(result.truncated);
    assert_eq!(result.outputs.len(), 1);
    assert_eq!(
        result
            .outputs
            .first()
            .ok_or("output")?
            .preview
            .as_ref()
            .ok_or("preview")?
            .len(),
        4096
    );
    assert!(result.outputs.first().ok_or("output")?.preview_truncated);
    let mut unlimited = result.clone();
    let mut omitted = 0;
    for node in &mut unlimited.run.nodes {
        if let Some(attempt) = &node.latest_attempt_id {
            let detail = daemon.client.attempt("aggregate", attempt).await?;
            if node.node_id.starts_with("step-") {
                assert_eq!(detail.context.as_ref().ok_or("context")?.entries.len(), 8);
                assert!(!detail.outputs.is_empty());
                assert!(detail.capability_provenance.is_some());
                assert!(detail.execution_authority.is_some());
            }
            omitted += usize::from(node.latest_attempt.is_none());
            node.latest_attempt = Some(detail);
            assert_eq!(
                daemon
                    .client
                    .node("aggregate", &node.execution_id)
                    .await?
                    .latest_attempt_id,
                node.latest_attempt_id
            );
        }
    }
    assert!(omitted > 0);
    assert!(
        serde_json::to_vec(&unlimited)?.len() > MAX_DOCUMENT_BYTES,
        "aggregate counterexample did not cross the byte bound"
    );
    let bytes = milkdrift_control_protocol::encode_json(&ResponseEnvelope {
        protocol: ProtocolVersion::CURRENT,
        request_id: "\"".repeat(MAX_REQUEST_ID_BYTES),
        value: result.clone(),
    })?;
    let decoded: ResponseEnvelope<RunResultRead> = decode_json(&bytes)?;
    assert_eq!(decoded.value, result);
    daemon.stop().await?;
    Ok(())
}
