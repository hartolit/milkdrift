//! Interrupt the composed start at its real durable command boundaries.
use super::*;
use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, BlueprintRevisionDocument, Mutation, MutationBatch, Node, NodeId,
    NodeKind, TerminalOutcome,
};
use milkdrift_control_protocol::{Command, ProtocolVersion};
use milkdrift_persistence::{EventPageQuery, PageSize, RunEventKind, RunQueryStore};

#[tokio::test]
async fn interrupted_create_and_start_recover_exact_internal_commands()
-> Result<(), Box<dyn std::error::Error>> {
    for stage in 0..3 {
        let directory = tempfile::tempdir()?;
        let token = directory.path().join("token");
        std::fs::write(&token, "start-boundaries-token")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600))?;
        }
        let plan = crate::host::tests::owner_test_config(directory.path(), &token, 32)?;
        let host = crate::DaemonHost::start(plan.clone())?;
        let session = host
            .authenticate_header(Some("Bearer start-boundaries-token"))
            .ok_or("session absent")?;
        let definition = BlueprintRevision::genesis(
            WorkflowId::new("recovery")?,
            MutationBatch::new(vec![Mutation::AddNode {
                node: Node::new(
                    NodeId::new("done")?,
                    NodeKind::Terminal {
                        outcome: TerminalOutcome::Success,
                    },
                )?,
            }])?,
            AuthorRef::new(session.actor.as_str())?,
            "recovery test",
        )?;
        let command = CommandRequest {
            protocol: ProtocolVersion::CURRENT,
            command_id: "start-exact".into(),
            expected_sequence: Some(0),
            expected_revision: Some(definition.id().to_string()),
            reason: "start once".into(),
            evidence: vec![],
            command: Command::StartRun {
                run_id: "recovery-run".into(),
                workflow_id: "recovery".into(),
                revision_id: definition.id().to_string(),
                inputs: vec![],
            },
        };
        let mut import = command.clone();
        import.command_id = "import".into();
        import.expected_revision = None;
        import.expected_sequence = None;
        import.command = Command::ImportBlueprint {
            document: serde_json::from_slice(
                &BlueprintRevisionDocument::new(&definition).to_canonical_json()?,
            )?,
        };
        host.command(session.clone(), import)
            .await
            .map_err(|error| error.message)?;
        if stage > 0 {
            let request = command.clone();
            let caller = session.clone();
            host.dispatch(false, move |owner| {
                if stage == 1 {
                    create(
                        owner,
                        &caller,
                        &request,
                        "recovery-run",
                        "recovery",
                        request
                            .expected_revision
                            .as_deref()
                            .ok_or_else(|| invalid("revision"))?,
                        &[],
                    )
                    .map(|_| ())
                } else {
                    owner.execute_new_command(&caller, &request).map(|_| ())
                }
            })
            .await
            .map_err(|error| error.message)?;
        }
        host.shutdown().await?;
        drop(host);
        let restarted = crate::DaemonHost::start(plan)?;
        let session = restarted
            .authenticate_header(Some("Bearer start-boundaries-token"))
            .ok_or("session absent")?;
        restarted
            .command(session.clone(), command.clone())
            .await
            .map_err(|error| error.message)?;
        assert!(
            restarted
                .command(session, command)
                .await
                .map_err(|error| error.message)?
                .replayed
        );
        restarted.shutdown().await?;
        drop(restarted);
        let store = milkdrift_redb_store::RedbStore::open(directory.path().join("data"))?;
        let events = store
            .events(&EventPageQuery::new(
                RunId::new("recovery-run")?,
                None,
                PageSize::new(100)?,
            )?)?
            .events;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::RunCreated { .. }))
                .count(),
            1,
            "stage {stage}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::RunStarted))
                .count(),
            1,
            "stage {stage}"
        );
    }
    Ok(())
}
