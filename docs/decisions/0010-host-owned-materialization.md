# ADR 0010: Host-owned materialization and artifact publication

- Status: accepted
- Date: 2026-08-26

## Context

A process needs files, while invocation contracts contain immutable workspace/artifact references
and adapters must not receive database handles or learn redb/filesystem content layout.

## Decision

`milkdrift-capability-host` owns `InvocationDataAccess`. Its production `RuntimeStore` bridge reads
exact workspace versions, reads verified artifact chunks under explicit authority, creates a
private temporary execution root, copies only selected inputs to validated relative paths, and
publishes declared regular files or bounded captures through `ArtifactStore`. Publication uses a
stable invocation/output/content identity, run budget accounting, restricted sensitivity, the
invocation producer, exact durable input causes, and execution provenance. The local-process
adapter receives only the opaque workspace lease, canonical root, selected paths, and
capability-domain artifact references.

Output traversal, absolute paths, dot components, symlinks, hardlinks, special files, excessive
depth/count/bytes, digest mismatch, and paths escaping the root are rejected. Incomplete
publication streams are aborted; a crash can leave only artifact-store-owned orphan candidates.
Undeclared files are ignored and disappear with the temporary workspace.

## Rejected alternatives

- Giving the process adapter `RuntimeStore` or redb, because persistence layout would leak across
  the concrete capability boundary.
- Passing canonical artifact filesystem paths, because that bypasses read authority and digest
  verification.
- Importing the complete execution tree, because undeclared or hostile outputs would become
  durable implicitly.

## Consequences

Materialization and publication can be tested independently with the production store. The
current contract imports regular files only; directory manifests, sparse files, and stronger
directory-relative no-follow handles require an explicit future schema rather than implicit
behavior.

## Reconsideration triggers

Add a new materialization schema when directory artifacts or platform-native openat/capability
handles are required and can preserve the same authority, bound, and provenance guarantees.
