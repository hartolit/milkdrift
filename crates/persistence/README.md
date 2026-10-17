# milkdrift-persistence

This package defines the facts that must be saved together so accepted work survives reopening.
[Runtime](../runtime/README.md) owns task meaning and legal state changes; storage adapters implement
these synchronous ports without scheduling tasks or interpreting model output.

## Accept work and recover its reply

Ordinary daemon tasks use [`WorkflowStore`](src/task.rs). Runtime supplies one bounded document
containing retained definitions, frozen starts, observations and results. A compare-and-swap commit
saves that document together with discovery metadata, capacity reservations and the exact
actor-scoped request receipt. Typed entry and managed-execution facts let storage enforce artifact
provenance and resource holds in the same transaction, without becoming a second task state machine.

Receipt lookup requires the authenticated actor and original request key. Identical canonical bytes
return the saved result before stale-generation or capacity checks; changed bytes conflict. An I/O
error may arrive after commit. Read the receipt or redeliver the exact request to discover acceptance;
an error alone does not establish rollback or prove that an external action did not occur.

Each accepted task reserves its remaining document growth, output allowance and mandatory stop
receipt. A server-issued stop identity can consume its reserved slot once. Other submissions cannot
spend that slot, and arbitrary keys do not create additional emergency capacity. History and accepted
keys are never evicted to admit new work. The storage usage query exposes the same counters used by
admission, including pending artifact writes and retained content bytes.

Discovery uses bounded pages and requires no previously retained workflow ID. The public boundary
filters those pages against current grants. Storage cursors and recorded actor identities do not
grant access, and corruption remains an error rather than an empty result.

## Select the durable owner

| Operation | Contract and consequence |
| --- | --- |
| Accept or change ordinary work | [`WorkflowStore`](src/task.rs) saves one generation and its exact command result atomically. |
| Name the daemon | [`DaemonMetadataStore`](src/daemon.rs) binds one shared display name to the installation, with guarded changes and exact actor-scoped replay. A startup seed never overwrites an accepted rename. |
| Publish or read artifact content | [`ArtifactStore`](src/artifact.rs) owns resumable chunks, verified publication, accounting and bounded reads. Content integrity and read authorization are separate requirements. |
| Hold a native directory | [`NativePathClaimStore`](src/native_paths.rs) atomically claims complete physical path sets; equal or nested writers conflict, while readers may share. |
| Protect a managed installation | [Managed resource ports](src/managed.rs) preserve frozen descriptor bindings, active uses, physical fences, independent evaluation evidence and update/removal guards. |
| Save remote acceptance and worker ownership | [`PeerExecutionStore`](src/peer.rs) retains canonical requests, claims, observations and compact terminal dispositions independently of any originating workflow. |
| Retain security audit | [`SecurityAuditStore`](src/security.rs) preserves a bounded ordered window with monotonic sequence identities. |
| Remember observed time | [`ClockWatermarkStore`](src/clock.rs) compares observations against durable high-water evidence; a rejected rollback cannot be used as current time. |
| Inspect storage | [`StorageAdmin`](src/admin.rs) exposes schema, sampled health and resumable integrity scans. A health sample is not a full historical scan. |

Native claims do not expire on elapsed time or restart. The host captures canonical physical roots;
runtime releases claims only after durable settlement. An uncertain writer continues to block
conflicting work. Managed holds likewise distinguish physical stop evidence from a workflow's logical
outcome; neither a cancellation request nor an uncertain outcome proves that a process stopped.

Artifact publication saves its intent before accepting chunks and verifies content before exposing a
committed reference. Task artifacts require the owning workflow's durable entry proof. Cumulative
output ceilings cover committed and pending output, while global capacity includes unrelated uploads
and retained content. Aborting a publication releases its reservation without manufacturing a result.

Peer tombstones retain acceptance and final disposition while eligible detailed observations retire.
Audit may evict its oldest rows. Artifact cleanup follows independent ownership and retention checks.
These mechanisms do not remove accepted ordinary task history or request identities.

## Verify a change

Run `cargo test -p milkdrift-persistence --all-features` for contract checks and the
[redb suite](../../adapters/redb-store/README.md) for durable implementation obligations.
The [development workflow](../../docs/development/workflow.md) owns focused commands and required gates.
Failure/reopen tests establish tested software boundaries, not filesystem power-loss behavior or
exactly-once external effects.
