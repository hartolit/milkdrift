# Identifier priority — production-runtime planning observation

Delta prepared this diagnostic; the coordinator inspected and ran it on 2026-10-10 against
`ff818e5b47ec7724a239fb0aa9b234880bca4c22`. It uses the production runtime and redb owner in a
throwaway temporary store, with fixture authority and a deterministic executor. The graph contains
only a branch and two successful terminals; no external task, provider, process or paid call enters.
No production source, schema, fixture or live data was changed.

Both branch predicates are true in both revisions. Destination node IDs and business intent remain
fixed: re-investigate the bad premise before considering cautious repair. The first variant assigns
port `a` to re-investigation; the second assigns it port `z`. The observed selected destination
changes. This demonstrates actual identifier coupling, not just a read of `BTreeMap` ordering.
It is evidence for explicit semantic arm order, not a claim that current replay is nondeterministic.

The coordinator ran exactly:

```sh
CARGO_TARGET_DIR=/home/hartolit/Projects/dev/milkdrift/target/remedial-planning-v3/priority-target cargo run --offline --manifest-path /tmp/milkdrift-priority-delta/Cargo.toml > /home/hartolit/Projects/dev/milkdrift/target/remedial-planning-v3/priority-probe.log 2>&1
```

Reported environment: rustc `1.95.0 (59807616e 2026-04-14)` and Cargo
`1.95.0 (f2d3ce0bd 2026-03-21)`. The build completed in 26.32 seconds. Exit succeeded, including
assertions for the exact selected destination, one eligible terminal and zero external dispatch.
The diagnostic printed:

```text
invalid_port=a cautious_port=z selected_port=a destination=reinvestigate expected_business_destination=reinvestigate business_outcome_matches=true
invalid_port=z cautious_port=a selected_port=a destination=cautious-repair expected_business_destination=reinvestigate business_outcome_matches=false
```

SHA-256 of executed manifest:
`6922a282db7091e2eb54ca30f7d4fd5d6d489d3f9f676c3793d7de7ceb50972c`.
SHA-256 of executed `main.rs`:
`6c845e16bbac52ee4925e4afcdb7806654f034f0ce2f8dc8f0e3809555c96b0e`.
SHA-256 of captured log:
`9360ad12bd3d212998b66dd46547942e6d3266d5d7765f532ef31f3c15ea7d6a`.
The manifest/source hashes were independently checked by Delta after the coordinator reported
execution. The exact source below preserves reproducibility if `/tmp` is cleared; it is a planning
observation, not maintained executable product input or a new acceptance fixture. Recreating it
requires updating absolute path dependencies if the checkout moves.

## Executed manifest

```toml
[package]
name = "milkdrift-priority-planning-probe"
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

## Executed Rust source

```rust
//! Read-only planning diagnostic against the production runtime and an isolated temporary store.
//! No external tasks exist. Fixture authority permits only this process's temporary test run.
use std::{collections::BTreeMap, sync::Arc};
use milkdrift_authority::{ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError,
    AuthorityEvaluator, DecisionReasonCode, GrantDigest, GrantId, PolicyId};
use milkdrift_blueprint::{AuthorRef, BlueprintRevision, BranchConfig, Condition, Edge, EdgeId,
    EdgeKind, Mutation, MutationBatch, Node, NodeId, NodeKind, PortId, TerminalOutcome,
    WorkflowId, WorkflowInterface};
use milkdrift_capability::{CapabilityDescriptorDocument, SideEffectClass};
use milkdrift_persistence::{CommandDisposition, Reason, RevisionStore, RunEventKind, RunJournal,
    WorkerId};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{CommandAuthorityClaim, DeterministicExecutor, ManualClock, RetryPolicy,
    RunCommand, RuntimeConfig, RuntimeService, SchedulerLimits, SequentialIdGenerator};
use milkdrift_workspace::{RunId, ScopeId, WorkspaceBudget, WorkspaceScope};
use tempfile::TempDir;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct ProbeAuthority;
impl AuthorityEvaluator for ProbeAuthority {
    fn evaluate(&self, request: &milkdrift_authority::AuthorityRequest)
        -> std::result::Result<AuthorityDecisionSnapshot, AuthorityError>
    {
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("test.priority-probe")?, 1, request.clone(),
            vec![DecisionReasonCode::Allowed],
            AuthorityBudget { artifact_bytes: Some(u64::MAX), ..AuthorityBudget::default() },
            SideEffectClass::Unknown,
        )
    }
}

fn revision(invalid_port: &str, cautious_port: &str) -> Result<BlueprintRevision> {
    // Both guards are identical and true. Business destinations and node IDs stay unchanged.
    let branch = Node::new(NodeId::new("choice")?, NodeKind::Branch {
        config: BranchConfig::new(BTreeMap::from([
            (PortId::new(invalid_port)?, Condition::Constant { value: true }),
            (PortId::new(cautious_port)?, Condition::Constant { value: true }),
        ]), None)?,
    })?.with_control_output(PortId::new(invalid_port)?)?
        .with_control_output(PortId::new(cautious_port)?)?;
    let terminal = |name: &str| -> Result<Node> {
        Ok(Node::new(NodeId::new(name)?, NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        })?.with_control_input(PortId::new("in")?)?)
    };
    let edge = |id: &str, port: &str, destination: &str| -> Result<Edge> {
        Ok(Edge::new(EdgeId::new(id)?, EdgeKind::Control, NodeId::new("choice")?,
            PortId::new(port)?, NodeId::new(destination)?, PortId::new("in")?))
    };
    Ok(BlueprintRevision::genesis(
        WorkflowId::new("priority-probe")?,
        MutationBatch::new(vec![
            Mutation::SetInterface { interface: WorkflowInterface::new([], [])? },
            Mutation::AddNode { node: branch },
            Mutation::AddNode { node: terminal("reinvestigate")? },
            Mutation::AddNode { node: terminal("cautious-repair")? },
            Mutation::AddEdge { edge: edge("invalid-route", invalid_port, "reinvestigate")? },
            Mutation::AddEdge { edge: edge("cautious-route", cautious_port, "cautious-repair")? },
        ])?, AuthorRef::new("human:priority-probe")?, "Identifier precedence planning diagnostic",
    )?)
}

fn run_case(invalid_port: &str, cautious_port: &str, expected_current: &str) -> Result {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "/home/hartolit/Projects/dev/milkdrift/crates/capability/tests/fixtures/descriptor-v1.json"
    ))?.body().clone();
    let executor = Arc::new(DeterministicExecutor::new(descriptor));
    let runtime = RuntimeService::new_with_authority(
        store.clone(), executor.clone(), Arc::new(ProbeAuthority), Arc::new(ManualClock::new(10000)),
        Arc::new(SequentialIdGenerator::new("priority-probe", 1)?),
        RuntimeConfig::new(WorkerId::new("worker-priority-probe")?, ActorRef::new("test:priority-probe")?,
            30000, 64, SchedulerLimits::new(8, 4, 2, 4)?, RetryPolicy::new(1, vec![], 10, 1000, 0)?)?,
    )?;
    let revision = revision(invalid_port, cautious_port)?;
    store.put_revision(&revision)?;
    let run = RunId::new("priority-run")?;
    let claim = CommandAuthorityClaim::new(GrantId::new("grant:priority-probe")?, 1,
        GrantDigest::new(format!("b3_{}", "0".repeat(64)))?, 0)?;
    for command in [RunCommand::CreateRun {
        workflow: revision.semantic().workflow().clone(), revision: revision.id().clone(),
        root_scope: WorkspaceScope::run_root(run.clone(), ScopeId::new("priority-scope")?),
        workspace_budget: WorkspaceBudget::new(128, 1048576, 1048576, 16, 1048576, 1048576)?,
        inputs: vec![],
    }, RunCommand::StartRun] {
        let document = runtime.command(run.clone(), ActorRef::new("human:priority-probe")?,
            store.head(&run)?, Reason::new("Planning-only runtime branch diagnostic")?, vec![], command)?;
        assert_eq!(runtime.handle_authorized_command(&document, &claim)?.result().disposition(),
            CommandDisposition::Accepted);
    }
    for _ in 0..8 {
        if runtime.projection(&run)?.is_completed() { break; }
        let tick = runtime.scheduler_tick()?;
        assert_eq!(tick.dispatched, 0, "This graph must contain no external task");
    }
    assert!(runtime.projection(&run)?.is_completed());
    let events = runtime.history(&run)?;
    let port = events.iter().find_map(|event| match event.kind() {
        RunEventKind::BranchRouteSelected { selected_port, .. } => Some(selected_port),
        _ => None,
    }).ok_or("No durable branch selection")?;
    let destination = revision.semantic().edges().values()
        .find(|edge| edge.source_node().as_str() == "choice" && edge.source_port() == port)
        .ok_or("No edge for selected port")?.target_node();
    assert_eq!(destination.as_str(), expected_current);
    let destinations: Vec<_> = events.iter().filter_map(|event| match event.kind() {
        RunEventKind::NodeBecameEligible { node, .. } if node.as_str() != "choice" => Some(node.as_str()),
        _ => None,
    }).collect();
    assert_eq!(destinations, vec![expected_current]);
    println!("invalid_port={invalid_port} cautious_port={cautious_port} selected_port={port} destination={destination} expected_business_destination=reinvestigate business_outcome_matches={}",
        destination.as_str() == "reinvestigate");
    Ok(())
}

fn main() -> Result {
    run_case("a", "z", "reinvestigate")?;
    run_case("z", "a", "cautious-repair")?;
    Ok(())
}
```
