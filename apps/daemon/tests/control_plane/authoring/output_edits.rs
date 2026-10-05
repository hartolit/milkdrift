use super::super::inputs::ModelFixture;
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incomplete_draft_reopens_and_output_limit_reaches_request_and_admission() -> TestResult {
    let directory = TempDir::new()?;
    let model = ModelFixture::start().await?;
    model.checked(async |model| {
        let config = model_configuration_document(&directory, model.address)?;
        let path = directory.path().join("model.json");
        let mut profile: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        *profile.get_mut("billing").ok_or("billing absent")? = json!({"type":"unbilled", "source":"controlled operator fixture"});
        *profile.get_mut("token_limits").ok_or("token limits absent")? = json!({"type":"byte_bpe", "template_tokens_per_message":32, "template_tokens_per_request":64, "maximum_input_tokens":100000, "maximum_output_tokens":1024, "output_control":"max_tokens", "source":"controlled bounded text fixture"});
        fs::write(&path, serde_json::to_vec(&profile)?)?;
        let plan = config.validate(directory.path())?;
        let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
        let mut retained = None;
        daemon.checked(model, async |daemon| {
            let added = draft(&author(&daemon.client, &initial("output-edit"), "add-only", Some(add("only")), false).await?)?;
            let saved = author(&daemon.client, &added, "save-only", Some(BlueprintEdit::Output { step: "only".into(), name: "result".into() }), true).await?;
            let original = draft(&saved)?;
            let original_id = original.base_revision.as_deref().ok_or("base absent")?;
            assert!(author(&daemon.client, &original, "remove-selected", Some(BlueprintEdit::Remove { step: "only".into() }), false).await.is_err());
            for (id, units) in [("zero", 0), ("too-large", u64::MAX)] {
                assert!(author(&daemon.client, &original, id, Some(BlueprintEdit::OutputLimit { step: "only".into(), maximum_output_units: units }), false).await.is_err());
            }
            // Definition access permits an edit with retained hidden selections;
            // saving and execution still require their own operation authority.
            let observer = client(&daemon.endpoint, OBSERVER_TOKEN)?;
            let hidden_edit = author(&observer, &original, "retained-limit", Some(BlueprintEdit::OutputLimit { step: "only".into(), maximum_output_units: 256 }), false).await?;
            assert_eq!(hidden_edit.value.pointer("/workflow/selections_resolved"), Some(&json!(false)));
            let hidden_draft = draft(&hidden_edit)?;
            assert!(author(&observer, &hidden_draft, "denied-save", None, true).await.is_err());
            assert!(observer.submit(&request("denied-edited-start", Some(0), Command::StartRun {
                run_id: "denied-edited-start".into(), workflow_id: "output-edit".into(),
                revision_id: original_id.into(), inputs: vec![],
            })).await.is_err());
            assert!(daemon.client.run("denied-edited-start").await.is_err());
            let adjusted = author(&daemon.client, &original, "adjust-only", Some(BlueprintEdit::OutputLimit { step: "only".into(), maximum_output_units: 256 }), false).await?;
            let mut expected = saved.value.get("workflow").ok_or("view absent")?.clone();
            *expected.pointer_mut("/steps/0/maximum_output_units").ok_or("limit absent")? = json!(256);
            assert_eq!(adjusted.value.get("workflow"), Some(&expected));
            let (_, revision) = BlueprintRevisionDocument::from_json(&serde_json::to_vec(adjusted.value.get("document").ok_or("document absent")?)?)?;
            let task = revision.semantic().nodes().get(&NodeId::new("only")?).ok_or("model step absent")?;
            let Some(milkdrift_blueprint::BindingSource::Literal { value }) = task.data_inputs().get(&PortId::new(milkdrift_model::MODEL_TASK_INPUT_NAME)?).and_then(milkdrift_blueprint::DataPort::binding) else { return Err("request absent".into()); };
            let request = milkdrift_model::ModelTaskRequestDocument::from_json(&serde_json::to_vec(value.value())?)?;
            assert_eq!(request.body().maximum_output_units(), 256);
            let cleared = author(&daemon.client, &draft(&adjusted)?, "clear-only", Some(BlueprintEdit::ClearOutput {}), false).await?;
            assert_eq!(cleared.value.pointer("/workflow/output"), Some(&serde_json::Value::Null));
            assert!(author(&daemon.client, &draft(&cleared)?, "save-no-output", None, true).await.is_err());
            let removed = author(&daemon.client, &draft(&cleared)?, "remove-only", Some(BlueprintEdit::Remove { step: "only".into() }), false).await?;
            let incomplete = draft(&removed)?;
            assert_eq!(removed.value.pointer("/workflow/steps"), Some(&json!([])));
            assert!(author(&daemon.client, &incomplete, "save-empty", None, true).await.is_err());
            assert_eq!(daemon.client.revision(original_id).await?.document, saved.value.get("document").cloned());
            retained = Some((incomplete, removed.value.get("workflow").cloned(), original_id.to_owned()));
            assert_eq!(model.request_count(), Some(0));
            Ok(())
        }).await?;
        let (incomplete, view, original) = retained.ok_or("draft absent")?;
        let daemon = start(plan, CONTROLLER_TOKEN).await?;
        daemon.checked(model, async |daemon| {
            assert_eq!(author(&daemon.client, &incomplete, "reopen-incomplete", None, false).await?.value.get("workflow").cloned(), view);
            let replacement = draft(&author(&daemon.client, &incomplete, "replacement", Some(add("replacement")), false).await?)?;
            let adjusted = draft(&author(&daemon.client, &replacement, "replacement-limit", Some(BlueprintEdit::OutputLimit { step: "replacement".into(), maximum_output_units: 256 }), false).await?)?;
            let saved = author(&daemon.client, &adjusted, "save-replacement", Some(BlueprintEdit::Output { step: "replacement".into(), name: "result".into() }), true).await?;
            let final_draft = draft(&saved)?;
            let final_id = final_draft.base_revision.as_deref().ok_or("base absent")?;
            assert_eq!(author(&daemon.client, &final_draft, "open-replacement", None, false).await?.value.get("workflow"), saved.value.get("workflow"));
            let start = |id: &str, revision: &str| request(id, Some(0), Command::StartRun { run_id: id.into(), workflow_id: "output-edit".into(), revision_id: revision.into(), inputs: vec![] });
            daemon.client.submit(&start("bounded-run", final_id)).await?;
            let run = wait_for_run(&daemon.client, "bounded-run", Duration::from_secs(45), |read| read.terminal.is_some()).await?;
            assert_eq!(run.terminal.as_deref(), Some("succeeded"));
            assert_eq!(model.request_count(), Some(1));
            assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.first().and_then(|body| body.get("max_tokens")), Some(&json!(256)));
            let attempt = attempt_id_for_node(&daemon.client, "bounded-run", "replacement").await?;
            let read = daemon.client.attempt("bounded-run", &attempt).await?;
            let response = read.outputs.iter().find(|output| output.name == "model_response").ok_or("response output absent")?;
            let bytes = daemon.client.artifact_range(&response.artifact.artifact_id, 0, response.artifact.size - 1).await?.bytes;
            let response: serde_json::Value = serde_json::from_slice(&bytes)?;
            let envelope: milkdrift_capability::InvocationAdmissionEnvelope = serde_json::from_value(response.pointer("/response/provider_metadata/org.milkdrift~1model-accounting/reservation_envelope").ok_or("reservation facts absent")?.clone())?;
            assert_eq!(envelope.output_units(), &milkdrift_capability::AdmissionBound::Bounded(256));
            assert_eq!(read.model_generation.ok_or("generation absent")?.requested_output_units, Some(256));
            assert_eq!(read.usage.ok_or("usage absent")?.output_units, Some(10));
            // Valid model-unit syntax may still exceed this endpoint's admitted envelope.
            // Preparation must refuse before HTTP rather than reserve or send the old limit.
            let excess = author(&daemon.client, &final_draft, "excess-limit", Some(BlueprintEdit::OutputLimit { step: "replacement".into(), maximum_output_units: 2048 }), true).await?;
            daemon.client.submit(&start("over-profile-run", draft(&excess)?.base_revision.as_deref().ok_or("base absent")?)).await?;
            let failed = wait_for_run(&daemon.client, "over-profile-run", Duration::from_secs(45), |read| read.terminal.is_some()).await?;
            assert_eq!(failed.terminal.as_deref(), Some("failed"));
            assert_eq!(model.request_count(), Some(1));
            assert_eq!(daemon.client.revision(&original).await?.summary.parents.len(), 0);
            Ok(())
        }).await
    }).await
}
