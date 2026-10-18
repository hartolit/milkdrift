//! A retained publication supplies a new future binding without rewriting its producer.

use super::{InvocationAllowance, TestResult, context, request, runtime_and_store};
use milkdrift_blueprint::{
    TaskBatch, TaskBindingRedirect, TaskChange, TaskId, TaskInputBinding, TaskInputValue,
};
use milkdrift_capability::{
    ArtifactReference, ErrorClass, InvocationEventKind, InvocationFailure, InvocationTerminal,
    OperationId, SideEffectClass, TerminalStatus,
};
use milkdrift_persistence::{ArtifactPublicationId, ArtifactStore, BeginArtifactPublication};
use milkdrift_runtime::{DeterministicExecutor, ManualClock, TaskState};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactOwner, ArtifactProvenance, ArtifactRetention,
    ArtifactSensitivity, CausalId, CausalReference, ContentDigest, MediaType, RunId,
    WorkspaceBudget,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

struct SourceAuthority(AtomicBool);

impl milkdrift_authority::AuthorityEvaluator for SourceAuthority {
    fn evaluate(
        &self,
        request: &milkdrift_authority::AuthorityRequest,
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, milkdrift_authority::AuthorityError>
    {
        if matches!(
            request.operation,
            milkdrift_authority::AuthorityOperation::ReadArtifactMetadata
                | milkdrift_authority::AuthorityOperation::ReadArtifactContent
        ) {
            let allowed = !self.0.load(Ordering::Acquire)
                && request.resources.artifact_sensitivity == Some(ArtifactSensitivity::Public)
                && request
                    .budget
                    .artifact_bytes
                    .is_some_and(|bytes| bytes <= 64);
            return milkdrift_authority::AuthorityDecisionSnapshot::from_evaluation(
                milkdrift_authority::PolicyId::new("historical-sources")?,
                1,
                request.clone(),
                vec![if allowed {
                    milkdrift_authority::DecisionReasonCode::Allowed
                } else {
                    milkdrift_authority::DecisionReasonCode::ArtifactScopeMismatch
                }],
                milkdrift_authority::AuthorityBudget {
                    artifact_bytes: Some(64),
                    ..milkdrift_authority::AuthorityBudget::default()
                },
                SideEffectClass::None,
            );
        }
        milkdrift_authority::AuthorityEvaluator::evaluate(&InvocationAllowance(Some(8)), request)
    }
}

fn publish(
    store: &dyn ArtifactStore,
    identity: &str,
    media: &str,
    bytes: &[u8],
) -> TestResult<ArtifactReference> {
    let metadata = ArtifactMetadata::new(
        milkdrift_workspace::ArtifactReference::new(
            ArtifactId::new(identity)?,
            ContentDigest::for_bytes(bytes),
            MediaType::new(media)?,
            u64::try_from(bytes.len())?,
        ),
        ArtifactSensitivity::Public,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("historical-output-fixture")?,
            },
            vec![],
        )?,
    )?;
    let run = RunId::new("historical-output-fixture")?;
    let usage = store.artifact_usage(&ArtifactOwner::Workflow { run: run.clone() })?;
    let publication = BeginArtifactPublication::new(
        ArtifactPublicationId::new(format!("publish:{identity}"))?,
        run,
        metadata.clone(),
        WorkspaceBudget::new(0, 0, 0, 16, 2_097_152, 16_777_216)?,
        usage,
    )?;
    store.begin_publication(&publication)?;
    for (index, chunk) in bytes.chunks(262_144).enumerate() {
        store.write_chunk(
            publication.publication(),
            u64::try_from(index.checked_mul(262_144).ok_or("offset overflow")?)?,
            chunk,
        )?;
    }
    store.commit_publication(publication.publication())?;
    Ok(ArtifactReference::new(
        identity,
        metadata.reference().digest().to_hex(),
        Some(media.into()),
        Some(u64::try_from(bytes.len())?),
    )?)
}

fn output(
    executor: &DeterministicExecutor,
    reference: &ArtifactReference,
    fail: bool,
) -> TestResult {
    let failure = fail
        .then(|| {
            InvocationFailure::new(
                ErrorClass::Provider,
                false,
                "research_failed",
                "Research failed after publishing evidence.",
                None,
            )
        })
        .transpose()?;
    executor.set_script(
        OperationId::new("model.generate")?,
        vec![
            InvocationEventKind::Output {
                name: "report".into(),
                reference: reference.clone(),
            },
            InvocationEventKind::Terminal {
                terminal: InvocationTerminal::new(
                    if fail {
                        TerminalStatus::Failure
                    } else {
                        TerminalStatus::Success
                    },
                    if fail {
                        Vec::new()
                    } else {
                        vec![reference.clone()]
                    },
                    failure,
                    None,
                    SideEffectClass::None,
                )?,
            },
        ],
    )?;
    Ok(())
}

#[test]
fn historical_publication_binds_after_restart_and_freezes_without_a_declaration() -> TestResult {
    let root = tempfile::tempdir()?;
    let clock = Arc::new(ManualClock::new(1_000));
    let authority = Arc::new(InvocationAllowance(Some(8)));
    let (workflow, producer_id, source, original) = {
        let (runtime, executor, control, store) =
            runtime_and_store(root.path(), authority.clone(), clock.clone(), 32)?;
        let mut create = request()?;
        create
            .tasks
            .first_mut()
            .ok_or("task absent")?
            .definition
            .inputs
            .clear();
        create.finish_when_settled = false;
        let workflow = control.create(&context()?, "historical", create)?.workflow;
        let source = publish(
            store.as_ref(),
            "original-report",
            "text/plain",
            b"original report",
        )?;
        output(&executor, &source, false)?;
        runtime.execute(runtime.tick(1)?.pop().ok_or("producer dispatch absent")?)?;
        let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
        let producer = record.tasks.values().next().ok_or("producer absent")?;
        assert!(producer.definition.outputs.is_empty());
        (workflow, producer.id.clone(), source, producer.clone())
    };
    let (runtime, executor, control, store) = runtime_and_store(root.path(), authority, clock, 32)?;
    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    let mut consumer = original.definition.clone();
    consumer.dependencies.insert(producer_id.clone());
    consumer.inputs = BTreeMap::from([(
        "source".into(),
        TaskInputBinding::TaskOutput {
            task: producer_id.clone(),
            output: "report".into(),
            optional: false,
        },
    )]);
    let batch = TaskBatch {
        expected_generation: record.generation,
        changes: vec![TaskChange::Add {
            alias: TaskId::new("consumer")?,
            definition: consumer,
        }],
    };
    let accepted = control.edit(&context()?, "connect-historical", &workflow, batch.clone())?;
    assert_eq!(
        control.edit(&context()?, "connect-historical", &workflow, batch)?,
        accepted
    );
    let dispatch = runtime.tick(1)?.pop().ok_or("consumer dispatch absent")?;
    let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
    assert_eq!(record.tasks.get(&producer_id), Some(&original));
    let consumer = record
        .tasks
        .values()
        .find(|task| task.id != producer_id)
        .ok_or("consumer absent")?;
    assert_eq!(
        consumer
            .frozen
            .as_ref()
            .and_then(|start| start.inputs.get("source")),
        Some(&TaskInputValue::Artifact { reference: source })
    );
    // A later output cannot change the already frozen source, even when it has the same name.
    output(
        &executor,
        &publish(
            store.as_ref(),
            "consumer-report",
            "text/plain",
            b"consumer report",
        )?,
        false,
    )?;
    runtime.execute(dispatch)?;
    assert_eq!(
        runtime
            .workflow(&workflow)?
            .ok_or("workflow absent")?
            .tasks
            .get(&producer_id),
        Some(&original)
    );
    Ok(())
}

#[test]
fn failed_historical_output_blocks_until_a_compatible_larger_correction_is_adopted() -> TestResult {
    for (media, bytes, compatible) in [
        (
            "text/plain",
            b"a corrected report longer than the original".as_slice(),
            true,
        ),
        ("text/markdown", b"wrong media".as_slice(), false),
    ] {
        let root = tempfile::tempdir()?;
        let (runtime, executor, control, store) = runtime_and_store(
            root.path(),
            Arc::new(InvocationAllowance(Some(8))),
            Arc::new(ManualClock::new(1_000)),
            32,
        )?;
        let mut create = request()?;
        create
            .tasks
            .first_mut()
            .ok_or("task absent")?
            .definition
            .inputs
            .clear();
        create.finish_when_settled = false;
        let workflow = control
            .create(&context()?, "failed-original", create)?
            .workflow;
        let source = publish(store.as_ref(), "failed-report", "text/plain", b"old")?;
        output(&executor, &source, true)?;
        runtime.execute(runtime.tick(1)?.pop().ok_or("producer dispatch absent")?)?;
        let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
        let original = record
            .tasks
            .values()
            .next()
            .ok_or("producer absent")?
            .clone();
        assert_eq!(original.state, TaskState::Failed);
        assert!(original.definition.outputs.is_empty());
        let mut consumer = original.definition.clone();
        consumer.title = "Future consumer".into();
        consumer.dependencies.insert(original.id.clone());
        consumer.inputs = BTreeMap::from([(
            "source".into(),
            TaskInputBinding::TaskOutput {
                task: original.id.clone(),
                output: "report".into(),
                optional: false,
            },
        )]);
        let mut optional = consumer.clone();
        optional.title = "Optional historical result".into();
        optional.required = false;
        if let Some(TaskInputBinding::TaskOutput { optional, .. }) =
            optional.inputs.get_mut("source")
        {
            *optional = true;
        }
        let mut correction = original.definition.clone();
        correction.title = "Corrected producer".into();
        correction.corrective_of = Some(original.id.clone());
        control.edit(
            &context()?,
            "add-consumer-and-correction",
            &workflow,
            TaskBatch {
                expected_generation: record.generation,
                changes: vec![
                    TaskChange::Add {
                        alias: TaskId::new("consumer")?,
                        definition: consumer,
                    },
                    TaskChange::Add {
                        alias: TaskId::new("correction")?,
                        definition: correction,
                    },
                    TaskChange::Add {
                        alias: TaskId::new("optional")?,
                        definition: optional,
                    },
                ],
            },
        )?;
        let replacement = publish(store.as_ref(), "replacement-report", media, bytes)?;
        output(&executor, &replacement, false)?;
        let dispatches = runtime.tick(4)?;
        assert_eq!(
            dispatches.len(),
            2,
            "only the correction and explicit optional consumer may start"
        );
        for dispatch in dispatches {
            runtime.execute(dispatch)?;
        }
        let before = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
        let optional = before
            .tasks
            .values()
            .find(|task| task.definition.title == "Optional historical result")
            .ok_or("optional consumer absent")?;
        assert_eq!(optional.state, TaskState::Completed);
        assert_eq!(
            optional
                .frozen
                .as_ref()
                .and_then(|start| start.inputs.get("source")),
            Some(&TaskInputValue::Artifact {
                reference: source.clone()
            })
        );
        let consumer = before
            .tasks
            .values()
            .find(|task| task.definition.title == "Future consumer")
            .ok_or("consumer absent")?;
        let correction = before
            .tasks
            .values()
            .find(|task| task.definition.corrective_of.is_some())
            .ok_or("correction absent")?;
        assert_eq!(consumer.state, TaskState::Queued);
        let consumer_id = consumer.id.clone();
        let correction_id = correction.id.clone();
        let adoption = TaskBatch {
            expected_generation: before.generation,
            changes: vec![TaskChange::AdoptCorrection {
                task: original.id.clone(),
                correction: correction_id.clone(),
                redirects: vec![TaskBindingRedirect {
                    consumer: consumer_id.clone(),
                    expected_version: consumer.version,
                    input: "source".into(),
                }],
            }],
        };
        let result = control.edit(&context()?, "adopt", &workflow, adoption);
        assert_eq!(result.is_ok(), compatible);
        let after = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
        if !compatible {
            assert_eq!(
                after, before,
                "media refusal must leave the entire workflow unchanged"
            );
            continue;
        }
        let retained = after.tasks.get(&original.id).ok_or("original absent")?;
        assert_eq!(retained.frozen, original.frozen);
        assert_eq!(retained.result, original.result);
        assert_eq!(retained.state, TaskState::Failed);
        assert_eq!(retained.adopted_correction, Some(correction_id));
        let _dispatch = runtime.tick(1)?.pop().ok_or("consumer should start")?;
        let record = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
        assert_eq!(
            record
                .tasks
                .get(&consumer_id)
                .and_then(|task| task.frozen.as_ref())
                .and_then(|start| start.inputs.get("source")),
            Some(&TaskInputValue::Artifact {
                reference: replacement
            })
        );
    }
    Ok(())
}

#[test]
fn source_admission_checks_current_permission_real_publication_and_byte_scope_atomically()
-> TestResult {
    for mode in ["valid", "wrong-digest", "unpublished", "over-authority"] {
        let root = tempfile::tempdir()?;
        let authority = Arc::new(SourceAuthority(AtomicBool::new(true)));
        let (runtime, executor, control, store) = runtime_and_store(
            root.path(),
            authority.clone(),
            Arc::new(ManualClock::new(1_000)),
            32,
        )?;
        let mut create = request()?;
        create
            .tasks
            .first_mut()
            .ok_or("task absent")?
            .definition
            .inputs
            .clear();
        create.finish_when_settled = false;
        let workflow = control
            .create(&context()?, "permission-source", create)?
            .workflow;
        let bytes = if mode == "over-authority" {
            vec![b'x'; 65]
        } else {
            b"source report".to_vec()
        };
        let actual = publish(store.as_ref(), "permission-report", "text/plain", &bytes)?;
        let reference = ArtifactReference::new(
            if mode == "unpublished" {
                "not-published"
            } else {
                actual.identity()
            },
            if mode == "wrong-digest" {
                "0".repeat(64)
            } else {
                actual.digest().into()
            },
            actual.media_type().map(str::to_owned),
            actual.size_bytes(),
        )?;
        output(&executor, &reference, false)?;
        runtime.execute(runtime.tick(1)?.pop().ok_or("producer dispatch absent")?)?;
        let before = runtime.workflow(&workflow)?.ok_or("workflow absent")?;
        let producer = before.tasks.values().next().ok_or("producer absent")?;
        let mut consumer = producer.definition.clone();
        consumer.dependencies.insert(producer.id.clone());
        consumer.inputs = BTreeMap::from([(
            "source".into(),
            TaskInputBinding::TaskOutput {
                task: producer.id.clone(),
                output: "report".into(),
                optional: false,
            },
        )]);
        let batch = TaskBatch {
            expected_generation: before.generation,
            changes: vec![TaskChange::Add {
                alias: TaskId::new("consumer")?,
                definition: consumer,
            }],
        };
        let denied = control.edit(&context()?, "source-denied", &workflow, batch.clone());
        assert!(
            matches!(
                &denied,
                Err(milkdrift_control::ControlError::AuthorizationDenied { .. })
            ),
            "{mode}: {denied:?}"
        );
        assert_eq!(
            runtime.workflow(&workflow)?.ok_or("workflow absent")?,
            before
        );
        authority.0.store(false, Ordering::Release);
        let result = control.edit(&context()?, "source-allowed", &workflow, batch);
        assert_eq!(result.is_ok(), mode == "valid");
        if mode != "valid" {
            assert_eq!(
                runtime.workflow(&workflow)?.ok_or("workflow absent")?,
                before
            );
        }
    }
    Ok(())
}
