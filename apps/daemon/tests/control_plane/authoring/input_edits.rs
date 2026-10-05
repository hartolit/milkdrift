use super::super::inputs::{ModelFixture, start_request, upload, workflow};
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn input_rename_and_removal_preserve_runs_replay_and_exact_bindings() -> TestResult {
    let directory = TempDir::new()?;
    let model = ModelFixture::start().await?;
    model.checked(async |model| {
        // This case overlaps three immutable pins; exact-entry saturation has separate tests.
        let plan = model_configuration_with_capacity(&directory, model.address, 3)?.validate(directory.path())?;
        let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
        let mut retained = None;
        daemon.checked(model, async |daemon| {
            let source = workflow(&daemon.client).await?;
            let opened = BlueprintDraft { workflow_id: "release-notes".into(), base_revision: Some(source), mutations: vec![] };
            let typo = author(&daemon.client, &opened, "save-typo", Some(BlueprintEdit::RenameInput { name: "brief".into(), new_name: "breif".into() }), true).await?;
            let old_draft = draft(&typo)?;
            let old_id = old_draft.base_revision.as_deref().ok_or("saved base absent")?;
            let mut input = upload(&daemon.client, "retained-brief", b"Harbor Host 1.4: add preview environments.").await?;
            input.name = "breif".into();
            let old_start = start_request("old-input-run", old_id, vec![input.clone()]);
            daemon.client.submit(&old_start).await?;
            let old_document = daemon.client.revision(old_id).await?;
            let with_collision = draft(&author(&daemon.client, &old_draft, "add-collision", Some(BlueprintEdit::Input { name: "occupied".into() }), false).await?)?;
            let unchanged = author(&daemon.client, &with_collision, "before-refusals", None, false).await?;
            for (id, edit) in [
                ("collision", BlueprintEdit::RenameInput { name: "breif".into(), new_name: "occupied".into() }),
                ("invalid", BlueprintEdit::RenameInput { name: "breif".into(), new_name: "invalid name".into() }),
                ("absent", BlueprintEdit::RenameInput { name: "missing".into(), new_name: "new".into() }),
                ("absent-same", BlueprintEdit::RenameInput { name: "missing".into(), new_name: "missing".into() }),
                ("remove-absent", BlueprintEdit::RemoveInput { name: "missing".into() }),
            ] {
                assert!(author(&daemon.client, &with_collision, id, Some(edit), false).await.is_err());
            }
            let after = author(&daemon.client, &with_collision, "after-refusals", None, false).await?;
            assert_eq!(after.value, unchanged.value);
            let same = author(&daemon.client, &with_collision, "same-name", Some(BlueprintEdit::RenameInput { name: "breif".into(), new_name: "breif".into() }), false).await?;
            assert_eq!(same.value, unchanged.value);
            let pending = draft(&author(&daemon.client, &with_collision, "remove-unused", Some(BlueprintEdit::RemoveInput { name: "occupied".into() }), false).await?)?;
            let corrected = author(&daemon.client, &pending, "correct-input", Some(BlueprintEdit::RenameInput { name: "breif".into(), new_name: "brief".into() }), false).await?;
            let view = corrected.value.get("workflow").ok_or("view absent")?;
            assert_eq!(view.get("inputs"), Some(&json!(["brief"])));
            for index in [0, 1] {
                assert_eq!(view.pointer(&format!("/steps/{index}/inputs/brief")), Some(&json!({"type":"workflow_input", "field":"brief"})));
                assert_eq!(view.pointer(&format!("/steps/{index}/prompt")), typo.value.pointer(&format!("/workflow/steps/{index}/prompt")));
            }
            assert_eq!(view.pointer("/steps/1/inputs/draft"), typo.value.pointer("/workflow/steps/1/inputs/draft"));
            assert_eq!(view.get("output"), typo.value.pointer("/workflow/output"));
            let pending = draft(&corrected)?;
            let saved = author(&daemon.client, &pending, "save-corrected", None, true).await?;
            assert!(author(&daemon.client, &pending, "save-corrected", None, true).await?.replayed);
            assert!(matches!(author(&daemon.client, &pending, "save-corrected", Some(BlueprintEdit::Rename { name: "conflict".into() }), true).await, Err(ClientError::Api(error)) if error.code == ErrorCode::Conflict));
            let corrected_draft = draft(&saved)?;
            let corrected_id = corrected_draft.base_revision.as_deref().ok_or("base absent")?;
            input.name = "brief".into();
            let corrected_start = start_request("corrected-input-run", corrected_id, vec![input.clone()]);
            daemon.client.submit(&corrected_start).await?;
            let removal = author(&daemon.client, &corrected_draft, "remove-connected", Some(BlueprintEdit::RemoveInput { name: "brief".into() }), false).await.err().ok_or("connected removal accepted")?;
            let ClientError::Api(removal) = removal else { return Err("expected public refusal".into()); };
            assert!(removal.message.contains("draft/brief"), "{removal:?}");
            assert!(removal.message.contains("review/brief"), "{removal:?}");
            let mut unused = corrected_draft.clone();
            for step in ["draft", "review"] {
                unused = draft(&author(&daemon.client, &unused, &format!("disconnect-{step}"), Some(BlueprintEdit::Disconnect { step: step.into(), input: "brief".into() }), false).await?)?;
            }
            let removed = author(&daemon.client, &unused, "save-removal", Some(BlueprintEdit::RemoveInput { name: "brief".into() }), true).await?;
            let removed_draft = draft(&removed)?;
            let removed_id = removed_draft.base_revision.as_deref().ok_or("removed base absent")?;
            assert!(daemon.client.revision(removed_id).await?.inputs.is_empty());
            assert_eq!(author(&daemon.client, &removed_draft, "reopen-removal", None, false).await?.value.get("workflow"), removed.value.get("workflow"));
            daemon.client.submit(&start_request("no-input-run", removed_id, vec![])).await?;
            for (run, pinned) in [("old-input-run", old_id), ("corrected-input-run", corrected_id), ("no-input-run", removed_id)] {
                let state = wait_for_run(&daemon.client, run, Duration::from_secs(45), |read| read.terminal.is_some()).await?;
                assert_eq!(state.terminal.as_deref(), Some("succeeded"));
                assert_eq!(state.revision_id.as_deref(), Some(pinned));
            }
            assert_eq!(daemon.client.revision(old_id).await?, old_document);
            assert!(daemon.client.artifact_metadata(&input.artifact_id).await.is_ok());
            assert_eq!(model.request_count(), Some(6));
            retained = Some((old_start, corrected_start, pending, removed_id.to_owned()));
            Ok(())
        }).await?;
        let (old_start, corrected_start, pending, removed_id) = retained.ok_or("results absent")?;
        let daemon = start(plan, CONTROLLER_TOKEN).await?;
        daemon.checked(model, async |daemon| {
            assert!(daemon.client.submit(&old_start).await?.replayed);
            assert!(daemon.client.submit(&corrected_start).await?.replayed);
            assert!(author(&daemon.client, &pending, "save-corrected", None, true).await?.replayed);
            assert!(daemon.client.revision(&removed_id).await?.inputs.is_empty());
            assert_eq!(model.request_count(), Some(6));
            Ok(())
        }).await
    }).await
}
