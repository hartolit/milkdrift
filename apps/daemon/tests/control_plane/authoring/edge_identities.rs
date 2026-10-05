use super::super::{
    inputs,
    results::{Responses, prose},
};
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delimiter_names_keep_distinct_connections_through_save_reopen_and_execution() -> TestResult
{
    let directory = tempfile::tempdir()?;
    let model = Responses::start(vec![
        prose("DRAFT ROUTING SENTINEL", "stop"),
        prose("FIRST REVIEW ONLY", "stop"),
        prose("FINAL REVIEW", "stop"),
    ])
    .await?;
    let plan = model_configuration(&directory, model.address)?;
    let daemon = start(plan.clone(), CONTROLLER_TOKEN).await?;
    let mut pending = initial("release-notes");
    for (index, edit) in [
        add("draft"),
        add("review"),
        add("review:x"),
        BlueprintEdit::Connect {
            step: "review".into(),
            input: "x:brief".into(),
            source: ModelInputSource::Step {
                step: "draft".into(),
            },
        },
        BlueprintEdit::Connect {
            step: "review:x".into(),
            input: "brief".into(),
            source: ModelInputSource::Step {
                step: "draft".into(),
            },
        },
        BlueprintEdit::Output {
            step: "review:x".into(),
            name: "notes".into(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        pending = draft(
            &author(
                &daemon.client,
                &pending,
                &format!("collision-edit-{index}"),
                Some(edit),
                false,
            )
            .await?,
        )?;
    }
    let saved = author(&daemon.client, &pending, "collision-save", None, true).await?;
    let saved_draft = draft(&saved)?;
    let revision = saved_draft
        .base_revision
        .as_deref()
        .ok_or("saved revision")?;
    let document = saved.value.get("document").ok_or("document")?;
    let (_, definition) = BlueprintRevisionDocument::from_json(&serde_json::to_vec(document)?)?;
    let edges: Vec<_> = definition
        .semantic()
        .edges()
        .values()
        .filter(|edge| {
            edge.kind() == EdgeKind::Data
                && edge.source_node().as_str() == "draft"
                && edge.source_port().as_str() == "final_text"
        })
        .collect();
    assert_eq!(edges.len(), 2);
    let endpoints: std::collections::BTreeSet<_> = edges
        .iter()
        .map(|edge| (edge.target_node().as_str(), edge.target_port().as_str()))
        .collect();
    assert_eq!(
        endpoints,
        std::collections::BTreeSet::from([("review", "x:brief"), ("review:x", "brief")])
    );
    assert_ne!(
        edges.first().ok_or("first edge")?.id(),
        edges.get(1).ok_or("second edge")?.id()
    );
    daemon.stop().await?;
    let daemon = start(plan, CONTROLLER_TOKEN).await?;
    let opened = author(&daemon.client, &saved_draft, "collision-open", None, false).await?;
    assert_eq!(opened.value.get("document"), Some(document));
    assert!(draft(&opened)?.mutations.is_empty());
    daemon
        .client
        .submit(&inputs::start_request("collision-run", revision, vec![]))
        .await?;
    let result = wait_for_run(
        &daemon.client,
        "collision-run",
        Duration::from_secs(45),
        |run| run.terminal.is_some(),
    )
    .await?;
    assert_eq!(
        result.terminal.as_deref(),
        Some("succeeded"),
        "{}",
        serde_json::to_string_pretty(&daemon.client.run_result("collision-run").await?)?
    );
    let requests = model.requests.lock().map_err(|_| "fixture lock")?.clone();
    assert_eq!(requests.len(), 3);
    for (index, port) in [(1, "x:brief"), (2, "brief")] {
        let text = requests.get(index).ok_or("review request")?.to_string();
        assert!(text.contains("DRAFT ROUTING SENTINEL"), "{text}");
        assert!(text.contains(port), "{text}");
        assert!(!text.contains("FIRST REVIEW ONLY"), "{text}");
    }
    daemon.stop().await
}
