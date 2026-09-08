# milkdrift-blueprint

Use this library to describe a workflow and create immutable revisions of it. Authors,
importers, and the control service supply nodes, ports, edges, and mutations; the library
validates the resulting graph before publishing a revision. It does not start tasks or
store execution state. [Architecture](../../docs/architecture.md) explains how the runtime,
persistence, authority, and capability owners use these definitions.

## Start with a definition

The [crate introduction](src/lib.rs) has a runnable Rust example that constructs an interface
and terminal node, puts them in a `MutationBatch`, and creates a `BlueprintRevision::genesis`.
An edit creates another revision; it does not change a previously published definition.
Missing targets, incompatible bindings, or cycles fail graph validation. Applying a revision
to a live run is a separate runtime/control operation that preserves completed history.

For an external task, begin with [`TaskConfig`](src/model/node.rs). Its capability requirement
describes the operation needed; its [`TaskContextPolicy`](src/context.rs) asks which inputs and
earlier evidence the runtime may consider. `TaskConfig::direct_inputs` chooses the default policy.
`TaskConfig::new` attaches an explicit one. The policy's rustdoc example constructs a review node
that also requests two levels of ancestor evidence, retains the default exclusions, and limits
selection to eight items. Declare data ports and bindings separately when assembling the graph.

## Follow a policy into an attempt

1. [Policy construction](src/context.rs) checks selector and budget bounds. Additional node,
   role, category, and exact-source selectors can request evidence beyond direct inputs.
   Exclusions take precedence, and selecting a source does not grant access to it.
2. For a task resolved to a model or process capability,
   [runtime dispatch](../runtime/src/engine/dispatch.rs) discovers historical/workspace metadata
   and [selects context](../runtime/src/context/selection.rs) under the task's policy, branch
   visibility, authority, and budgets. Other capability categories currently skip this path.
3. The runtime saves a [`ContextManifest`](../model/src/context.rs) as an artifact before
   dispatch. It records actual selection, omissions, source identities and digests, totals,
   and the policy digest. A retry retains that selection rather than scanning newer history.
4. [Materialization](../runtime/src/context/source/materialize.rs) and the
   [capability host](../capability-host/src/materialization.rs) supply the selected content.
   Model adapters include it as evidence; process profiles must explicitly map context inputs
   to files. The manifest does not replace the task's declared input bindings.

With the default `fail_closed = true`, an unavailable, denied, or over-budget required candidate
fails active selection before dispatch. Optional candidates can be omitted with a reason. A
malformed exact reference or oversized manifest also fails preparation. See the policy's
`fail_closed` documentation for the precise checks: the current `StopAtFirstOverflow` mode can
skip later required candidates. Use `OmitOversized` when relying on required-item checks.

Session intent is part of the policy, but the runtime currently does not enforce it against the
adapter request. Model session selection is supplied separately, and current endpoint mappings
accept only fresh requests. Choosing a continuation policy alone does not create a session.

## API detail and verification

This package has no feature flags and needs no endpoint or daemon for definition construction.
Generate API documentation with `cargo doc -p milkdrift-blueprint --no-deps --open`.
[ADR 0011](../../docs/decisions/0011-causal-context-manifests.md) explains the context design;
[operator examples](../../examples/operator/README.md) show complete workflows using the daemon.

From the repository root:

```sh
cargo test -p milkdrift-blueprint --test kernel --all-features
cargo test -p milkdrift-blueprint --doc --all-features
cargo test -p milkdrift-runtime --test causal_context --all-features
```

Use the [verification policy](../../docs/development/workflow.md#choose-verification-for-the-change)
for additional checks required by a change.
