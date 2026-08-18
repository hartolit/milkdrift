//! Process-style headless runtime evidence against the production local store.

use std::sync::Arc;

use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, Edge, EdgeId, EdgeKind, Mutation, MutationBatch, Node, NodeId,
    NodeKind, PortId, TerminalOutcome, WorkflowId, WorkflowInterface,
};
use milkdrift_capability::{
    CapabilityDescriptorDocument, CapabilityRequirement, ErrorClass, OperationId,
};
use milkdrift_persistence::{
    ActorRef, CommandId, Reason, RevisionStore, RunJournal, RunSequence, TimestampMillis, WorkerId,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    DeterministicExecutor, ManualClock, RetryPolicy, RunCommand, RunCommandDocument, RunLifecycle,
    RuntimeConfig, RuntimeService, SchedulerLimits, SequentialIdGenerator,
};
use milkdrift_workspace::{RunId, ScopeId, WorkspaceBudget, WorkspaceScope};
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn sequence_revision() -> TestResult<BlueprintRevision> {
    let workflow = WorkflowId::new("runtime-reopen")?;
    let task_id = NodeId::new("generate")?;
    let terminal_id = NodeId::new("done")?;
    let next = PortId::new("next")?;
    let input = PortId::new("in")?;
    let task = Node::new(
        task_id.clone(),
        NodeKind::Task {
            requirement: CapabilityRequirement::new(OperationId::new("model.generate")?),
        },
    )?
    .with_control_output(next.clone())?;
    let terminal = Node::new(
        terminal_id.clone(),
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(input.clone())?;
    let batch = MutationBatch::new(vec![
        Mutation::SetInterface {
            interface: WorkflowInterface::new([], [])?,
        },
        Mutation::AddNode { node: task },
        Mutation::AddNode { node: terminal },
        Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new("generate-done")?,
                EdgeKind::Control,
                task_id,
                next,
                terminal_id,
                input,
            ),
        },
    ])?;
    Ok(BlueprintRevision::genesis(
        workflow,
        batch,
        AuthorRef::new("human:runtime-test")?,
        "durable runtime restart fixture",
    )?)
}

fn service(
    store: Arc<RedbStore>,
    clock: Arc<ManualClock>,
    id_prefix: &str,
) -> TestResult<RuntimeService> {
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    let executor = Arc::new(DeterministicExecutor::new(descriptor));
    let config = RuntimeConfig::new(
        WorkerId::new("worker-runtime-test")?,
        ActorRef::new("controller:runtime-test")?,
        30_000,
        32,
        SchedulerLimits::new(8, 4, 2, 4)?,
        RetryPolicy::new(3, vec![ErrorClass::Transport], 100, 10_000, 25)?,
    )?;
    Ok(RuntimeService::new(
        store,
        executor,
        clock,
        Arc::new(SequentialIdGenerator::new(id_prefix, 1)?),
        config,
    )?)
}

#[test]
fn redb_run_replays_after_complete_object_teardown_and_finishes() -> TestResult {
    let directory = TempDir::new()?;
    let revision = sequence_revision()?;
    let run = RunId::new("run-process-reopen")?;
    let actor = ActorRef::new("human:runtime-test")?;
    let clock = Arc::new(ManualClock::new(1_000));

    let before_restart = {
        let store = Arc::new(RedbStore::open(directory.path())?);
        store.put_revision(&revision)?;
        let runtime = service(store.clone(), clock.clone(), "before-restart")?;
        let create = RunCommandDocument::new(
            CommandId::new("command-create-run")?,
            run.clone(),
            actor.clone(),
            RunSequence::ZERO,
            TimestampMillis::new(1_000),
            Reason::new("create restart-test run")?,
            Vec::new(),
            RunCommand::CreateRun {
                workflow: revision.semantic().workflow().clone(),
                revision: revision.id().clone(),
                root_scope: WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-run-root")?),
                workspace_budget: WorkspaceBudget::new(
                    32, 16_384, 131_072, 8, 1_048_576, 4_194_304,
                )?,
                inputs: Vec::new(),
            },
        )?;
        let accepted = runtime.handle_command(&create)?;
        assert!(!accepted.replayed());
        let replayed = runtime.handle_command(&create)?;
        assert!(replayed.replayed());
        assert_eq!(store.head(&run)?, RunSequence::FIRST);

        let start = RunCommandDocument::new(
            CommandId::new("command-start-run")?,
            run.clone(),
            actor,
            store.head(&run)?,
            TimestampMillis::new(1_001),
            Reason::new("start restart-test run")?,
            Vec::new(),
            RunCommand::StartRun,
        )?;
        runtime.handle_command(&start)?;
        let projection = runtime.projection(&run)?;
        assert_eq!(projection.lifecycle(), RunLifecycle::Running);
        projection
    };

    let after_restart = {
        let store = Arc::new(RedbStore::open(directory.path())?);
        let runtime = service(store, clock, "after-restart")?;
        let projection = runtime.projection(&run)?;
        assert_eq!(projection, before_restart);
        runtime.recover()?;
        let tick = runtime.tick()?;
        assert_eq!(tick.dispatched, 1);
        assert_eq!(tick.completed, 1);
        let projection = runtime.projection(&run)?;
        assert!(matches!(projection.lifecycle(), RunLifecycle::Terminal(_)));
        projection
    };

    let store = Arc::new(RedbStore::open(directory.path())?);
    let runtime = service(store, Arc::new(ManualClock::new(2_000)), "terminal-reopen")?;
    assert_eq!(runtime.projection(&run)?, after_restart);
    Ok(())
}
