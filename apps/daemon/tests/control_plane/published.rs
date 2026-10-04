//! Public invocation, restricted results, recovery and retirement through the production daemon.
use super::support::*;
#[path = "../support/published.rs"]
pub(super) mod fixture;
use fixture::definition;
use milkdrift_capability::{
    AdmissionConstraints, CapabilityCategory, DescriptorBuilder, InvocationCounts, InvocationId,
    InvocationRequest, Locality, ResolvedCapabilitySnapshot, TerminalStatus,
};
use milkdrift_control::PublicationDraft;
use milkdrift_peer_protocol::{DirectInvocationRequest, InvocationAcceptance, PeerRequestId};
use milkdrift_persistence::published::{PublishedMethod, PublishedOutput};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invoke_only_published_outputs_replay_retirement_and_restart() -> TestResult {
    let directory = TempDir::new()?;
    let profile = configured_process_profile(&directory)?;
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&profile)?)?;
    *value
        .pointer_mut("/profile/stdout")
        .ok_or("fixture field /profile/stdout absent")? = json!({"max_capture_bytes":1024,"stream_progress":false,"max_progress_events":0,"overflow_action":"terminate","artifact_name":"result"});
    fs::write(&profile, serde_json::to_vec(&value)?)?;
    let mut config = configuration_document_with_process_profiles(&directory, 64, vec![profile])?;
    config.runtime.controller_activation = milkdrift_daemon::ControllerActivation::Enabled;
    config.runtime.effect_threads = 1;
    config.runtime.effect_queue = 1;
    config.serving.worker_threads = 1;
    config.serving.observation_hot_retention_ms = 100;
    config.serving.poll_interval_ms = 5;
    config.serving.clients.execution_limits.nested_invocations = Some(InvocationCounts::new(4, 0));
    config.runtime.publication_services.insert(
        CapabilityId::new("method:echo")?,
        milkdrift_authority::GrantId::new("grant:integration-controller")?,
    );
    // No arbitrary artifact, workspace, run, revision, resource or secret authority.
    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .preset = AuthorityPresetConfig::Invoker;
    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .authority
        .dangerous_allow_broad_authority = true;
    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .authority
        .resources
        .artifacts = ArtifactAuthorityScope::none();
    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .authority
        .resources
        .workflow_run = WorkflowRunScope::Any;
    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .authority
        .resources
        .capability = CapabilityAuthorityScopeBuilder::new(SideEffectClass::ReadOnly)
        .only_capabilities(std::collections::BTreeSet::from([CapabilityId::new(
            "method:echo",
        )?]))?
        .only_categories(std::collections::BTreeSet::from([CapabilityCategory::Tool]))?
        .only_operations(std::collections::BTreeSet::from([OperationId::new(
            "method.invoke",
        )?]))?
        .build();
    config
        .actors
        .get_mut(1)
        .ok_or("fixture actor absent")?
        .authority
        .budget = config
        .actors
        .get_mut(0)
        .ok_or("fixture actor absent")?
        .authority
        .budget;
    let plan = config.validate(directory.path())?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let invoker = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    let (base, revision) = definition()?;
    for (name, definition) in [("base", &base), ("governed", &revision)] {
        daemon
            .client
            .submit(&request(
                name,
                None,
                Command::ImportBlueprint {
                    document: serde_json::from_slice(
                        &BlueprintRevisionDocument::new(definition).to_canonical_json()?,
                    )?,
                },
            ))
            .await?;
    }
    let discovery = daemon.client.execution_discovery().await?;
    let process = discovery
        .catalog
        .entries
        .iter()
        .find(|entry| entry.descriptor.identity().as_str() == "golden-local-process")
        .ok_or("process absent")?;
    let operation = process
        .descriptor
        .operation(&OperationId::new("process.execute")?)
        .ok_or("operation absent")?
        .clone();
    let descriptor = DescriptorBuilder::new(
        CapabilityId::new("method:echo")?,
        1,
        CapabilityCategory::Tool,
        AdmissionConstraints::new(2, 2)?,
        Locality::Local,
    )
    .operations(BTreeMap::from([(
        OperationId::new("method.invoke")?,
        operation,
    )]))
    .execution_trust(process.descriptor.execution_trust())
    .build()?;
    let draft = PublicationDraft {
        documentation: "Run the fixed governed fixture and return only its accepted result."
            .to_owned(),
        descriptor,
        revision: revision.id().clone(),
        service_grant: milkdrift_authority::GrantId::new("grant:integration-controller")?,
        inputs: BTreeMap::new(),
        outputs: BTreeMap::from([(
            "result".to_owned(),
            PublishedOutput {
                field: milkdrift_workspace::ValueKey::new("result")?,
                media_type: "application/octet-stream".to_owned(),
                maximum_bytes: 1024,
            },
        )]),
        workspace_budget: WorkspaceBudget::new(128, 65536, 1048576, 128, 1048576, 16777216)?,
        allowance: milkdrift_persistence::ControllerResourceBudget::new(
            0, None, 1000, 1000, 1048576, 4, 0,
        )?,
        maximum_outstanding: 2,
        maximum_depth: 4,
        maximum_duration_ms: 30000,
    };
    let prepare = request(
        "prepare",
        None,
        Command::PrepareMethod {
            document: serde_json::to_value(&draft)?,
        },
    );
    assert!(invoker.submit(&prepare).await.is_err());
    let mut wrong = draft.clone();
    wrong.service_grant = milkdrift_authority::GrantId::new("grant:integration-observer")?;
    assert!(
        daemon
            .client
            .submit(&request(
                "wrong-service",
                None,
                Command::PrepareMethod {
                    document: serde_json::to_value(wrong)?
                }
            ))
            .await
            .is_err()
    );
    let prepared = daemon.client.submit(&prepare).await?;
    assert_eq!(daemon.client.submit(&prepare).await?.value, prepared.value);
    assert!(
        invoker
            .execution_discovery()
            .await?
            .catalog
            .entries
            .is_empty()
    );
    fs::write(
        directory.path().join("publication.json"),
        serde_json::to_vec(&draft)?,
    )?;
    cli_ok(
        &daemon,
        &directory,
        "prepare-cli",
        &[
            "method",
            "prepare",
            "publication.json",
            "--output",
            "method.json",
        ],
    )?;
    let method: PublishedMethod =
        serde_json::from_slice(&fs::read(directory.path().join("method.json"))?)?;
    assert_eq!(serde_json::to_value(&method)?, prepared.value);
    let (promoted_without_comparison, _) = cli(
        &daemon,
        &directory,
        "unearned-promotion",
        &[
            "learning",
            "promote",
            "method.json",
            "--comparison",
            "human:integration-controller",
            "prepare-cli",
            "--expected-previous-version",
            "1",
        ],
        true,
    )?;
    assert!(!promoted_without_comparison);
    assert!(
        invoker
            .execution_discovery()
            .await?
            .catalog
            .entries
            .is_empty()
    );
    assert_eq!(
        method.agreement,
        revision.semantic().agreement().ok_or("agreement")?.digest()
    );
    assert_eq!(
        method.service.grant_digest.as_str(),
        daemon.client.authority().await?.grant_digest
    );
    let publish = request(
        "publish",
        None,
        Command::PublishMethod {
            document: serde_json::to_value(&method)?,
            expected_previous_version: None,
        },
    );
    assert!(invoker.submit(&publish).await.is_err());
    daemon.client.submit(&publish).await?;
    let changed_publication = request("changed-publication-command", None, publish.command.clone());
    assert!(matches!(daemon.client.submit(&changed_publication).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict));
    let discovery = invoker.execution_discovery().await?;
    assert_eq!(discovery.catalog.entries.len(), 1);
    let entry = discovery
        .catalog
        .entries
        .first()
        .ok_or("catalog entry absent")?;
    let request_doc = DirectInvocationRequest {
        host: discovery.host,
        request_id: PeerRequestId::new("public-call")?,
        catalog_generation: discovery.catalog.generation,
        catalog_digest: discovery.catalog.digest,
        selection: ResolvedCapabilitySnapshot::from_descriptor(
            &entry.descriptor,
            &OperationId::new("method.invoke")?,
        )?,
        request: InvocationRequest::new(
            InvocationId::new("public-call")?,
            method.descriptor.identity().clone(),
            OperationId::new("method.invoke")?,
            None,
            None,
            vec![],
            BTreeMap::new(),
        )?,
        limits: discovery.limits,
        deadline_unix_ms: u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis(),
        )?
        .checked_add(60000)
        .ok_or("invocation deadline overflow")?,
    };
    let accepted = invoker.invoke(&request_doc).await?;
    let InvocationAcceptance::Accepted { execution, .. } = accepted else {
        return Err(format!("not accepted: {accepted:?}").into());
    };
    let mut promoted = method.clone();
    let mut newer = serde_json::to_value(&promoted.descriptor)?;
    *newer
        .pointer_mut("/descriptor_revision")
        .ok_or("fixture field /descriptor_revision absent")? = json!(2);
    promoted.descriptor = serde_json::from_value(newer)?;
    daemon
        .client
        .submit(&request(
            "promote",
            None,
            Command::PublishMethod {
                document: serde_json::to_value(&promoted)?,
                expected_previous_version: Some(1),
            },
        ))
        .await?;
    daemon
        .client
        .submit(&request(
            "retire",
            None,
            Command::RetireMethod {
                capability: "method:echo".to_owned(),
                generation: 1,
                expected_version: 1,
            },
        ))
        .await?;
    match invoker.invoke(&request_doc).await? {
        InvocationAcceptance::Accepted {
            execution: replay,
            replayed: true,
            ..
        }
        | InvocationAcceptance::Archived {
            execution: replay, ..
        } => assert_eq!(replay, execution),
        other => return Err(format!("exact publication replay changed: {other:?}").into()),
    }
    // Observe the method's full accepted allowance and its subsequent terminal publication.
    let deadline = tokio::time::Instant::now()
        + Duration::from_millis(method.maximum_duration_ms)
        + Duration::from_secs(10);
    let terminal = loop {
        let page = invoker.invocation_observations(&execution, 0, 32).await?;
        if let Some(terminal) = page
            .observations
            .iter()
            .find_map(|item| item.event.kind().terminal())
            .or_else(|| match &page.history {
                milkdrift_peer_protocol::ObservationHistory::Hot => None,
                milkdrift_peer_protocol::ObservationHistory::Archived { summary } => summary
                    .final_observation
                    .as_ref()
                    .and_then(|item| item.event.kind().terminal()),
            })
        {
            break terminal.clone();
        }
        if tokio::time::Instant::now() >= deadline {
            #[expect(
                clippy::print_stderr,
                reason = "The failed integration case includes its authorized bounded run page in captured test diagnostics."
            )]
            {
                eprintln!(
                    "runs: {:?}",
                    daemon
                        .client
                        .runs(
                            None,
                            None,
                            &PageRequest {
                                limit: 10,
                                cursor: None
                            }
                        )
                        .await?
                );
            }
            return Err(format!("public operation stalled: {page:?}").into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(terminal.status(), TerminalStatus::Success, "{terminal:?}");
    assert_eq!(
        terminal
            .usage()
            .and_then(|usage| usage.nested_work())
            .map(|nested| nested.invocations().process()),
        Some(1)
    );
    let output = terminal.outputs().first().ok_or("public output absent")?;
    assert!(invoker.artifact_metadata(output.identity()).await.is_err());
    assert!(
        invoker
            .submit(&request(
                "internal-inspect",
                None,
                Command::InspectMethod {
                    capability: "method:echo".to_owned(),
                    generation: 1
                }
            ))
            .await
            .is_err()
    );
    let chunk = invoker
        .invocation_output(&execution, output.identity(), 0, 1024)
        .await?;
    assert_eq!(chunk.bytes, b"golden\n");
    assert_eq!(
        chunk.metadata.sensitivity(),
        ArtifactSensitivity::Restricted
    );
    let runs = daemon
        .client
        .runs(
            None,
            Some("published-test"),
            &PageRequest {
                limit: 10,
                cursor: None,
            },
        )
        .await?;
    assert_eq!(
        runs.items.len(),
        1,
        "one accepted call must own one real internal run"
    );
    let child = runs.items.first().ok_or("run absent")?;
    assert_eq!(child.revision_id.as_deref(), Some(revision.id().as_str()));
    assert!(invoker.run(&child.run_id).await.is_err());
    assert!(
        invoker
            .invocation_output(&execution, "unrelated", 0, 1024)
            .await
            .is_err()
    );
    let mut altered = request_doc.clone();
    altered.deadline_unix_ms += 1;
    assert!(matches!(
        invoker.invoke(&altered).await?,
        InvocationAcceptance::Rejected { .. }
    ));
    assert!(
        daemon
            .client
            .invocation_output(&execution, output.identity(), 0, 1024)
            .await
            .is_err()
    );
    assert!(
        invoker
            .invocation_output(&execution, "unrelated", 0, 1024)
            .await
            .is_err()
    );
    let mut changed = request_doc.clone();
    changed.request_id = PeerRequestId::new("after-retirement")?;
    assert!(matches!(
        invoker.invoke(&changed).await?,
        InvocationAcceptance::Rejected { .. }
    ));
    // The retention horizon makes a record eligible; maintenance performs archival later.
    let archive_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match invoker.invoke(&request_doc).await? {
            InvocationAcceptance::Archived {
                execution: replay, ..
            } => {
                assert_eq!(replay, execution);
                break;
            }
            InvocationAcceptance::Accepted {
                execution: replay,
                replayed: true,
                ..
            } => assert_eq!(replay, execution),
            other => return Err(format!("exact publication replay changed: {other:?}").into()),
        }
        assert!(
            tokio::time::Instant::now() < archive_deadline,
            "completed publication did not archive"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        invoker
            .invocation_output(&execution, output.identity(), 0, 1024)
            .await?
            .bytes,
        b"golden\n"
    );
    daemon.stop().await?;
    let reopened = start(plan, CONTROLLER_TOKEN).await?;
    let invoker = client(&reopened.endpoint, OBSERVER_TOKEN)?;
    assert!(matches!(
        invoker.invoke(&request_doc).await?,
        InvocationAcceptance::Archived { .. }
    ));
    assert_eq!(
        invoker
            .invocation_output(&execution, output.identity(), 0, 1024)
            .await?
            .bytes,
        b"golden\n"
    );
    reopened.stop().await
}
