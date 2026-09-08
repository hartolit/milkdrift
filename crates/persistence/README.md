# milkdrift-persistence

This package defines what must be saved together so a runtime decision can be recovered.
[Runtime](../runtime/README.md) decides which events a command produces; storage adapters
implement the synchronous traits here. Change this package when callers and storage need a
different durable contract, not to add database mechanics.

## Commit a command and recover its reply

Accepting a new run must save its creation event, root scope, initial values, usage, discovery
summary, and response together. Otherwise restart could find a run without its inputs, or a
caller could lose the response to work already accepted.

Runtime packages that decision as an [`AtomicRunCommitRequest`](src/journal/commit.rs).
Its [`CommandReceipt`](src/journal/receipt.rs) binds command/run/actor identity to canonical
intent and retains a complete audit document. The request constructor checks that events,
result, workspace mutations, and derived accounting agree. `RunJournal::commit_command`
then checks current storage and saves the whole transaction.

```text
planned command -> receipt + result + events + workspace/accounts + indexes
                                              |
                                         atomic commit
                                              |
                               saved result survives a lost reply
```

A matching saved receipt returns `Replayed` before a new delivery's optimistic sequence is
checked. Changed intent under the same identity conflicts. A new command returns `Committed`;
inspect its disposition because a durable rejection has a response but no semantic events.
The runtime receipt excludes delivery time and sequence from intent. The daemon's
[`ApplicationCommandStore`](src/application.rs) separately binds the complete canonical
external request to its original response.

An I/O error can arrive after commit. Preserve the command identity and intent, read
`RunJournal::command_result` and compare fingerprints, or redeliver the same request.
A read failure does not prove absence. A sequence, usage, lease, or account conflict requires
the runtime to reread state and reconsider its plan. A run with a writable artifact reservation
may be busy until that publication commits or aborts.

Artifact bytes are published before a journal event can reference them. An optional snapshot
is saved after the append that commits its payload commitment. Neither operation can replace
command history. The [redb adapter](../../adapters/redb-store/README.md) implements these
transactions; `RunJournal` owns the detailed obligations for another implementation.

## Select the port for the operation

| Reader or writer's task | Contract and consequence |
| --- | --- |
| Store a definition | [`RevisionStore`](src/revision.rs) verifies immutable bytes and ancestry. Storing a revision does not adopt it into a run. |
| Read history or discover work | [`RunQueryStore`](src/journal/query.rs) supplies verified event pages and derived discovery. Follow continuations even after an empty filtered page. |
| Read logical workspace state | [`WorkspaceStore`](src/journal/query.rs) resolves exact scopes and value versions. Writes accompany their owning journal events. |
| Publish or read artifact content | [`ArtifactStore`](src/artifact.rs) owns resumable chunks, verified publication, accounting, and bounded reads. Authorization proof and content integrity are separate requirements. |
| Reserve cumulative resources | [Controller accounts](src/controller_account.rs) change with entry/terminal events or artifact publication. Read totals through `ControllerAccountStore`; do not maintain a separate counter. |
| Accelerate recovery | [`SnapshotStore`](src/snapshot.rs) stores optional checkpoints verified against journal commitments. Invalid optional checkpoints can be discarded for replay. |
| Retain external replies, layouts, proposals, or audit | [Application ports](src/application.rs) keep exact receipts, independent presentation state, derived proposal discovery, and bounded audit. |
| Save remote acceptance and worker ownership | [`PeerExecutionStore`](src/peer.rs) binds canonical requests, claims, observations, and retained dispositions. The serving peer owns these facts separately from the origin run. |
| Remember observed time | [`ClockWatermarkStore`](src/clock.rs) atomically compares an observation with durable high-water evidence. A rejected rollback must not be used as current time. |
| Inspect storage integrity | [`StorageAdmin`](src/admin.rs) exposes schema, sampled health, and resumable read-only scrubs. Health is not a complete historical scan. |

Persistence queries are internal storage ports. Authentication and authority filtering belong
to their boundary callers; a cursor or stored value is not a grant. Corrupt evidence remains
an error rather than an empty result. Pages bound individual work and memory, not the total
time required to traverse a store.

Retention follows the owning family. Application receipt archival preserves exact requests and
responses in cold storage; peer tombstones retain acceptance and final disposition with less
observation detail. Audit may evict its oldest rows. Runtime projection compaction and optional
snapshot discard leave journal history intact. Artifact cleanup acts only under its separate
ownership and retention checks.

## Verify a change

This package needs no database setup or feature selection to construct documents. Run
`cargo test -p milkdrift-persistence --all-features` for their contracts, and use the
redb contract suite to verify implementation obligations. [Development workflow](../../docs/development/workflow.md)
owns focused commands and the checks required for each kind of change.

Failure/reopen tests establish the tested software boundaries. They do not establish filesystem
power-loss behavior or exactly-once external effects: command acceptance cannot prove an external
outcome. Runtime interprets durable entry, terminal, cancellation, and uncertainty observations
before permitting another attempt.
