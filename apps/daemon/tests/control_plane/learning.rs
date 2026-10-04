//! Identity-based source selection uses retained artifacts and refuses invented comparisons.
use super::support::*;
use serde_json::{Value, json};

pub(super) async fn selection_roundtrip(
    daemon: &RunningDaemon,
    directory: &TempDir,
) -> TestResult<Value> {
    let revision = BlueprintRevision::genesis(
        WorkflowId::new("learning-source")?,
        MutationBatch::new(vec![Mutation::AddNode {
            node: Node::new(
                NodeId::new("done")?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )?,
        }])?,
        AuthorRef::new("human:operator")?,
        "source history for selected evidence",
    )?;
    daemon
        .client
        .submit(&request(
            "learning-source-definition",
            None,
            Command::ImportBlueprint {
                document: serde_json::from_slice(
                    &BlueprintRevisionDocument::new(&revision).to_canonical_json()?,
                )?,
            },
        ))
        .await?;
    daemon
        .client
        .submit(&request(
            "learning-source-start",
            Some(0),
            Command::StartRun {
                run_id: "learning-source".into(),
                workflow_id: "learning-source".into(),
                revision_id: revision.id().to_string(),
                inputs: vec![],
            },
        ))
        .await?;
    wait_for_run(
        &daemon.client,
        "learning-source",
        Duration::from_secs(10),
        |run| run.terminal.is_some(),
    )
    .await?;
    let guidance = super::inputs::upload(
        &daemon.client,
        "knowledge-guidance",
        b"A controlled evidence selection, with no claim of learned improvement.",
    )
    .await?;
    let args = [
        "learning",
        "select",
        revision.id().as_str(),
        "--workspace",
        "slotbook",
        "--guidance",
        &guidance.artifact_id,
        "--page",
        "learning-source",
        "1",
        "1",
    ];
    let selected = cli_ok(daemon, directory, "selected-knowledge", &args)?;
    assert_eq!(
        selected
            .pointer("/receipt")
            .ok_or("fixture field /receipt absent")?,
        &json!({"actor":"human:integration-controller","command":"selected-knowledge"})
    );
    let record = selected
        .pointer("/value")
        .ok_or("fixture field /value absent")?;
    assert_eq!(
        record
            .pointer("/kind")
            .ok_or("fixture field /kind absent")?,
        "selection"
    );
    let reference: ArtifactReference = serde_json::from_value(
        record
            .pointer("/selection/guidance")
            .ok_or("fixture field /selection/guidance absent")?
            .clone(),
    )?;
    let metadata = daemon
        .client
        .artifact_metadata(&guidance.artifact_id)
        .await?;
    assert_eq!(reference.artifact().as_str(), metadata.artifact_id);
    assert_eq!(reference.digest().to_hex(), metadata.digest);
    assert_eq!(reference.size_bytes(), metadata.size);
    assert_eq!(
        record
            .pointer("/pages/0/events")
            .ok_or("fixture field /pages/0/events absent")?
            .as_array()
            .ok_or("events")?
            .len(),
        1
    );
    assert_eq!(
        cli_ok(daemon, directory, "selected-knowledge", &args)?
            .get("value")
            .ok_or("selected record absent")?,
        record
    );
    let inspected = cli_ok(
        daemon,
        directory,
        "inspect-selection",
        &[
            "learning",
            "inspect",
            "human:integration-controller",
            "selected-knowledge",
        ],
    )?;
    assert_eq!(
        inspected
            .pointer("/value")
            .ok_or("fixture field /value absent")?,
        record
    );
    assert_eq!(
        inspected
            .pointer("/receipt")
            .ok_or("fixture field /receipt absent")?,
        selected
            .pointer("/receipt")
            .ok_or("fixture field /receipt absent")?
    );
    let (ok, text) = cli(
        daemon,
        directory,
        "human-selection",
        &[
            "learning",
            "inspect",
            "human:integration-controller",
            "selected-knowledge",
        ],
        false,
    )?;
    assert!(
        ok && text.contains("guidance") && text.contains("pages") && text.contains("selection"),
        "{text}"
    );
    let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(
        observer
            .submit(&request(
                "private-selection",
                None,
                Command::Learning {
                    document: json!({"type":"inspect","receipt":selected.pointer("/receipt").ok_or("fixture field /receipt absent")?})
                }
            ))
            .await
            .is_err()
    );
    let selection = json!({"method":revision.id(),"workspace":"slotbook","guidance":guidance.artifact_id,"artifacts":[],"pages":[{"run":"learning-source","first":1,"count":64}],"supersedes":null,"approval":null});
    let missing = daemon
        .client
        .submit(&request(
            "missing-pages",
            None,
            Command::Learning {
                document: json!({"type":"select_sources","selection":selection}),
            },
        ))
        .await;
    assert!(
        matches!(missing, Err(ClientError::Api(error)) if error.message.contains("missing evidence"))
    );
    let mut hidden = selection;
    *hidden
        .pointer_mut("/guidance")
        .ok_or("fixture field /guidance absent")? = json!("artifact:absent");
    assert!(
        daemon
            .client
            .submit(&request(
                "hidden-source",
                None,
                Command::Learning {
                    document: json!({"type":"select_sources","selection":hidden})
                }
            ))
            .await
            .is_err()
    );
    // A source selection cannot be passed off as an evaluator declaration or comparison.
    let (ok, _) = cli(
        daemon,
        directory,
        "compare-wrong-receipts",
        &[
            "learning",
            "compare",
            "--declaration",
            "human:integration-controller",
            "selected-knowledge",
            "--candidate",
            "human:integration-controller",
            "selected-knowledge",
        ],
        true,
    )?;
    assert!(!ok);
    let fabricated = json!({"type":"compare","declaration":selected.pointer("/receipt").ok_or("fixture field /receipt absent")?,"candidate":selected.pointer("/receipt").ok_or("fixture field /receipt absent")?,"scores":[100]});
    assert!(
        daemon
            .client
            .submit(&request(
                "fabricated-scores",
                None,
                Command::Learning {
                    document: fabricated
                }
            ))
            .await
            .is_err()
    );
    // Handwritten graph edits have no authenticated model-output provenance.
    fs::write(
        directory.path().join("manual-proposal.json"),
        br#"{"mutations":[]}"#,
    )?;
    let (ok, _) = cli(
        daemon,
        directory,
        "manual-candidate",
        &[
            "learning",
            "candidate",
            "manual-proposal.json",
            "--declaration",
            "human:integration-controller",
            "selected-knowledge",
            "--expected-benefit",
            "fewer failures",
            "--applicability",
            "this study",
            "--counterevidence",
            "a failed required check",
        ],
        true,
    )?;
    assert!(!ok);
    Ok(record.clone())
}

pub(super) async fn selection_after_restart(
    daemon: &RunningDaemon,
    expected: &Value,
) -> TestResult {
    let read = daemon.client.submit(&request("selection-after-restart", None, Command::Learning { document: json!({"type":"inspect","receipt":{"actor":"human:integration-controller","command":"selected-knowledge"}}) })).await?;
    assert_eq!(&read.value, expected);
    Ok(())
}
