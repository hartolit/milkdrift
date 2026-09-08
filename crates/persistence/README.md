# milkdrift-persistence

This library defines what Milkdrift asks storage to save and what a storage implementation must
return after interruption. The runtime decides which commands and events are valid; persistence
ties each saved result to its events, workspace changes, and records used to find unfinished work.
The [redb adapter](../../adapters/redb-store/src/lib.rs) implements these synchronous traits.
Persistence itself opens no database and starts no tasks or workers.

## Follow a command into storage

Consider creating a run with an input value. Saving only its creation event would leave recovery
with a run whose input is missing. Saving only the input would leave data without the event that
explains why it exists. The command's response must also survive, so a client can repeat delivery
after losing the first response.

1. The [runtime command](../runtime/src/command.rs) produces a `CommandReceipt`. It retains the
   complete canonical audit document and fingerprints the command identity, run, actor, and
   canonical intent. Runtime intent includes reason, evidence, command content, and, for an
   authorized command, the exact authority claim. Delivery time and the runtime's optimistic
   sequence stay in the audit document. The external API's complete-request replay contract is
   separately owned by the [application receipt port](src/application.rs).
2. The [runtime commit planner](../runtime/src/engine/command_planning/commit.rs) derives events,
   workspace scope/value mutations, accounting, and discovery changes from the proposed next
   state. It supplies these with the receipt and result to `AtomicRunCommitRequest::new`.
   The constructor checks consistency and bounds without accessing storage: event sequences
   must follow the expected head, result event IDs must match, and mutations must exactly
   materialize the scopes and values introduced by those events.
3. [`RunJournal::commit_command`](src/journal/commit.rs) checks saved command identity before
   testing a new request's optimistic sequence. An existing matching receipt returns its original
   result; changed intent conflicts. For a new command, the storage implementation checks the
   current sequence, workspace usage, any lease/account guards, and referenced artifacts.
4. The [redb append implementation](../../adapters/redb-store/src/journal/append.rs) commits the
   receipt/result, contiguous events and history links, run head, workspace mutations/accounting,
   and discovery changes in one write transaction. Attached controller-account changes and
   projection commitments participate in that transaction. A discovery index tells recovery
   where work may remain; it cannot replace journal evidence of what happened.

Artifact bytes must already have been published through [`ArtifactStore`](src/artifact.rs).
The journal commit validates their committed references and charges newly referenced artifacts
to the run. It does not upload their content. Similarly, a projection commitment binds checkpoint
bytes to this append; the runtime saves the optional snapshot separately afterward. Failure to
save that snapshot does not undo the command; recovery can replay the journal.

The [receipt documentation](src/journal/receipt.rs) includes an executable example showing why
changed delivery metadata can retain the same runtime intent fingerprint while changed intent
cannot. `CommandReceipt` and `CommandResultDocument` document construction and decoding;
`AtomicRunCommitRequest` documents the inputs that must agree. These are storage-facing APIs:
applications submit commands through the daemon's authority/runtime path.

## Interpret the response and a lost response

`AtomicRunCommitOutcome::Committed` means a previously unseen command's result was saved. Inspect
its `CommandDisposition`: an accepted command has events, while a rejected command has a durable
result with no semantic events or workspace/index changes. A rejection can therefore replay too.
`Replayed` returns the original result without another append, even if the supplied sequence is
now stale. Replay still checks the integrity of stored evidence.

An error is different from a durable rejection. Constructor errors happen before any write.
Sequence, workspace-usage, lease, or account revision conflicts require the runtime to reread
the relevant state and reconsider its plan. An idempotency conflict means the command identity
already belongs to another intent; changing its key is not a recovery strategy for a lost reply.
Missing artifact content must be resolved before a referencing event can be accepted.
For a new append, the redb implementation also returns `Storage` with `OwnerBusy` while an artifact
publication holds a reservation for that run. Finish or abort it through its owning port before
retrying the journal operation; changing the usage guard does not release that reservation.

A storage error can arrive **after** commit. Preserve the command identity and canonical intent.
Read `RunJournal::command_result`, compare its fingerprint with the receipt, or redeliver the
same request through `commit_command`. A found result settles command acceptance; a read failure
does not establish absence. `RunQueryStore::events` supplies bounded, verified pages of the
supporting history. Corrupt evidence is an error, never a reason to treat the run as empty.

Command replay does not call an adapter again. External work may already have happened before
a later observation fails to save. Only the runtime can interpret entry, terminal, cancellation,
and uncertainty facts and decide whether another attempt is permitted.
[ADR 0004](../../docs/decisions/0004-side-effects-retries-and-uncertain-outcomes.md) explains this
distinction; [status](../../docs/product/status.md) owns current qualification and limits.

## Entry points and verification

This package has no feature flags and requires no database setup to construct its documents.
Use the traits needed by the consumer: `RevisionStore` stores definitions, `RunJournal` commits
runtime commands, `RunQueryStore` reads history/discovery, and `WorkspaceStore` reads saved scopes
and values. Artifact, application, peer, account, clock, snapshot, and administrative ports have
separate obligations in their API documentation. Concrete storage lifecycle belongs to the
adapter and [daemon](../../docs/operations/daemon.md).

Generate API documentation with `cargo doc -p milkdrift-persistence --no-deps --open`.
[Architecture](../../docs/architecture.md) explains ownership across packages;
[ADR 0003](../../docs/decisions/0003-redb-transactions-and-content-addressed-artifacts.md) explains
the transaction design and limits of local integrity checks. For ordinary daemon use, start with
the maintained [operator examples](../../examples/operator/README.md).

From the repository root:

```sh
cargo test -p milkdrift-persistence --all-features
cargo test -p milkdrift-redb-store --test contracts --all-features command_fault_boundaries_are_atomic_and_replayable
cargo test -p milkdrift-redb-store --test contracts --all-features journal_reopens_and_idempotency_conflicts_without_duplicate_events
cargo test -p milkdrift-runtime --test durable_runtime --all-features
```

The storage fault test distinguishes failures before commit from an error after commit. The
reopen test checks retained history and conflicting redelivery; runtime tests check recovery and
durable authority denial. These tests do not establish filesystem power-loss behavior or
exactly-once external effects. Use the [verification policy](../../docs/development/workflow.md#choose-verification-for-the-change)
to select the remaining checks for a change.
