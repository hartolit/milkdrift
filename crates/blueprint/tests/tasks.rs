//! Ordinary task definitions reject ambiguous references before durable admission.
use milkdrift_blueprint::{
    TaskDefinition, TaskId, TaskInputBinding, TaskInputValue, WorkflowLimits, validate_task_set,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
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
    assert!(consumer.validate().is_err());
    consumer.dependencies.insert(a.clone());
    let mut definitions = BTreeMap::from([(a.clone(), task()?), (b.clone(), consumer)]);
    assert!(
        validate_task_set(&definitions, &WorkflowLimits::default()).is_err(),
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
    validate_task_set(&definitions, &WorkflowLimits::default())?;
    definitions
        .get_mut(&a)
        .ok_or("task absent")?
        .dependencies
        .insert(b.clone());
    assert!(validate_task_set(&definitions, &WorkflowLimits::default()).is_err());
    definitions.get_mut(&a).ok_or("task absent")?.dependencies = BTreeSet::new();
    definitions.get_mut(&a).ok_or("task absent")?.corrective_of = Some(b.clone());
    assert!(
        validate_task_set(&definitions, &WorkflowLimits::default()).is_err(),
        "correction waiting and explicit dependency form one causal cycle"
    );
    definitions.get_mut(&b).ok_or("task absent")?.corrective_of = Some(a);
    assert!(validate_task_set(&definitions, &WorkflowLimits::default()).is_err());
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
    Ok(())
}
