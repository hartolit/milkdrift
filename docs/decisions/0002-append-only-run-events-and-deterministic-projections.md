# ADR 0002: Append-only run events and deterministic projections

- Status: accepted
- Date: 2026-08-18

## Context

Runs must survive restart, accept duplicated commands safely, and remain explainable after workflow definitions or live capability observations change. Storing a mutable run row would lose transition evidence and make recovery depend on whichever code last wrote the row. Letting workers write journal facts would also give adapters authority over workflow truth.

## Decision

Each run is one append-only aggregate with a single monotonically increasing sequence. A versioned command carries an idempotency identity, actor, exact aggregate, optimistic sequence guard, boundary-supplied timestamp, bounded intent, reason, and evidence. The runtime reads and projects the exact history, validates the command, and atomically commits closed schema-v1 event facts plus the durable command result and discoverability changes.

Only the runtime translates commands and authenticated worker reports into events. Workers never append arbitrary facts. Event envelopes have stable event identities, explicit schema versions, sequence numbers, bounded bodies, and integrity digests. Unknown versions, gaps, duplicates, and malformed history are errors rather than empty state. Exact command-result reads and idempotent replay validate the authoritative journal head, so a surviving result cannot turn missing or lowered history into success.

Run projections are pure folds over ordered events. The runtime verifies and folds exact history through fixed-size resumable pages rather than materializing an unbounded event vector. Replay does not read clocks, identifiers, registries, providers, files, databases, networks, or mutable globals. The same fold deterministically compacts settled operational detail after every event, so command planning sees bounded active state rather than a lifetime read model. Exact history is exposed through stable-cursor journal pages.

Ownership is explicit: the journal is complete immutable history; the active projection is bounded operational state; a snapshot is an optional bounded recovery checkpoint; historical read models reconstruct or query paged journal facts. Projection snapshot schema v2 names its exact covered sequence and cumulative history digest and serializes the live-compacted projection without first cloning a historical projection. Invalid, mismatched, corrupt, v1, or otherwise unsupported optional checkpoints are discarded and replayed rather than guessed or migrated.

Operational compaction performs no durable retention or deletion. Run events and artifact/output records remain in their authoritative stores, and compact summaries retain stable provenance and reference anchors so later timeline, inspection, or causal-context consumers can reconstruct evidence from journal pages.

## Rejected alternatives

- Mutable run and node rows as primary truth, because partial updates and hidden state make replay and audit unreliable.
- Eventual coordination between event append, idempotency, and indexes, because a crash can expose contradictory acceptance states.
- Worker-authored events, because adapters would be able to decide workflow transitions.
- Re-evaluating branches or capability selection during replay, because mutable inputs would change history.

## Consequences

Commands can be retried without duplicating semantic facts, recovery has one sequence authority, and historical read models can be rebuilt independently. Command validation and candidate construction clone only the bounded active projection where atomic validation needs a disposable candidate. Event evolution requires explicit migration and golden fixtures. More facts are recorded, but large content remains in the artifact store and events carry bounded references.

This does not promise universal constant memory. Active branches, unresolved or retained effects, current output/context/artifact references, workflow shape, and explicit workspace limits remain in scope. Settled retry attempts collapse to a total-attempt count plus the last facts needed to validate the current retry; recovery, continuation, progress, lease, timer, signal, and reconciliation histories remain in the journal.

## Reconsideration triggers

Reconsider the physical journal encoding if measured workloads exceed the local adapter's transaction or replay bounds. Do not reconsider append-only truth or deterministic projection unless an equally auditable model proves the same crash, idempotency, and compatibility invariants.
