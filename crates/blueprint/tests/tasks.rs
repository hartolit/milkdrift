//! Ordinary task definitions reject ambiguous references before durable admission.
use milkdrift_blueprint::{
    TaskDefinition, TaskId, TaskInputBinding, TaskInputValue, WorkflowLimits, validate_task_set,
};
use serde_json::json;
use std::collections::BTreeMap;
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn task() -> Result<TaskDefinition, Box<dyn std::error::Error>> {
    let capability = milkdrift_capability::CapabilityRequirement::new(
        milkdrift_capability::OperationId::new("process.execute")?,
    );
    Ok(serde_json::from_value(
        json!({"title":"Inspect input", "instructions":"Read and summarize the explicit input", "capability":capability}),
    )?)
}

#[test]
fn defaults_and_missing_supplied_values_are_definition_facts_not_runtime_shortcuts() -> TestResult {
    let mut task = task()?;
    assert!(task.required);
    assert!(task.outputs.is_empty());
    task.inputs.insert(
        "brief".into(),
        TaskInputBinding::Supplied {
            name: "brief".into(),
            optional: false,
        },
    );
    task.validate()?;
    let limits = WorkflowLimits::default();
    assert_eq!(
        (
            limits.tasks,
            limits.concurrent_tasks,
            limits.model_batches,
            limits.tool_calls_per_task
        ),
        (256, 4, 32, 16)
    );
    Ok(())
}

#[test]
fn dependency_and_correction_cycles_are_refused_without_hiding_missing_inputs() -> TestResult {
    let a = TaskId::new("a")?;
    let b = TaskId::new("b")?;
    let mut consumer = task()?;
    consumer.inputs.insert(
        "source".into(),
        TaskInputBinding::TaskOutput {
            task: a.clone(),
            output: "report".into(),
            optional: false,
        },
    );
    consumer.validate()?;
    let mut definitions = BTreeMap::from([(a.clone(), task()?), (b.clone(), consumer)]);
    assert!(
        validate_task_set(&definitions, &WorkflowLimits::default(), |_, _| None).is_err(),
        "even an acyclic binding must name a declared output"
    );
    definitions
        .get_mut(&a)
        .ok_or("task absent")?
        .outputs
        .insert(
            "report".into(),
            milkdrift_blueprint::TaskOutputDeclaration {
                required: true,
                media_type: Some("text/plain".into()),
                max_bytes: 4096,
            },
        );
    validate_task_set(&definitions, &WorkflowLimits::default(), |_, _| None)?;
    definitions.get_mut(&a).ok_or("task absent")?.dependencies =
        milkdrift_blueprint::TaskDependencies::Ordered(BTreeMap::from([(
            b.clone(),
            milkdrift_blueprint::TaskOrderCondition::Success,
        )]));
    assert!(validate_task_set(&definitions, &WorkflowLimits::default(), |_, _| None).is_err());
    definitions.get_mut(&a).ok_or("task absent")?.dependencies = Default::default();
    definitions.get_mut(&a).ok_or("task absent")?.corrective_of = Some(b.clone());
    assert!(
        validate_task_set(&definitions, &WorkflowLimits::default(), |_, _| None).is_err(),
        "correction waiting and explicit dependency form one causal cycle"
    );
    definitions.get_mut(&b).ok_or("task absent")?.corrective_of = Some(a);
    assert!(validate_task_set(&definitions, &WorkflowLimits::default(), |_, _| None).is_err());
    Ok(())
}

#[test]
fn captured_bytes_have_verified_identity_and_output_contracts_have_finite_bounds() -> TestResult {
    let captured = TaskInputValue::text_file(
        "Cargo.toml".into(),
        "[package]\nname = 'actual-input'".into(),
    )?;
    let mut wire = serde_json::to_value(&captured)?;
    *wire.get_mut("text").ok_or("captured text absent")? = json!("changed bytes");
    assert!(
        serde_json::from_value::<TaskInputValue>(wire)?
            .validate()
            .is_err()
    );
    assert!(TaskInputValue::text_file("../Cargo.toml".into(), "x".into()).is_err());
    let mut value = serde_json::to_value(task()?)?;
    *value
        .get_mut("outputs")
        .ok_or("output declarations absent")? = json!({"report":{"max_bytes":2097153}});
    assert!(
        serde_json::from_value::<TaskDefinition>(value)?
            .validate()
            .is_err()
    );
    let mut definition = task()?;
    for name in ["report", "diagnostic"] {
        definition.outputs.insert(
            name.into(),
            milkdrift_blueprint::TaskOutputDeclaration {
                required: false,
                media_type: Some("text/plain".into()),
                max_bytes: milkdrift_blueprint::MAX_TASK_OUTPUT_BYTES,
            },
        );
    }
    definition.validate()?;
    definition.outputs.remove("diagnostic");
    assert_eq!(
        definition
            .outputs
            .get("report")
            .ok_or("report missing")?
            .max_bytes,
        milkdrift_blueprint::MAX_TASK_OUTPUT_BYTES
    );
    Ok(())
}

#[test]
fn retained_named_outputs_require_complete_bounded_evidence_and_keep_cycles_invalid() -> TestResult
{
    let producer = TaskId::new("producer")?;
    let consumer = TaskId::new("consumer")?;
    let mut definition = task()?;
    definition.inputs.insert(
        "result".into(),
        TaskInputBinding::TaskOutput {
            task: producer.clone(),
            output: "report".into(),
            optional: false,
        },
    );
    let mut definitions =
        BTreeMap::from([(producer.clone(), task()?), (consumer.clone(), definition)]);
    for (media, size, accepted) in [
        (Some("text/plain"), Some(2_097_152), true),
        (Some("text/plain"), Some(2_097_153), false),
        (Some("text/plain"), None, false),
        (None, Some(8), false),
    ] {
        let reference = milkdrift_capability::ArtifactReference::new(
            "retained-report",
            "0".repeat(64),
            media.map(str::to_owned),
            size,
        )?;
        let result = validate_task_set(&definitions, &WorkflowLimits::default(), |id, output| {
            (id == &producer && output == "report").then_some(&reference)
        });
        assert_eq!(result.is_ok(), accepted);
        assert!(
            validate_task_set(&definitions, &WorkflowLimits::default(), |id, output| {
                (id == &producer && output == "different-name").then_some(&reference)
            })
            .is_err()
        );
    }
    definitions
        .get_mut(&producer)
        .ok_or("producer absent")?
        .dependencies = milkdrift_blueprint::TaskDependencies::Ordered(BTreeMap::from([(
        consumer,
        milkdrift_blueprint::TaskOrderCondition::Success,
    )]));
    let reference = milkdrift_capability::ArtifactReference::new(
        "retained-report",
        "0".repeat(64),
        Some("text/plain".into()),
        Some(8),
    )?;
    assert!(
        validate_task_set(&definitions, &WorkflowLimits::default(), |_, _| Some(
            &reference
        ))
        .is_err()
    );
    Ok(())
}
