# milkdrift-workspace

A workflow needs to pass an exact input to a task and keep the results separate when work forks.
This package describes that logical workspace: scopes identify who owns a value, versions preserve
earlier values, and artifact references identify content stored elsewhere. A workspace here is
not a directory; the [capability host](../capability-host/src/materialization.rs) turns selected
values into files when a process needs them.

## Let a branch use an input without changing its parent

A `WorkspaceValueReference` names a run, scope, key, and exact version. There is no implicit
“latest” lookup in a reference. `ScopeLineage` is the complete root-to-leaf chain used to check
workspace visibility. Its leaf can read ancestor values and owns new versions of its own streams.
Another branch's local stream is outside that chain.

```text
root: request v1
  |
  +-- branch A: inherited request v1 --> successor v2
  |
  +-- branch B: can still read root's request v1
```

The [crate example](src/lib.rs) constructs this relationship. Use `WorkspaceValueEntry::initial`
for a new stream, `inherited` for a branch-local version derived from an ancestor, and `successor`
to advance that local stream. `imported` starts a local stream from an exact value in another run,
such as a declared subworkflow result. These constructors check the shape of the record; runtime
and persistence establish the source's existence, lineage or import relationship, and authority.
Constructing a reference alone makes none of those facts durable.

## Pass a reference to large content

Use `WorkspaceValue::Json` for small bounded structured values and `WorkspaceValue::Artifact` for
separately stored bytes. An `ArtifactReference` binds a logical artifact ID to its content digest,
size, and media type. Two records can refer to identical bytes while retaining distinct identities
and provenance. `verifies` checks bytes against a reference; it does not fetch or publish them.
The [artifact-reference example](src/artifact.rs) demonstrates that check.

`ArtifactMetadata` adds sensitivity, retention, and provenance. The producer identifies how the
artifact arose; causal references identify the inputs used to produce it. The
[persistence artifact port](../persistence/src/artifact.rs) owns publication and verified reads.
The host uses it to publish process/model outputs with their invocation and input provenance.
An artifact's classification does not bypass the daemon's authenticated read path.

## Account for accepted values

`WorkspaceBudget` calculates whether another value version or artifact admission fits. Each
version counts, even if its value is an artifact reference. Inline JSON bytes and artifact-content
bytes have separate counters. An artifact-valued entry consumes a value version; its referenced
artifact must also be admitted to the run's artifact accounting. Persistence avoids charging an
already admitted exact artifact twice within that run.

The budget methods return proposed `WorkspaceUsage` without writing anything. The owner commits
usage with the corresponding durable change, preventing two callers from independently spending
the same remaining allowance. Limits are inclusive, zero permits no use of that resource, and
overflow or excess returns `WorkspaceError` rather than wrapping a counter.

The [contract tests](tests/contracts.rs) cover sibling isolation, imports, invalid versions,
artifact verification, and accounting. This package has no feature flags or service prerequisites;
follow the [verification policy](../../docs/development/workflow.md#choose-verification-for-the-change)
when changing its documentation or contracts.
