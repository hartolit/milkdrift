//! Actual-binary observations submit one planner and let the daemon admit every descendant.

use super::{Daemon, ready, start};
use milkdrift_control_protocol::tasks::{
    CreateWorkflowRequest, InputBinding, TaskReadDto, TaskStateDto,
};
use milkdrift_evidence::{
    EvidenceResult,
    application::{ensure, write_private},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

pub(super) async fn run(
    root: &Path,
    daemon_binary: &Path,
    instance: &mut Daemon,
) -> EvidenceResult<Value> {
    let client = super::check::client(instance)?;
    let project = super::projects::attach(
        &client,
        &root.join("daemon-a/animation-project"),
        "ci-attach-animation",
    )
    .await?;
    let request: CreateWorkflowRequest = serde_json::from_value(json!({
        "request_id":"ci-animation","project_id":project.id,"title":"Animation task evolution","description":"One planner produces all parallel and corrective future work",
        "tasks":[{"alias":"planner","definition":{
            "title":"Plan animation research","description":"Plan research from the separately supplied animation brief",
            "instructions":"Plan a Rust learned-animation investigation using explicit research and composition dependencies.",
            "capability":{"id":"controlled-planner","operation":"model.generate"},
            "execution":{"kind":"explicit","expected_generation":project.generation,"settings":{"environment":{"kind":"host"},"allow_changes":false},"access":"none","tools":[]},
            "inputs":{"brief":{"kind":"supplied","name":"brief","optional":false}},
            "outputs":{"model_response":{"required":true,"media_type":"application/vnd.milkdrift.model-response.v1+json","max_bytes":2097152}},
            "dependencies":[],"required":true,"corrective_of":null,"paths":[],"tools":[],
            "actions":{"automatic_repair":true}
        }}],
        "inputs":{"brief":{"kind":"text","text":"Investigate learned animation in Rust: motion representation, training data, and render validation. Preserve failed research and correct its future consumers."}},
        "finish_when_settled":false,"limits":null
    }))?;
    let receipt = client
        .create_workflow(&request)
        .await
        .map_err(|error| format!("animation planner submission failed: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut longest_observation_ms = 0;
    let workflow = loop {
        let observation = Instant::now();
        let current = client
            .workflow(&receipt.workflow_id)
            .await
            .map_err(|error| {
                format!(
                    "animation workflow observation failed after {} ms: {error}",
                    observation.elapsed().as_millis()
                )
            })?;
        longest_observation_ms = longest_observation_ms.max(observation.elapsed().as_millis());
        if current.state == "completed" {
            break current;
        }
        if Instant::now() >= deadline || current.wait_question.is_some() {
            write_private(
                &root.join("animation-incomplete.json"),
                &serde_json::to_vec_pretty(&current)?,
            )?;
            return Err("automatic animation journey did not complete; inspect private animation-incomplete.json".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    ensure(
        workflow.task_count == 8 && workflow.tasks.len() == 8,
        "automatic model batches lost or duplicated a task",
    )?;
    ensure(
        workflow.admitted_model_batches == 3
            && workflow.admissions.len() == 3
            && workflow
                .admissions
                .iter()
                .all(|admission| admission.status == "accepted"),
        "the three model action batches lack durable accepted linkage",
    )?;
    let mut tasks = Vec::new();
    for task in &workflow.tasks {
        tasks.push(
            client
                .task(&workflow.id, &task.id)
                .await
                .map_err(|error| format!("animation retained task read failed: {error}"))?,
        );
    }
    ensure(
        tasks
            .iter()
            .filter(|task| task.definition.actions.is_some())
            .all(|task| {
                task.definition
                    .outputs
                    .get("model_response")
                    .is_some_and(|output| {
                        output.required
                            && output.media_type.as_deref()
                                == Some("application/vnd.milkdrift.model-response.v1+json")
                    })
                    && !task.definition.outputs.contains_key("final_text")
            }),
        "action tasks must require the canonical structured model response",
    )?;
    let render = task(&tasks, "Research render validation")?;
    let motion = task(&tasks, "Research motion representation")?;
    let training = task(&tasks, "Research training data")?;
    let compose = task(&tasks, "Animation composition")?;
    let correction = task(&tasks, "Corrected render research")?;
    let final_task = task(&tasks, "Final animation report")?;
    ensure(
        render.state == TaskStateDto::Failed
            && tasks
                .iter()
                .filter(|task| task.state == TaskStateDto::Completed)
                .count()
                == 7,
        "failed research history was rewritten or a required correction never completed",
    )?;
    ensure(
        overlap(motion, training)?,
        "research tasks have no overlapping actual start/end evidence",
    )?;
    ensure(
        compose
            .frozen
            .as_ref()
            .and_then(|start| start.inputs.get("failed_render"))
            .and_then(|input| input.text.as_deref())
            .is_some_and(|text| text.contains("failed")),
        "compositor did not receive explicit failed settled outcome",
    )?;
    ensure(
        matches!(final_task.definition.inputs.get("render"), Some(InputBinding::TaskOutput { task, optional: false, .. }) if task == &correction.id),
        "adoption did not rebind the required-success future consumer",
    )?;
    let adopted_input = final_task
        .frozen
        .as_ref()
        .and_then(|start| start.inputs.get("render"))
        .and_then(|input| input.artifact_id.as_ref())
        .ok_or("final consumer omitted frozen correction artifact")?;
    ensure(
        correction
            .result
            .as_ref()
            .is_some_and(|result| result.artifacts.contains(adopted_input)),
        "final consumer did not freeze the successful correction's immutable output",
    )?;
    ensure(
        final_task.version == 2,
        "model adoption did not retain the future definition change",
    )?;
    ensure(
        client
            .task_result(&workflow.id, &final_task.id)
            .await
            .map_err(|error| format!("animation final result read failed: {error}"))?
            .text
            .contains("Final animation report"),
        "final result was unavailable through the public client",
    )?;
    for folder in ["motion", "training", "render"] {
        let bytes = fs::read(
            root.join("daemon-a/animation-project")
                .join(folder)
                .join("research.txt"),
        )?;
        ensure(
            !bytes.is_empty(),
            "native model tools did not produce their scoped research file",
        )?;
    }
    let before_requests = request_count(root)?;
    drop(client);
    instance.child.terminate()?;
    instance.child = start(daemon_binary, &instance.config, &instance.log)?;
    ready(instance).await?;
    let reopened = super::check::client(instance)?;
    let recovered = reopened
        .workflow(&workflow.id)
        .await
        .map_err(|error| format!("animation workflow read after restart failed: {error}"))?;
    ensure(
        recovered.tasks == workflow.tasks && recovered.admissions == workflow.admissions,
        "restart changed accepted descendants or response admission linkage",
    )?;
    ensure(
        reopened
            .create_workflow(&request)
            .await
            .map_err(|error| format!("animation planner replay after restart failed: {error}"))?
            == receipt,
        "exact initial planner replay changed its durable acceptance",
    )?;
    ensure(
        request_count(root)? == before_requests,
        "restart or replay duplicated an external model invocation",
    )?;
    write_private(
        &root.join("animation-workflow.json"),
        &serde_json::to_vec_pretty(&recovered)?,
    )?;
    write_private(
        &root.join("animation-tasks.json"),
        &serde_json::to_vec_pretty(&tasks)?,
    )?;
    Ok(json!({
        "outcome":"passed","workflow":workflow.id,"initial_client_tasks":1,"retained_tasks":8,
        "accepted_model_batches":3,"client_continuation_batches":0,"parallel_start_end_evidence":true,
        "failed_outcome_composition":true,"correction_rebinds_required_input":true,
        "native_read_write_tools":true,"restart_preserves_accepted_batches":true,
        "canonical_action_outputs":true,"longest_workflow_observation_ms":longest_observation_ms,
        "evidence_scope":"Controlled responses establish daemon construction and bounded tool execution, not real-model quality"
    }))
}

fn task<'a>(tasks: &'a [TaskReadDto], title: &str) -> EvidenceResult<&'a TaskReadDto> {
    tasks
        .iter()
        .find(|task| task.definition.title == title)
        .ok_or_else(|| format!("animation task {title} absent").into())
}

fn overlap(left: &TaskReadDto, right: &TaskReadDto) -> EvidenceResult<bool> {
    Ok(left.started_at.ok_or("research start absent")?
        < right.ended_at.ok_or("research end absent")?
        && right.started_at.ok_or("research start absent")?
            < left.ended_at.ok_or("research end absent")?)
}

fn request_count(root: &Path) -> EvidenceResult<usize> {
    fs::read_dir(root.join("controlled-model"))?.try_fold(0usize, |count, entry| {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with("request-") {
            count
                .checked_add(1)
                .ok_or_else(|| "fixture request count overflow".into())
        } else {
            Ok(count)
        }
    })
}
