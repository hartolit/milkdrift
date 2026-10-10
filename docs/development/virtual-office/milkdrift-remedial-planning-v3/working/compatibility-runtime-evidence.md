# U19 topology and prospective-repair runtime diagnostic

Delta authored this bounded standalone Rust diagnostic; Rowan ran it against production blueprint,
runtime and redb with isolated `TempDir` stores. Production source/schema were unchanged. The fixture
executor never entered an external task; the only Task case remains behind an active 60-second
Wait and is never dispatched. No provider, live daemon, deployed store or paid capability was used.

This is current-behavior evidence for the [U19 comparison](structure-comparison.md#u19-reopens-representation-and-removes-the-conversion-prerequisite-for-repair),
not an implementation of proposed region source, a whole-language conversion proof or acceptance
of the exposed crossing behavior. Final process exit 0 means all diagnostic cases were attempted;
its output explicitly reports one observed production error.

## Exact observations

| Case | Construction and real runtime observation | What it establishes |
| --- | --- | --- |
| Proper nested All joins | Eight nodes/nine control edges accepted; run reached explicit terminal and succeeded. | Positive control for the same node/configuration family. |
| Crossed All joins | Same counts accepted; after 32 bounded ticks the run remained Running with no pending successor scans and retained branch ownership. | Validator admits non-nested ownership; the runtime does not give it general cross-scope token behavior. This is not useful extra concurrency. |
| Crossed Any outer join | Accepted definition; runtime returned `InvalidHistory` at sequence 37: timer cancellation lacks a structured owner cancellation fact. | A real current validation/runtime defect boundary, not a modeled risk or legitimate successful expressive capability. No production repair was performed. |
| Fork with two direct success terminals and no Join | Accepted and succeeded; two branch-terminal facts, no fabricated JoinSatisfied. | Exact supported behavior, also covered by the existing structured-runtime test. It is not automatically equivalent to a new joined Parallel's output semantics. |
| Paused prefix Wait → pending Task → unchanged crossed All graph | Task requirement replacement produced `ChangedPending / UseNewOnNextInvocation`; Wait was `UnchangedActive / Preserve`; approval and application succeeded. Active Wait execution identity stayed identical; Task remained unentered; crossing structure unchanged. | Safe native future editing can succeed without converting an unrelated non-nested part of the definition. Does not show the later crossing work would complete. |

The first run stopped at the genuine crossed-Any error before later cases. Delta changed the
harness to retain each case error and continue. The second run reached the repair case but its
harness stored the child revision before its parent, producing a parent-revision-not-found refusal.
The final harness stores/starts the parent first, then its proposed child revision. These were
observation-control corrections, not product fixes. An initial field named `active_branches` counted
retained records rather than only active records; the final output correctly says
`retained_branch_records`. Do not infer four active branches from the first log.

The crossed graph and the safe pending repair need different disposition. Future hardening must
not rewrite old accepted history or fabricate a Join/terminal/cancellation fact. Precise validation
for new construction and graph-native prospective repair of unentered topology may prevent the
crossing; a runtime fix must preserve actual cancellation/ownership evidence for already accepted
work. Existing active/uncertain scopes cannot be relocated under a guessed region decomposition.
A store load cannot simply reject all old affected definitions, and an unrelated native Task edit
cannot be barred as a substitute for investigating the defect. The final implementation program
must include explicit source/runtime fault oracles, not reproduce this failure as intended new
region semantics.

`PlanTransition::push_event` in
[transition.rs](../../../../../crates/runtime/src/engine/transition.rs) applies the proposed event to
the candidate projection before pushing it into the events to commit. The crossed-Any error is a
planned-event consistency failure; it is not evidence that an invalid cancellation event was retained
in the journal. No separate store-corruption/recovery qualification was performed by this diagnostic.

## Reproduction and identities

The final command, run by Rowan from the repository root, was:

```sh
CARGO_TARGET_DIR=/home/hartolit/Projects/dev/milkdrift/target/remedial-planning-v3/priority-target cargo run --offline --manifest-path /tmp/milkdrift-compatibility-delta/Cargo.toml > /home/hartolit/Projects/dev/milkdrift/target/remedial-planning-v3/compatibility-probe.log 2>&1
```

Environment uses Rust 1.95.0 (`59807616e`, 2026-04-14), Cargo 1.95.0 (`f2d3ce0b`, 2026-03-21),
Linux repository host. Final incremental compilation took 0.73 seconds. The first build took
10.85 seconds. These are diagnostic build times, not runtime or product performance qualification.
Production source at the planning checkpoint is unchanged; the reproduction paths below bind the
actual checked-in production crates, with `runtime/test-support` only for the deterministic fixture.

| Retained object | SHA-256 |
| --- | --- |
| Final Cargo.toml | `386fe6b59380aae526dcd8680de936bd282b2f037d3413af9b6ad0d1ecbf4f91` |
| Final src/main.rs | `3bc4bc04636d22286620a850bee5c2a8c352b1a12cc2b835041bb930588f16b4` |
| Final compatibility-probe.log | `49a095afd3d00986e1f36d99c966475062d5d22c9431c8141451ea5cb6456b44` |
| Initial log, stopped at crossed Any | `baa37b93c304ae2c7d3a34b4db985189e690f30988f3fbb50c58a3336b599733` |
| Second log, with parent-storage harness error | `31e1206b22000aed302d7c0a2d7a0f2bba22db1197278c74bfc70479b1de3403` |

Raw preliminary and final logs remain under `target/remedial-planning-v3/`; the exact final manifest,
source and log are embedded below so `/tmp` and ignored build artifacts are not their sole record.

## Final manifest

```toml
[package]
name = "milkdrift-compatibility-planning-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
milkdrift-authority = { path = "/home/hartolit/Projects/dev/milkdrift/crates/authority" }
milkdrift-blueprint = { path = "/home/hartolit/Projects/dev/milkdrift/crates/blueprint" }
milkdrift-capability = { path = "/home/hartolit/Projects/dev/milkdrift/crates/capability" }
milkdrift-persistence = { path = "/home/hartolit/Projects/dev/milkdrift/crates/persistence" }
milkdrift-redb-store = { path = "/home/hartolit/Projects/dev/milkdrift/adapters/redb-store" }
milkdrift-runtime = { path = "/home/hartolit/Projects/dev/milkdrift/crates/runtime", features = ["test-support"] }
milkdrift-workspace = { path = "/home/hartolit/Projects/dev/milkdrift/crates/workspace" }
tempfile = "3.21.0"
```

## Final diagnostic source

```rust
//! Planning diagnostic: real blueprint/runtime/redb, isolated TempDirs, no external tasks entered.
use std::{collections::BTreeSet, sync::Arc};
use milkdrift_authority::{ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError,
    AuthorityEvaluator, DecisionReasonCode, GrantDigest, GrantId, PolicyId};
use milkdrift_blueprint::{AuthorRef, BlueprintRevision, Edge, EdgeId, EdgeKind, ForkConfig,
    JoinConfig, JoinPolicy, Mutation, MutationBatch, Node, NodeId, NodeKind, PortId,
    TerminalOutcome, WorkflowId, WorkflowInterface};
use milkdrift_capability::{CapabilityDescriptorDocument, CapabilityRequirement, OperationId, SideEffectClass};
use milkdrift_persistence::{AuthorityDecision, CommandDisposition, Reason, ReconciliationDecisionId,
    ReconciliationId, ReconciliationPolicy, RevisionStore, RunEventKind, RunJournal, WorkerId};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{CommandAuthorityClaim, DeterministicExecutor, ManualClock, RetryPolicy,
    RunCommand, RuntimeConfig, RuntimeService, SchedulerLimits, SequentialIdGenerator};
use milkdrift_workspace::{RunId, ScopeId, WorkspaceBudget, WorkspaceScope};
use tempfile::TempDir;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
struct ProbeAuthority;
impl AuthorityEvaluator for ProbeAuthority {
    fn evaluate(&self, request: &milkdrift_authority::AuthorityRequest)
        -> std::result::Result<AuthorityDecisionSnapshot, AuthorityError> {
        AuthorityDecisionSnapshot::from_evaluation(PolicyId::new("test.compatibility-probe")?, 1,
            request.clone(), vec![DecisionReasonCode::Allowed], AuthorityBudget {
                cost_minor: Some(u64::MAX), duration_ms: Some(u64::MAX), invocations: Some(u64::MAX),
                artifact_bytes: Some(u64::MAX), units: Some(u64::MAX), concurrency: Some(u32::MAX),
            }, SideEffectClass::Unknown)
    }
}
fn id(s: &str) -> NodeId { NodeId::new(s).unwrap() }
fn port(s: &str) -> PortId { PortId::new(s).unwrap() }
fn wait(name: &str, millis: u64) -> Result<Node> {
    Ok(Node::new(id(name), NodeKind::Wait { duration_ms: millis })?
        .with_control_input(port("in"))?.with_control_output(port("out"))?)
}
fn task(operation: &str) -> Result<Node> {
    Ok(Node::new(id("pending-task"), NodeKind::task_direct_inputs(
        CapabilityRequirement::new(OperationId::new(operation)?))?)?
        .with_control_input(port("in"))?.with_control_output(port("out"))?)
}
fn fork(name: &str) -> Result<Node> {
    Ok(Node::new(id(name), NodeKind::Fork { config: ForkConfig::new(
        BTreeSet::from([port("a"), port("b")]))? })?
        .with_control_input(port("in"))?.with_control_output(port("a"))?.with_control_output(port("b"))?)
}
fn join(name: &str, owner: &str, policy: JoinPolicy) -> Result<Node> {
    Ok(Node::new(id(name), NodeKind::Join { config: JoinConfig::new(id(owner), policy) })?
        .with_control_input(port("in"))?.with_control_output(port("out"))?)
}
fn terminal(name: &str) -> Result<Node> {
    Ok(Node::new(id(name), NodeKind::Terminal { outcome: TerminalOutcome::Success })?
        .with_control_input(port("in"))?)
}
fn edge(name: &str, source: &str, source_port: &str, target: &str) -> Result<Edge> {
    Ok(Edge::new(EdgeId::new(name)?, EdgeKind::Control, id(source), port(source_port), id(target), port("in")))
}
fn build(name: &str, nodes: Vec<Node>, edges: Vec<Edge>) -> Result<BlueprintRevision> {
    let mut mutations = vec![Mutation::SetInterface { interface: WorkflowInterface::new([], [])? }];
    mutations.extend(nodes.into_iter().map(|node| Mutation::AddNode { node }));
    mutations.extend(edges.into_iter().map(|edge| Mutation::AddEdge { edge }));
    Ok(BlueprintRevision::genesis(WorkflowId::new(name)?, MutationBatch::new(mutations)?,
        AuthorRef::new("human:compatibility-probe")?, "Planning-only graph compatibility diagnostic")?)
}
fn crossed(name: &str, crossing: bool, policy: JoinPolicy, prefix: bool) -> Result<BlueprintRevision> {
    let mut nodes = vec![fork("F")?, fork("G")?, wait("A", 1)?, wait("B", 1)?, wait("C", 1)?,
        join("JF", "F", policy)?, join("JG", "G", JoinPolicy::All)?, terminal("done")?];
    let mut edges = vec![edge("F-a", "F", "a", "G")?, edge("F-b", "F", "b", "B")?,
        edge("G-a", "G", "a", "A")?, edge("G-b", "G", "b", "C")?];
    if crossing {
        edges.extend([edge("A-JF", "A", "out", "JF")?, edge("B-JF", "B", "out", "JF")?,
            edge("JF-JG", "JF", "out", "JG")?, edge("C-JG", "C", "out", "JG")?,
            edge("JG-done", "JG", "out", "done")?]);
    } else {
        edges.extend([edge("A-JG", "A", "out", "JG")?, edge("C-JG", "C", "out", "JG")?,
            edge("JG-JF", "JG", "out", "JF")?, edge("B-JF", "B", "out", "JF")?,
            edge("JF-done", "JF", "out", "done")?]);
    }
    if prefix {
        nodes.extend([wait("entry-wait", 60_000)?, task("model.generate")?]);
        edges.extend([edge("entry-pending", "entry-wait", "out", "pending-task")?,
            edge("pending-F", "pending-task", "out", "F")?]);
    }
    build(name, nodes, edges)
}
struct Harness { _dir: TempDir, store: Arc<RedbStore>, clock: Arc<ManualClock>, runtime: RuntimeService,
    claim: CommandAuthorityClaim, run: RunId }
impl Harness {
    fn new(name: &str) -> Result<Self> {
        let dir = TempDir::new()?;
        let store = Arc::new(RedbStore::open(dir.path())?);
        let clock = Arc::new(ManualClock::new(10_000));
        let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
            "/home/hartolit/Projects/dev/milkdrift/crates/capability/tests/fixtures/descriptor-v1.json"))?.body().clone();
        let runtime = RuntimeService::new_with_authority(store.clone(),
            Arc::new(DeterministicExecutor::new(descriptor)), Arc::new(ProbeAuthority), clock.clone(),
            Arc::new(SequentialIdGenerator::new(name, 1)?), RuntimeConfig::new(WorkerId::new(name)?,
                ActorRef::new("test:compatibility-probe")?, 30000, 256, SchedulerLimits::new(8, 4, 2, 4)?,
                RetryPolicy::new(1, vec![], 10, 1000, 0)?)?)?;
        Ok(Self { _dir: dir, store, clock, runtime, run: RunId::new(name)?,
            claim: CommandAuthorityClaim::new(GrantId::new("grant:compatibility-probe")?, 1,
                GrantDigest::new(format!("b3_{}", "0".repeat(64)))?, 0)? })
    }
    fn command(&self, command: RunCommand) -> Result<CommandDisposition> {
        let doc = self.runtime.command(self.run.clone(), ActorRef::new("human:compatibility-probe")?,
            self.store.head(&self.run)?, Reason::new("Planning-only isolated diagnostic")?, vec![], command)?;
        Ok(self.runtime.handle_authorized_command(&doc, &self.claim)?.result().disposition())
    }
    fn start(&self, revision: &BlueprintRevision) -> Result {
        self.store.put_revision(revision)?;
        assert_eq!(self.command(RunCommand::CreateRun { workflow: revision.semantic().workflow().clone(),
            revision: revision.id().clone(), root_scope: WorkspaceScope::run_root(self.run.clone(), ScopeId::new("root")?),
            workspace_budget: WorkspaceBudget::new(512, 1048576, 1048576, 32, 1048576, 1048576)?, inputs: vec![] })?, CommandDisposition::Accepted);
        assert_eq!(self.command(RunCommand::StartRun)?, CommandDisposition::Accepted);
        Ok(())
    }
}
fn observe(name: &str, revision: BlueprintRevision) -> Result {
    println!("CASE {name}: definition=accepted nodes={} edges={}", revision.semantic().nodes().len(), revision.semantic().edges().len());
    let h = Harness::new(name)?;
    h.start(&revision)?;
    for _ in 0..32 {
        if h.runtime.projection(&h.run)?.is_completed() { break; }
        h.clock.advance(10)?;
        assert_eq!(h.runtime.scheduler_tick()?.dispatched, 0, "No task should enter");
    }
    let p = h.runtime.projection(&h.run)?;
    println!("CASE {name}: lifecycle={:?} completed={} pending_successors={} retained_branch_records={}",
        p.lifecycle(), p.is_completed(), p.pending_successor_execution_ids().len(), p.branches().len());
    for e in h.runtime.history(&h.run)? {
        match e.kind() {
            RunEventKind::NodeBecameEligible { node, scope, .. } => println!("  eligible node={node} scope={scope:?}"),
            RunEventKind::JoinSatisfied { rule, branches, .. } => println!("  join rule={rule:?} selected={}", branches.len()),
            RunEventKind::BranchTerminal { outcome, .. } => println!("  branch_terminal outcome={outcome:?}"),
            RunEventKind::RunTerminal { outcome, .. } => println!("  run_terminal outcome={outcome:?}"),
            _ => {},
        }
    }
    Ok(())
}
fn prospective_repair() -> Result {
    let old = crossed("crossed-pending-repair", true, JoinPolicy::All, true)?;
    let new = old.revise(old.id(), MutationBatch::new(vec![Mutation::ReplaceNode {
        node: task("model.assess")? }])?, AuthorRef::new("human:compatibility-probe")?,
        "Replace only an unentered task; leave unrelated crossing topology unchanged")?;
    let h = Harness::new("crossed-pending-repair")?;
    h.start(&old)?;
    h.store.put_revision(&new)?;
    let before = h.runtime.projection(&h.run)?;
    let wait_execution = before.executions_for_node(&id("entry-wait")).next().ok_or("active wait absent")?.execution().clone();
    assert!(before.executions_for_node(&id("pending-task")).next().is_none());
    assert_eq!(h.command(RunCommand::PauseRun)?, CommandDisposition::Accepted);
    assert_eq!(h.command(RunCommand::RequestRevisionAdoption { reconciliation: ReconciliationId::new("repair")?,
        revision: new.id().clone(), policy: ReconciliationPolicy::FinishCurrentThenAdopt })?, CommandDisposition::Accepted);
    let p = h.runtime.projection(&h.run)?;
    let plan = p.reconciliation().plans().values().next().ok_or("plan absent")?;
    for item in plan.items() { println!("REPAIR item node={:?} classification={:?} action={:?}", item.node, item.classification, item.action); }
    let plan_id = plan.plan().clone();
    assert_eq!(h.command(RunCommand::DecideReconciliation { plan: plan_id.clone(),
        decision: ReconciliationDecisionId::new("approve-repair")?, outcome: AuthorityDecision::Approve })?, CommandDisposition::Accepted);
    assert_eq!(h.command(RunCommand::ApplyReconciliation { plan: plan_id })?, CommandDisposition::Accepted);
    let p = h.runtime.projection(&h.run)?;
    assert_eq!(p.revision(), Some(new.id()));
    assert_eq!(p.executions_for_node(&id("entry-wait")).next().ok_or("wait lost")?.execution(), &wait_execution);
    assert!(p.executions_for_node(&id("pending-task")).next().is_none());
    println!("REPAIR accepted=true active_wait_identity_preserved=true pending_task_unentered=true unrelated_crossing_unchanged=true");
    Ok(())
}
fn main() -> Result {
    // A production refusal/error is a diagnostic observation, not permission to skip later cases.
    let mut observed_errors = 0;
    for (name, crossing, policy) in [
        ("proper-nested-all", false, JoinPolicy::All),
        ("crossed-all", true, JoinPolicy::All),
        ("crossed-any", true, JoinPolicy::Any),
    ] {
        let result = crossed(name, crossing, policy, false).and_then(|r| observe(name, r));
        if let Err(error) = result {
            observed_errors += 1;
            println!("OBSERVED_CASE_ERROR case={name} error={error:?}");
        }
    }
    let nojoin = (|| -> Result {
        let revision = build("fork-no-join", vec![fork("F")?, terminal("left")?, terminal("right")?],
            vec![edge("left", "F", "a", "left")?, edge("right", "F", "b", "right")?])?;
        observe("fork-no-join", revision)
    })();
    if let Err(error) = nojoin {
        observed_errors += 1;
        println!("OBSERVED_CASE_ERROR case=fork-no-join error={error:?}");
    }
    if let Err(error) = prospective_repair() {
        observed_errors += 1;
        println!("OBSERVED_CASE_ERROR case=prospective-repair error={error:?}");
    }
    println!("DIAGNOSTIC_FINISHED observed_case_errors={observed_errors}; completion is not an acceptance pass");
    Ok(())
}
```

## Exact final output

```text
   Compiling milkdrift-compatibility-planning-probe v0.0.0 (/tmp/milkdrift-compatibility-delta)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.73s
     Running `target/remedial-planning-v3/priority-target/debug/milkdrift-compatibility-planning-probe`
CASE proper-nested-all: definition=accepted nodes=8 edges=9
CASE proper-nested-all: lifecycle=Terminal(Succeeded) completed=true pending_successors=0 retained_branch_records=0
  eligible node=F scope=ScopeReference { run: RunId("proper-nested-all"), scope: ScopeId("root") }
  eligible node=G scope=ScopeReference { run: RunId("proper-nested-all"), scope: ScopeId("proper-nested-all-scope-9") }
  eligible node=B scope=ScopeReference { run: RunId("proper-nested-all"), scope: ScopeId("proper-nested-all-scope-15") }
  eligible node=A scope=ScopeReference { run: RunId("proper-nested-all"), scope: ScopeId("proper-nested-all-scope-23") }
  eligible node=C scope=ScopeReference { run: RunId("proper-nested-all"), scope: ScopeId("proper-nested-all-scope-29") }
  branch_terminal outcome=Succeeded
  eligible node=JF scope=ScopeReference { run: RunId("proper-nested-all"), scope: ScopeId("root") }
  branch_terminal outcome=Succeeded
  eligible node=JG scope=ScopeReference { run: RunId("proper-nested-all"), scope: ScopeId("proper-nested-all-scope-9") }
  branch_terminal outcome=Succeeded
  join rule=All selected=2
  branch_terminal outcome=Succeeded
  join rule=All selected=2
  eligible node=done scope=ScopeReference { run: RunId("proper-nested-all"), scope: ScopeId("root") }
  run_terminal outcome=Succeeded
CASE crossed-all: definition=accepted nodes=8 edges=9
CASE crossed-all: lifecycle=Running completed=false pending_successors=0 retained_branch_records=4
  eligible node=F scope=ScopeReference { run: RunId("crossed-all"), scope: ScopeId("root") }
  eligible node=G scope=ScopeReference { run: RunId("crossed-all"), scope: ScopeId("crossed-all-scope-9") }
  eligible node=B scope=ScopeReference { run: RunId("crossed-all"), scope: ScopeId("crossed-all-scope-15") }
  eligible node=A scope=ScopeReference { run: RunId("crossed-all"), scope: ScopeId("crossed-all-scope-23") }
  eligible node=C scope=ScopeReference { run: RunId("crossed-all"), scope: ScopeId("crossed-all-scope-29") }
  branch_terminal outcome=Succeeded
  eligible node=JF scope=ScopeReference { run: RunId("crossed-all"), scope: ScopeId("root") }
  branch_terminal outcome=Succeeded
  eligible node=JG scope=ScopeReference { run: RunId("crossed-all"), scope: ScopeId("crossed-all-scope-9") }
CASE crossed-any: definition=accepted nodes=8 edges=9
OBSERVED_CASE_ERROR case=crossed-any error=InvalidHistory("event crossed-any-event-57 at sequence 37: timer cancellation lacks a structured owner cancellation fact")
CASE fork-no-join: definition=accepted nodes=3 edges=2
CASE fork-no-join: lifecycle=Terminal(Succeeded) completed=true pending_successors=0 retained_branch_records=0
  eligible node=F scope=ScopeReference { run: RunId("fork-no-join"), scope: ScopeId("root") }
  eligible node=left scope=ScopeReference { run: RunId("fork-no-join"), scope: ScopeId("fork-no-join-scope-9") }
  eligible node=right scope=ScopeReference { run: RunId("fork-no-join"), scope: ScopeId("fork-no-join-scope-15") }
  branch_terminal outcome=Succeeded
  branch_terminal outcome=Succeeded
  run_terminal outcome=Succeeded
REPAIR item node=Some(NodeId("A")) classification=UnchangedPending action=Preserve
REPAIR item node=Some(NodeId("B")) classification=UnchangedPending action=Preserve
REPAIR item node=Some(NodeId("C")) classification=UnchangedPending action=Preserve
REPAIR item node=Some(NodeId("F")) classification=UnchangedPending action=Preserve
REPAIR item node=Some(NodeId("G")) classification=UnchangedPending action=Preserve
REPAIR item node=Some(NodeId("JF")) classification=UnchangedPending action=Preserve
REPAIR item node=Some(NodeId("JG")) classification=UnchangedPending action=Preserve
REPAIR item node=Some(NodeId("done")) classification=UnchangedPending action=Preserve
REPAIR item node=Some(NodeId("entry-wait")) classification=UnchangedActive action=Preserve
REPAIR item node=Some(NodeId("pending-task")) classification=ChangedPending action=UseNewOnNextInvocation
REPAIR accepted=true active_wait_identity_preserved=true pending_task_unentered=true unrelated_crossing_unchanged=true
DIAGNOSTIC_FINISHED observed_case_errors=1; completion is not an acceptance pass
```
