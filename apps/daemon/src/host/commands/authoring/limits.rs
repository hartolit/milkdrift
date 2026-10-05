//! Legal definitions cross the complete response boundary before any new revision is stored.
use super::*;
use crate::host::DaemonHost;
use milkdrift_control_client::{BearerCredential, ClientConfig, ControlClient};
use milkdrift_control_protocol::{
    Command, MAX_DOCUMENT_BYTES, MAX_REQUEST_ID_BYTES, ProtocolVersion, ResponseEnvelope,
};
use milkdrift_persistence::{ApplicationCommandStore, CommandId, RevisionStore};
use serde_json::Value;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug)]
enum Mode {
    Open,
    Draft,
    Save,
    Construct,
    Copy,
}

fn command(mode: Mode, base: &BlueprintRevision) -> CommandRequest {
    let draft = BlueprintDraft {
        workflow_id: "size-boundary".into(),
        base_revision: Some(base.id().to_string()),
        mutations: vec![],
    };
    CommandRequest {
        protocol: ProtocolVersion::CURRENT,
        command_id: "c".repeat(192),
        expected_sequence: None,
        expected_revision: draft.base_revision.clone(),
        reason: "Exact complete result capacity".into(),
        evidence: vec![],
        command: match mode {
            Mode::Open | Mode::Draft | Mode::Save => Command::AuthorBlueprint {
                draft,
                edit: (!matches!(mode, Mode::Open)).then(|| BlueprintEdit::Rename {
                    name: "Edited size boundary".into(),
                }),
                save: matches!(mode, Mode::Save),
            },
            Mode::Construct => Command::ConstructBlueprint { draft, store: true },
            Mode::Copy => Command::CopyBlueprint {
                source_revision: base.id().to_string(),
                workflow_id: "size-copied".into(),
                name: "Edited size boundary".into(),
            },
        },
    }
}

fn case(
    owner: &Owner,
    session: &ActorSession,
    mode: Mode,
    total: usize,
    text: char,
) -> TestResult<(
    BlueprintRevision,
    BlueprintRevision,
    CommandRequest,
    CommandAccepted,
)> {
    let (_, legacy) = BlueprintRevisionDocument::from_json(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/authoring/legacy-workflow.json"
    )))?;
    let template = ModelWorkflow::read(&legacy)?
        .steps
        .first()
        .ok_or("template step")?
        .clone();
    let mut model = ModelWorkflow::empty("size-boundary");
    for index in 0..40 {
        let mut step = template.clone();
        step.id = format!("step-{index:02}");
        step.inputs.clear();
        let mut request = serde_json::to_value(&step.request)?;
        *request
            .pointer_mut("/messages/0/parts/0/text")
            .ok_or("prompt")? = Value::String(
            text.to_string()
                .repeat(total / 40 + usize::from(index < total % 40)),
        );
        step.request = serde_json::from_value(request)?;
        model.steps.push(step);
    }
    model.output = Some(("step-39".into(), "result".into()));
    model.complete()?;
    let base = model.build(
        WorkflowId::new("size-boundary")?,
        AuthorRef::new(session.actor.as_str())?,
        "Boundary source",
    )?;
    // Every generated fixture goes through the same canonical reader as an import.
    let (_, base) = BlueprintRevisionDocument::from_json(
        &BlueprintRevisionDocument::new(&base).to_canonical_json()?,
    )?;
    let mut request = command(mode, &base);
    let diagnostics = model
        .authorize_models(owner, session, Some(&model))
        .map_err(|e| e.message)?;
    if !matches!(mode, Mode::Open) {
        model.name = "Edited size boundary".into();
    }
    let generated = model.build(
        WorkflowId::new("size-boundary")?,
        AuthorRef::new(session.actor.as_str())?,
        &request.reason,
    )?;
    let mut draft = BlueprintDraft {
        workflow_id: "size-boundary".into(),
        base_revision: Some(base.id().to_string()),
        mutations: graph::difference(Some(&base), &generated)
            .into_iter()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()?,
    };
    let (revision, view) = if matches!(mode, Mode::Copy) {
        let revision = model.build(
            WorkflowId::new("size-copied")?,
            AuthorRef::new(session.actor.as_str())?,
            &format!("Copy of {}. {}", base.id(), request.reason),
        )?;
        draft = BlueprintDraft {
            workflow_id: "size-copied".into(),
            base_revision: None,
            mutations: vec![],
        };
        (revision, json!({"copied_from":base.id()}))
    } else {
        if let Command::ConstructBlueprint { draft: input, .. } = &mut request.command {
            *input = draft.clone();
        }
        (
            candidate(&draft, Some(&base), session, &request.reason).map_err(|e| e.message)?,
            if matches!(mode, Mode::Construct) {
                Value::Null
            } else {
                model.view(&diagnostics)
            },
        )
    };
    let accepted = result(
        &request,
        &mut draft,
        &revision,
        matches!(mode, Mode::Save | Mode::Construct | Mode::Copy),
        view,
    )
    .map_err(|e| e.message)?;
    Ok((base, revision, request, accepted))
}

fn envelope(value: &CommandAccepted) -> TestResult<Vec<u8>> {
    Ok(serde_json::to_vec(&ResponseEnvelope {
        protocol: ProtocolVersion::CURRENT,
        request_id: "\"".repeat(MAX_REQUEST_ID_BYTES),
        value,
    })?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legal_authoring_boundaries_fit_receipts_http_and_replay() -> TestResult {
    for mode in [
        Mode::Open,
        Mode::Draft,
        Mode::Save,
        Mode::Construct,
        Mode::Copy,
    ] {
        for text in ['x', '"'] {
            let root = tempfile::tempdir()?;
            let token = root.path().join("token");
            std::fs::write(&token, "size-boundary-token")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600))?;
            }
            let host = DaemonHost::start(crate::host::tests::owner_test_config(
                root.path(),
                &token,
                32,
            )?)?;
            let session = host
                .auth
                .authenticate(b"size-boundary-token")
                .ok_or("session")?;
            let (request, expected) = host
                .dispatch(false, move |owner| {
                    let check = || -> TestResult<_> {
                        let overhead = envelope(&case(owner, &session, mode, 40, text)?.3)?.len();
                        let step =
                            envelope(&case(owner, &session, mode, 41, text)?.3)?.len() - overhead;
                        let total = 40 + (MAX_DOCUMENT_BYTES - overhead) / step;
                        let (base, revision, request, accepted) =
                            case(owner, &session, mode, total, text)?;
                        let size = envelope(&accepted)?.len();
                        assert!(
                            size <= MAX_DOCUMENT_BYTES && MAX_DOCUMENT_BYTES - size < step,
                            "{mode:?} {text:?}: {size}"
                        );
                        let (large_base, large_revision, mut large_request, large) =
                            case(owner, &session, mode, total + 1, text)?;
                        // The old value-only test accepts this counterexample; the complete
                        // envelope cannot. Refusal must precede the candidate revision write.
                        assert!(serde_json::to_vec(&large)?.len() <= MAX_DOCUMENT_BYTES);
                        assert!(envelope(&large)?.len() > MAX_DOCUMENT_BYTES);
                        owner.store.put_revision(&large_base)?;
                        large_request.command_id = "d".repeat(192);
                        let refusal =
                            crate::host::receipts::execute(owner, &session, large_request.clone())
                                .err()
                                .ok_or("oversized success")?;
                        assert_eq!(refusal.code, ErrorCode::InvalidInput);
                        if !matches!(mode, Mode::Open) {
                            assert!(owner.store.revision(large_revision.id())?.is_none());
                        }
                        let receipt = owner
                            .store
                            .application_command_receipt(
                                &session.actor,
                                &CommandId::new(&large_request.command_id)?,
                            )?
                            .ok_or("refusal receipt")?;
                        assert!(matches!(
                            receipt.result(),
                            milkdrift_persistence::ApplicationCommandResult::Rejected { .. }
                        ));
                        owner.store.put_revision(&base)?;
                        if !matches!(mode, Mode::Open) {
                            assert!(owner.store.revision(revision.id())?.is_none());
                        }
                        Ok((request, accepted))
                    };
                    check().map_err(|e| failure(e.to_string()))
                })
                .await
                .map_err(|e| e.message)?;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let endpoint = url::Url::parse(&format!("http://{}/", listener.local_addr()?))?;
            let (stop, stopped) = tokio::sync::oneshot::channel();
            let serving_host = host.clone();
            let serving = tokio::spawn(async move {
                crate::serve(listener, serving_host, async {
                    let _closed = stopped.await;
                })
                .await
            });
            // Exercise the real transport with the maximum legal, maximally escaped ID.
            let response = reqwest::Client::new()
                .post(endpoint.join("v1/commands")?)
                .bearer_auth("size-boundary-token")
                .header("x-request-id", "\"".repeat(MAX_REQUEST_ID_BYTES))
                .json(&request)
                .send()
                .await?;
            let status = response.status();
            let bytes = response.bytes().await?;
            assert!(
                status.is_success(),
                "{mode:?} {text:?}: {}",
                String::from_utf8_lossy(&bytes)
            );
            let initial: ResponseEnvelope<CommandAccepted> =
                milkdrift_control_protocol::decode_json(&bytes)?;
            assert_eq!(initial.value, expected);
            let client = ControlClient::new(
                ClientConfig::new(endpoint.clone()),
                BearerCredential::new("size-boundary-token")?,
            )?;
            let replay = client.submit(&request).await?;
            assert!(replay.replayed);
            assert_eq!(replay.value, expected.value);
            let replay_http = reqwest::Client::new()
                .post(endpoint.join("v1/commands")?)
                .bearer_auth("size-boundary-token")
                .header("x-request-id", "\\".repeat(MAX_REQUEST_ID_BYTES))
                .json(&request)
                .send()
                .await?;
            assert!(replay_http.status().is_success());
            let replay_http: ResponseEnvelope<CommandAccepted> =
                milkdrift_control_protocol::decode_json(&replay_http.bytes().await?)?;
            assert_eq!(replay_http.value, replay);
            stop.send(()).map_err(|_| "server stopped")?;
            serving.await??;
        }
    }
    Ok(())
}

struct FailOnce {
    point: milkdrift_redb_store::FaultPoint,
    armed: std::sync::atomic::AtomicBool,
}

impl milkdrift_redb_store::FaultInjector for FailOnce {
    fn check(
        &self,
        point: milkdrift_redb_store::FaultPoint,
    ) -> Result<(), milkdrift_persistence::PersistenceError> {
        if point == self.point && self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            // Bounds used to be converted to a durable rejection even after a successful
            // producer. Inject at the actual transaction boundary, including lost replies.
            Err(milkdrift_persistence::PersistenceError::Bounds {
                location: "test_result_commit",
                reason: "injected durable writer failure".into(),
            })
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn definition_write_and_receipt_failures_remain_uncertain_and_exactly_recoverable()
-> TestResult {
    use milkdrift_redb_store::{FaultPoint, RedbStore, RedbStoreConfig};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    for point in [
        FaultPoint::BeforeRevisionCommit,
        FaultPoint::AfterRevisionCommit,
        FaultPoint::BeforeApplicationCommit,
        FaultPoint::AfterApplicationCommit,
    ] {
        let root = tempfile::tempdir()?;
        let token = root.path().join("token");
        std::fs::write(&token, "failure-boundary-token")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600))?;
        }
        let host = DaemonHost::start(crate::host::tests::owner_test_config(
            root.path(),
            &token,
            32,
        )?)?;
        let session = host
            .auth
            .authenticate(b"failure-boundary-token")
            .ok_or("session")?;
        let faults = Arc::new(FailOnce {
            point,
            armed: AtomicBool::new(false),
        });
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(root.path().join("fault-store"))
                .with_fault_injector(faults.clone()),
        )?);
        host.dispatch(false, move |owner| {
            let check = || -> TestResult {
                // Redirect the definition/receipt owners to the real fault-injected store;
                // this fixture never schedules work against the unrelated runtime store.
                owner.store = store;
                let workflow = owner.workflow.as_mut().ok_or("workflow")?;
                workflow.control = Arc::new(milkdrift_control::ControlService::new(
                    owner.store.clone(),
                    owner.store.clone(),
                    workflow.runtime.clone(),
                    owner.authority.clone(),
                ));
                let (base, revision, request, expected) =
                    case(owner, &session, Mode::Save, 40, 'x')?;
                owner.store.put_revision(&base)?;
                faults.armed.store(true, Ordering::SeqCst);
                let failure = crate::host::receipts::execute(owner, &session, request.clone())
                    .err()
                    .ok_or("fault must refuse a certain result")?;
                assert_eq!(
                    failure.code,
                    ErrorCode::Uncertain,
                    "{point:?}: {}",
                    failure.message
                );
                assert!(!failure.retryable);
                assert_eq!(
                    owner.store.revision(revision.id())?.is_some(),
                    point != FaultPoint::BeforeRevisionCommit
                );
                let identity = CommandId::new(&request.command_id)?;
                let receipt = owner
                    .store
                    .application_command_receipt(&session.actor, &identity)?;
                assert_eq!(
                    receipt.is_some(),
                    point == FaultPoint::AfterApplicationCommit
                );
                if let Some(receipt) = receipt {
                    assert!(matches!(
                        receipt.result(),
                        milkdrift_persistence::ApplicationCommandResult::Accepted { .. }
                    ));
                }
                let recovered = crate::host::receipts::execute(owner, &session, request.clone())
                    .map_err(|e| e.message)?;
                assert_eq!(recovered.value, expected.value);
                assert_eq!(owner.store.revision(revision.id())?, Some(revision));
                assert!(
                    crate::host::receipts::execute(owner, &session, request)
                        .map_err(|e| e.message)?
                        .replayed
                );
                Ok(())
            };
            check().map_err(|e| failure(e.to_string()))
        })
        .await
        .map_err(|e| e.message)?;
        host.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn deepest_legal_definition_values_survive_complete_wrappers_and_common_producers()
-> TestResult {
    use milkdrift_blueprint::{BlueprintMetadata, Node, NodeId, NodeKind, TerminalOutcome};
    use milkdrift_capability::{BoundedJson, ExtensionKey};
    use std::collections::{BTreeMap, BTreeSet};
    let mut value = json!("quotes\" backslashes\\ and\nlines");
    let mut depth = 0;
    while BoundedJson::new(json!([value.clone()])).is_ok() {
        value = json!([value]);
        depth += 1;
        assert!(depth <= 64);
    }
    assert!(depth >= 40, "fixture must reach the legal nesting boundary");
    let metadata = BlueprintMetadata::new(
        "Nested definition",
        "",
        BTreeSet::new(),
        BTreeMap::from([(
            ExtensionKey::new("org.milkdrift/nested-fixture")?,
            BoundedJson::new(value)?,
        )]),
    )?;
    let operations = [
        Mutation::SetMetadata { metadata },
        Mutation::AddNode {
            node: Node::new(
                NodeId::new("done")?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )?,
        },
    ];
    let root = tempfile::tempdir()?;
    let token = root.path().join("token");
    std::fs::write(&token, "nested-definition-token")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600))?;
    }
    let host = DaemonHost::start(crate::host::tests::owner_test_config(
        root.path(),
        &token,
        32,
    )?)?;
    let session = host
        .auth
        .authenticate(b"nested-definition-token")
        .ok_or("session")?;
    host.dispatch(false, move |owner| {
        let check = || -> TestResult {
            let draft = BlueprintDraft {
                workflow_id: "nested-definition".into(),
                base_revision: None,
                mutations: operations
                    .iter()
                    .map(serde_json::to_value)
                    .collect::<Result<_, _>>()?,
            };
            let mut request = CommandRequest {
                protocol: ProtocolVersion::CURRENT,
                command_id: "nested-construct".into(),
                expected_sequence: None,
                expected_revision: None,
                reason: "Exercise deepest legal extension".into(),
                evidence: vec![],
                command: Command::ConstructBlueprint { draft, store: true },
            };
            let constructed = crate::host::receipts::execute(owner, &session, request.clone())
                .map_err(|e| e.message)?;
            let document = constructed.value.get("document").ok_or("document")?.clone();
            BlueprintRevisionDocument::from_json(&serde_json::to_vec(&document)?)?;
            for (id, command) in [
                ("nested-replay", request.command.clone()),
                (
                    "nested-import",
                    Command::ImportBlueprint {
                        document: document.clone(),
                    },
                ),
                ("nested-validate", Command::ValidateBlueprint { document }),
            ] {
                request.command_id = id.into();
                request.command = command;
                let result = crate::host::receipts::execute(owner, &session, request.clone())
                    .map_err(|e| e.message)?;
                let decoded: ResponseEnvelope<CommandAccepted> =
                    milkdrift_control_protocol::decode_json(&envelope(&result)?)?;
                assert_eq!(decoded.value, result);
                let replay = crate::host::receipts::execute(owner, &session, request.clone())
                    .map_err(|e| e.message)?;
                assert!(replay.replayed);
                assert_eq!(replay.value, result.value);
            }
            Ok(())
        };
        check().map_err(|e| failure(e.to_string()))
    })
    .await
    .map_err(|e| e.message)?;
    host.shutdown().await?;
    Ok(())
}
