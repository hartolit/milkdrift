# ADR 0002: Append-only run events and deterministic projections

- Status: accepted
- Date: 2026-08-18

## Context

Runs must survive restart, accept duplicated commands safely, and remain explainable after workflow definitions or live capability observations change. Storing a mutable run row would lose transition evidence and make recovery depend on whichever code last wrote the row. Letting workers write journal facts would also give adapters authority over workflow truth.

## Decision

Each run is one append-only aggregate with a single monotonically increasing sequence. A versioned command carries an idempotency identity, actor, exact aggregate, optimistic sequence guard, boundary-supplied timestamp, bounded intent, reason, and evidence. The runtime reads and projects the exact history, validates the command, and atomically commits closed schema-v1 event facts plus the durable command result and discoverability changes.

Only the runtime translates commands and authenticated worker reports into events. Workers never append arbitrary facts. Event envelopes have stable event identities, explicit schema versions, sequence numbers, bounded bodies, and integrity digests. Unknown versions, gaps, duplicates, and malformed history are errors rather than empty state. Exact command-result reads and idempotent replay validate the authoritative journal head, so a surviving result cannot turn missing or lowered history into success.

Run projections are pure folds over ordered events. The runtime verifies and folds exact history through fixed-size resumable pages rather than materializing an unbounded event vector. Replay does not read clocks, identifiers, registries, providers, files, databases, networks, or mutable globals. Snapshots are optional checked accelerators naming the exact covered sequence and digest; the journal remains authoritative.

## Rejected alternatives

- Mutable run and node rows as primary truth, because partial updates and hidden state make replay and audit unreliable.
- Eventual coordination between event append, idempotency, and indexes, because a crash can expose contradictory acceptance states.
- Worker-authored events, because adapters would be able to decide workflow transitions.
- Re-evaluating branches or capability selection during replay, because mutable inputs would change history.

## Consequences

Commands can be retried without duplicating semantic facts, recovery has one sequence authority, and read models can be rebuilt independently. Event evolution requires explicit migration and golden fixtures. More facts are recorded, but large content remains in the artifact store and events carry bounded references.

## Reconsideration triggers

Reconsider the physical journal encoding if measured workloads exceed the local adapter's transaction or replay bounds. Do not reconsider append-only truth or deterministic projection unless an equally auditable model proves the same crash, idempotency, and compatibility invariants.
