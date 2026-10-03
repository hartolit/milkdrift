//! Public authoring proves definitions and refusals without entering an external capability.
use super::support::*;
use milkdrift_control_protocol::{
    BlueprintDraft, BlueprintEdit, CommandAccepted, ModelInputSource,
};
use serde_json::json;

async fn author(
    client: &ControlClient,
    draft: &BlueprintDraft,
    id: &str,
    edit: Option<BlueprintEdit>,
    save: bool,
) -> Result<CommandAccepted, ClientError> {
    let mut command = request(
        id,
        None,
        Command::AuthorBlueprint {
            draft: draft.clone(),
            edit,
            save,
        },
    );
    command.expected_revision = draft.base_revision.clone();
    client.submit(&command).await
}
fn draft(result: &CommandAccepted) -> TestResult<BlueprintDraft> {
    Ok(serde_json::from_value(result.value["draft"].clone())?)
}
fn add(step: &str) -> BlueprintEdit {
    BlueprintEdit::AddModel {
        step: step.into(),
        capability: "writing-model".into(),
        prompt: "Use only the explicitly supplied evidence.".into(),
        maximum_output_units: 512,
    }
}
fn initial(workflow: &str) -> BlueprintDraft {
    BlueprintDraft {
        workflow_id: workflow.into(),
        base_revision: None,
        mutations: vec![],
    }
}

fn editor_scope() -> TestResult<CapabilityAuthorityScope> {
    use std::collections::BTreeSet;
    Ok(
        CapabilityAuthorityScopeBuilder::new(SideEffectClass::Unknown)
            .only_capabilities(BTreeSet::from([
                CapabilityId::new("writing-model")?,
                CapabilityId::new("milkdrift-workflow-control")?,
            ]))?
            .only_operations(BTreeSet::from([
                OperationId::new("model.generate")?,
                OperationId::new("workflow.accept_result")?,
            ]))?
            .only_trust_zones(BTreeSet::from([
                milkdrift_capability::TrustZone::new("test")?,
                milkdrift_capability::TrustZone::new("milkdrift-control")?,
            ]))?
            .build(),
    )
}

pub(super) fn model_configuration(
    directory: &TempDir,
    address: std::net::SocketAddr,
) -> TestResult<DaemonPlan> {
    use milkdrift_model_provider::{
        AuthMode, BillingTerms, EndpointLimits, EndpointProfile, ModelFeature, ModelTokenLimits,
        ProviderProtocol, ProxyPolicy, RedirectPolicy, TlsPolicy,
    };
    let profile = EndpointProfile::new(
        milkdrift_capability::ProviderProfileRef::new("writing-profile")?,
        1,
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".into(),
        },
        format!("http://{address}"),
        "controlled-writer",
        AuthMode::NoAuth,
        EndpointLimits {
            connect_timeout_ms: 1000,
            request_timeout_ms: 5000,
            idle_timeout_ms: 5000,
            max_request_bytes: 262_144,
            max_response_bytes: 262_144,
            max_headers: 64,
            max_header_bytes: 65_536,
            max_stream_line_bytes: 4096,
            max_stream_event_bytes: 65_536,
            max_fragment_bytes: 4096,
        },
        RedirectPolicy::Deny,
        TlsPolicy::WebPkiRoots,
        ProxyPolicy::Disabled,
        std::collections::BTreeSet::from([ModelFeature::SystemRole]),
        1,
        true,
        std::collections::BTreeSet::from(["127.0.0.1".into()]),
        std::collections::BTreeSet::from(["test".into()]),
        BTreeMap::new(),
        BillingTerms::Unknown,
        ModelTokenLimits::Unknown,
    )?;
    let path = directory.path().join("model.json");
    fs::write(&path, profile.to_canonical_json()?)?;
    let mut config = configuration_document_with_process_profiles(directory, 64, vec![])?;
    config.actors[0].authority.resources.capability = editor_scope()?;
    config.actors[1].authority = ActorGrantConfig::dangerous_administrator();
    config.actors[1].authority.resources.capability = CapabilityAuthorityScope::deny_all();
    config
        .adapters
        .model_profiles
        .push(milkdrift_daemon::ModelProfileConfig {
            capability_id: "writing-model".into(),
            profile: path,
        });
    Ok(config.validate(directory.path())?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_authoring_saves_reopens_and_refuses_unsafe_edits_without_execution() -> TestResult {
    let directory = tempfile::tempdir()?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let plan = model_configuration(&directory, listener.local_addr()?)?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let hidden = client(&daemon.endpoint, OBSERVER_TOKEN)?;
    assert!(hidden.capabilities().await?.is_empty());
    assert!(
        author(
            &hidden,
            &initial("hidden-model"),
            "hidden-selection",
            Some(add("draft")),
            false
        )
        .await
        .is_err()
    );
    let mut notes = draft(
        &author(
            &daemon.client,
            &initial("release-notes"),
            "new-notes",
            Some(BlueprintEdit::Rename {
                name: "Release notes".into(),
            }),
            false,
        )
        .await?,
    )?;
    assert!(
        author(&daemon.client, &notes, "incomplete", None, true)
            .await
            .is_err()
    );
    let mut index = 0;
    for edit in [
        BlueprintEdit::Input {
            name: "brief".into(),
        },
        add("draft"),
        add("review"),
        BlueprintEdit::Connect {
            step: "draft".into(),
            input: "brief".into(),
            source: ModelInputSource::RunInput {
                name: "brief".into(),
            },
        },
        BlueprintEdit::Connect {
            step: "review".into(),
            input: "brief".into(),
            source: ModelInputSource::RunInput {
                name: "brief".into(),
            },
        },
        BlueprintEdit::Connect {
            step: "review".into(),
            input: "draft".into(),
            source: ModelInputSource::Step {
                step: "draft".into(),
            },
        },
        BlueprintEdit::Output {
            step: "review".into(),
            name: "notes".into(),
        },
    ] {
        index += 1;
        notes = draft(
            &author(
                &daemon.client,
                &notes,
                &format!("edit-{index}"),
                Some(edit),
                false,
            )
            .await?,
        )?;
    }
    for (id, edit) in [
        (
            "forward",
            BlueprintEdit::Connect {
                step: "draft".into(),
                input: "future".into(),
                source: ModelInputSource::Step {
                    step: "review".into(),
                },
            },
        ),
        (
            "missing-input",
            BlueprintEdit::Connect {
                step: "draft".into(),
                input: "other".into(),
                source: ModelInputSource::RunInput {
                    name: "undeclared".into(),
                },
            },
        ),
        (
            "bad-move",
            BlueprintEdit::Move {
                step: "review".into(),
                before: Some("draft".into()),
            },
        ),
        (
            "used-remove",
            BlueprintEdit::Remove {
                step: "draft".into(),
            },
        ),
        (
            "unknown-model",
            BlueprintEdit::Model {
                step: "draft".into(),
                capability: "secret-model".into(),
            },
        ),
    ] {
        assert!(
            author(&daemon.client, &notes, id, Some(edit), false)
                .await
                .is_err(),
            "{id}"
        );
    }
    let saved = author(&daemon.client, &notes, "save-notes", None, true).await?;
    let original = saved.value["document"].clone();
    let (_, authored) = BlueprintRevisionDocument::from_json(&serde_json::to_vec(&original)?)?;
    for node in authored.semantic().nodes().values() {
        if let NodeKind::Task { config } = node.kind() {
            assert!(
                CapabilityAuthorityScope::requirement_envelope(config.requirement())?
                    .is_subset_of(&editor_scope()?),
                "authored task {} exceeds the documented grant",
                node.id()
            );
        }
    }
    let semantic = &original["revision"]["semantic"];
    assert_eq!(semantic["interface"]["inputs"]["brief"]["required"], true);
    assert_eq!(
        semantic["nodes"]["review"]["data_inputs"]["brief"]["binding"],
        json!({"type":"workflow_input","field":"brief"})
    );
    assert_eq!(
        semantic["nodes"]["review"]["data_inputs"]["draft"]["binding"],
        json!({"type":"node_output","node":"draft","port":"final_text","path":[]})
    );
    assert_eq!(
        semantic["nodes"]["review"]["kind"]["config"]["context_policy"]["ancestor_depth"],
        serde_json::Value::Null
    );
    assert_eq!(
        semantic["nodes"]["author.review.accept"]["data_inputs"]["milkdrift.acceptance"]["binding"]
            ["value"]["requirement"],
        json!({"type":"model_prose"})
    );
    assert_eq!(
        semantic["nodes"]["author.review.hold"]["kind"]["type"],
        "signal_wait"
    );
    let control = semantic["edges"]
        .as_object()
        .ok_or("edges")?
        .values()
        .filter(|edge| edge["kind"] == "control")
        .map(|edge| {
            (
                edge["source_node"].as_str().unwrap_or_default(),
                edge["source_port"].as_str().unwrap_or_default(),
                edge["target_node"].as_str().unwrap_or_default(),
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert!(control.contains(&("draft", "out", "author.draft.accept")));
    assert!(control.contains(&("author.draft.accept", "out", "author.draft.gate")));
    assert!(control.contains(&("author.draft.gate", "pass", "review")));
    assert!(control.contains(&("author.review.gate", "pass", "author.done")));
    assert!(control.contains(&("author.review.gate", "fail", "author.review.hold")));
    assert!(control.contains(&("author.review.hold", "out", "author.review.failed")));
    assert_eq!(
        control
            .iter()
            .filter(|(_, _, target)| *target == "author.done")
            .count(),
        1
    );
    let saved_draft = draft(&saved)?;
    assert!(
        author(
            &daemon.client,
            &saved_draft,
            "empty-prompt",
            Some(BlueprintEdit::Prompt {
                step: "review".into(),
                prompt: "  \n".into()
            }),
            true
        )
        .await
        .is_err()
    );
    let id = saved_draft
        .base_revision
        .as_deref()
        .ok_or("saved revision")?;
    assert_eq!(
        daemon.client.revision(id).await?.document,
        Some(original.clone())
    );
    let unchanged = author(&daemon.client, &saved_draft, "save-unchanged", None, true).await?;
    assert_eq!(unchanged.value["document"], original);
    let replay = author(&daemon.client, &notes, "save-notes", None, true).await?;
    assert!(replay.replayed);
    assert_eq!(saved.value, replay.value);
    let mut stale = request(
        "stale",
        None,
        Command::AuthorBlueprint {
            draft: saved_draft.clone(),
            edit: None,
            save: true,
        },
    );
    assert!(matches!(
        daemon.client.submit(&stale).await,
        Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict
    ));
    stale.command_id = "wrong-workflow".into();
    stale.expected_revision = saved_draft.base_revision.clone();
    if let Command::AuthorBlueprint { draft, .. } = &mut stale.command {
        draft.workflow_id = "other".into();
    }
    assert!(daemon.client.submit(&stale).await.is_err());
    let edited = author(
        &daemon.client,
        &saved_draft,
        "edit-prompt",
        Some(BlueprintEdit::Prompt {
            step: "review".into(),
            prompt: "Check unsupported claims and missing caveats. Return revised notes.".into(),
        }),
        true,
    )
    .await?;
    assert_ne!(edited.value["revision_id"], saved.value["revision_id"]);
    assert_eq!(edited.value["document"]["revision"]["parents"], json!([id]));
    assert_eq!(
        daemon.client.revision(id).await?.document,
        Some(original.clone())
    );
    let mut meeting = draft(
        &author(
            &daemon.client,
            &initial("meeting-summary"),
            "new-meeting",
            Some(add("summarize")),
            false,
        )
        .await?,
    )?;
    meeting = draft(
        &author(
            &daemon.client,
            &meeting,
            "meeting-output",
            Some(BlueprintEdit::Output {
                step: "summarize".into(),
                name: "summary".into(),
            }),
            true,
        )
        .await?,
    )?;
    assert_ne!(meeting.base_revision, saved_draft.base_revision);
    // An authorized expert can build a richer immutable definition. The narrow editor refuses it.
    let mutations = vec![
        json!({"type":"set_metadata","metadata":{"name":"Rich notes","description":"preserve this","labels":[],"extensions":{}}}),
    ];
    let rich_draft = BlueprintDraft {
        mutations,
        ..saved_draft.clone()
    };
    let mut rich = request(
        "rich-definition",
        None,
        Command::ConstructBlueprint {
            draft: rich_draft,
            store: true,
        },
    );
    rich.expected_revision = saved_draft.base_revision.clone();
    let rich = daemon.client.submit(&rich).await?;
    let rich_draft = draft(&rich)?;
    assert!(
        author(
            &daemon.client,
            &rich_draft,
            "unsupported",
            Some(BlueprintEdit::Rename {
                name: "Lossy".into()
            }),
            true
        )
        .await
        .is_err()
    );
    assert_eq!(
        daemon
            .client
            .revision(rich_draft.base_revision.as_deref().ok_or("rich id")?)
            .await?
            .document,
        Some(rich.value["document"].clone())
    );
    assert!(
        daemon
            .client
            .runs(
                None,
                None,
                &PageRequest {
                    limit: 20,
                    cursor: None
                }
            )
            .await?
            .items
            .is_empty()
    );
    assert_eq!(
        listener
            .accept()
            .err()
            .ok_or("unexpected model connection")?
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    daemon.stop().await?;
    let reopened = start(plan, CONTROLLER_TOKEN).await?;
    assert_eq!(reopened.client.revision(id).await?.document, Some(original));
    assert_eq!(
        author(&reopened.client, &notes, "save-notes", None, true)
            .await?
            .value,
        saved.value
    );
    assert_eq!(
        listener
            .accept()
            .err()
            .ok_or("unexpected model connection")?
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    reopened.stop().await?;
    Ok(())
}
