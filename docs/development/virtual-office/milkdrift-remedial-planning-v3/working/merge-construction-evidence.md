# Exclusive-convergence construction evidence

Ada authored the prospective case and Rust probe; Rowan executed it against the current production
blueprint library on 2026-10-10 and returned the result. The inspected production source is the
unchanged source at `908e7893f5dadb84d12712573c8daaa946829e39`; later sprint commits change
documentation only. The integrated observation register owns execution/checkpoint qualification.

The independent expectation was: choose one arm, complete its work and continue once. The positive
control has separate successful terminals; the variation connects those same arm endpoints to
one shared successful terminal. No provider, runtime scheduler or permission result is mocked.
This observation tests **construction/validation only**, not runtime execution or a new merge.

Rowan ran the isolated package with its build output under `target/remedial-planning-v3/merge-build`.
The raw compiler and result output is `target/remedial-planning-v3/merge-probe.log`. Its result is:

```text
separate-terminals: accepted rev_f5ff07cabdb4c2e27e1a5a38a33c5d4ed176761a80d7dab433ec68740e7fe2a2
shared-continuation: refused Validation(ValidationError { diagnostics: [Diagnostic { code: AmbiguousControlFlow, location: "nodes.done-a.control_inputs", message: "only a join may receive multiple control edges", operation_index: Some(8), context: {} }] })
```

This confirms the [topology validator](../../../../../crates/blueprint/src/validation.rs) refuses
the ordinary direct convergence. It does not prove the broader runtime cannot express the outcome
through an explicit child-workflow construction. That alternative has its own identity, context,
authority and live-edit consequences. This is an expression limitation for the proposed operator
journey, not evidence of accepted history becoming corrupt.

## Reproduce without changing production source

Create an isolated temporary Rust package outside the workspace, using the manifest and source
below. Replace the absolute dependency path only if the checkout is elsewhere. Use the repository's
pinned Rust toolchain. The original isolated package resolved its own 25-package dependency lock;
it did not modify the repository lockfile. A repeat must record its resolved versions and source
checkpoint. The command is `cargo run --manifest-path PATH/Cargo.toml --target-dir OUTPUT`.

```toml
[package]
name = "milkdrift-merge-planning-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[dependencies]
milkdrift-blueprint = { path = "/home/hartolit/Projects/dev/milkdrift/crates/blueprint" }
```

```rust
use std::collections::BTreeMap;
use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, BranchConfig, Condition, Edge, EdgeId, EdgeKind, Mutation,
    MutationBatch, Node, NodeId, NodeKind, PortId, TerminalOutcome, WorkflowId,
    WorkflowInterface,
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn edge(id: &str, source: &str, port: &str, target: &str) -> Result<Edge> {
    Ok(Edge::new(
        EdgeId::new(id)?, EdgeKind::Control, NodeId::new(source)?, PortId::new(port)?,
        NodeId::new(target)?, PortId::new("in")?,
    ))
}

fn terminal(id: &str) -> Result<Node> {
    Ok(Node::new(NodeId::new(id)?, NodeKind::Terminal { outcome: TerminalOutcome::Success })?
        .with_control_input(PortId::new("in")?)?)
}

fn probe(shared_continuation: bool) -> Result<()> {
    let branch = Node::new(NodeId::new("choice")?, NodeKind::Branch {
        config: BranchConfig::new(
            BTreeMap::from([(PortId::new("a")?, Condition::Constant { value: true })]),
            Some(PortId::new("b")?),
        )?,
    })?.with_control_output(PortId::new("a")?)?.with_control_output(PortId::new("b")?)?;
    let wait = |id: &str| -> Result<Node> {
        Ok(Node::new(NodeId::new(id)?, NodeKind::Wait { duration_ms: 1 })?
            .with_control_input(PortId::new("in")?)?
            .with_control_output(PortId::new("out")?)?)
    };
    let mut nodes = vec![branch, wait("left")?, wait("right")?, terminal("done-a")?];
    if !shared_continuation { nodes.push(terminal("done-b")?); }
    let edges = vec![
        edge("choice-a", "choice", "a", "left")?,
        edge("choice-b", "choice", "b", "right")?,
        edge("left-done", "left", "out", "done-a")?,
        edge("right-done", "right", "out", if shared_continuation { "done-a" } else { "done-b" })?,
    ];
    let mut mutations = vec![Mutation::SetInterface { interface: WorkflowInterface::new([], [])? }];
    mutations.extend(nodes.into_iter().map(|node| Mutation::AddNode { node }));
    mutations.extend(edges.into_iter().map(|edge| Mutation::AddEdge { edge }));
    let label = if shared_continuation { "shared-continuation" } else { "separate-terminals" };
    let result = BlueprintRevision::genesis(
        WorkflowId::new(format!("probe-{label}"))?, MutationBatch::new(mutations)?,
        AuthorRef::new("human:planning-probe")?, "Read-only planning construction probe",
    );
    match result {
        Ok(revision) => println!("{label}: accepted {}", revision.id()),
        Err(error) => println!("{label}: refused {error:?}"),
    }
    Ok(())
}

fn main() -> Result<()> {
    probe(false)?;
    probe(true)?;
    Ok(())
```
