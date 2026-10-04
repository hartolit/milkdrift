use super::*;
use crate::inputs::ModelFixture;

#[tokio::test]
async fn stalled_reads_and_collection_limits_finish_without_payloads() -> TestResult {
    let directory = TempDir::new()?;
    let mut snapshot = FailureSnapshot::new(Duration::from_millis(1));
    let start = Instant::now();
    let end = tokio::time::Instant::now() + Duration::from_millis(30);
    assert!(
        snapshot
            .read(
                end,
                "stalled",
                std::future::pending::<Result<(), ClientError>>()
            )
            .await
            .is_none()
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    snapshot.reads = MAX_READS;
    assert!(
        snapshot
            .read(end, "exhausted", async {
                Ok::<_, ClientError>("PRIVATE-PROMPT")
            })
            .await
            .is_none()
    );
    let path = retain(directory.path(), json!({"collected":snapshot.collected}))?;
    let text = fs::read_to_string(path)?;
    assert!(text.contains("read deadline"));
    assert!(text.contains("collection limit"));
    assert!(!text.contains("PRIVATE-PROMPT"));
    Ok(())
}

#[test]
fn redaction_truncation_and_parallel_retention_are_explicit() -> TestResult {
    let directory = TempDir::new()?;
    let state: RunRead = serde_json::from_value(
        json!({"run_id":"run","sequence":7,"lifecycle":"running",
        "terminal":null,"workflow_id":"workflow","revision_id":null,"semantic_digest":null,"nodes":[],
        "uncertainty_count":0,"governing_agreement":{"secret":"PRIVATE-PROMPT"},"agreement_adoptions":0,
        "controller_accounting":{"token":"PRIVATE-CREDENTIAL"},"published_source":{"request":"PRIVATE-INPUT"}}),
    )?;
    let summary = run_summary(&state);
    assert!(!summary.to_string().contains("PRIVATE"));
    let paths = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            retain(
                directory.path(),
                json!({"last":summary,"collected":"x".repeat(MAX_BUNDLE_BYTES)}),
            )
            .map_err(|e| e.to_string())
        });
        let second = scope
            .spawn(|| retain(directory.path(), json!({"last":summary})).map_err(|e| e.to_string()));
        Ok::<_, Box<dyn std::error::Error>>((
            first.join().map_err(|_| "first writer panicked")??,
            second.join().map_err(|_| "second writer panicked")??,
        ))
    })?;
    assert_ne!(paths.0, paths.1);
    assert!(fs::read_to_string(&paths.0)?.contains("bundle byte limit"));
    for path in [paths.0, paths.1] {
        assert!(fs::metadata(path)?.len() <= u64::try_from(MAX_BUNDLE_BYTES)?);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_terminal_and_early_error_keep_evidence_and_join_fixtures() -> TestResult {
    let directory = TempDir::new()?;
    let model = ModelFixture::start().await?;
    let address = model.address;
    let outcome = model
        .checked(async |model| {
            let daemon = start(configuration(&directory, 64)?, CONTROLLER_TOKEN).await?;
            daemon
                .checked(model, async |daemon| {
                    let definition = BlueprintRevision::genesis(
                        WorkflowId::new("golden")?,
                        MutationBatch::new(vec![Mutation::AddNode {
                            node: Node::new(
                                NodeId::new("done")?,
                                NodeKind::Terminal {
                                    outcome: TerminalOutcome::Failure,
                                },
                            )?,
                        }])?,
                        AuthorRef::new("human:test")?,
                        "controlled failed terminal",
                    )?;
                    daemon
                        .client
                        .submit(&request(
                            "import",
                            None,
                            Command::ImportBlueprint {
                                document: serde_json::from_slice(
                                    &BlueprintRevisionDocument::new(&definition)
                                        .to_canonical_json()?,
                                )?,
                            },
                        ))
                        .await?;
                    daemon
                        .client
                        .submit(&request(
                            "start",
                            Some(0),
                            Command::StartRun {
                                run_id: "failure".into(),
                                workflow_id: "golden".into(),
                                revision_id: definition.id().to_string(),
                                inputs: vec![],
                            },
                        ))
                        .await?;
                    let timeout =
                        wait_for_run(&daemon.client, "failure", Duration::ZERO, |_| false)
                            .await
                            .err()
                            .ok_or("expected timeout")?
                            .to_string();
                    assert!(timeout.contains("diagnostics:"));
                    assert!(timeout.contains("bounded state"));
                    let terminal =
                        wait_for_run(&daemon.client, "failure", Duration::from_secs(5), |_| false)
                            .await
                            .err()
                            .ok_or("expected terminal failure")?
                            .to_string();
                    assert!(terminal.contains("unexpected workflow terminal"));
                    assert!(terminal.contains("diagnostics:"));
                    Err("primary early error".into())
                })
                .await
        })
        .await;
    assert!(
        outcome
            .err()
            .ok_or("expected failure")?
            .to_string()
            .starts_with("primary early error")
    );
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    let _store = RedbStore::open(directory.path().join("data"))?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn assertion_unwind_and_failed_retention_still_join_fixture() -> TestResult {
    use futures_util::FutureExt as _;
    let directory = TempDir::new()?;
    let blocked = directory.path().join("not-a-directory");
    fs::write(&blocked, b"owned test file")?;
    assert!(retain(&blocked, json!({})).is_err());
    let model = ModelFixture::start().await?;
    let address = model.address;
    let result = std::panic::AssertUnwindSafe(model.checked(async |model| {
        let daemon = start(configuration(&directory, 64)?, CONTROLLER_TOKEN).await?;
        daemon
            .checked(model, async |_daemon| {
                assert_eq!(1, 2, "controlled assertion failure");
                Ok(())
            })
            .await
    }))
    .catch_unwind()
    .await;
    assert!(result.is_err());
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    let _store = RedbStore::open(directory.path().join("data"))?;
    Ok(())
}
