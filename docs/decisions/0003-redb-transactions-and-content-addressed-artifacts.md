# ADR 0003: Redb transactions and content-addressed artifact ownership

- Status: accepted
- Date: 2026-08-18

## Context

The durable runtime needs immutable revision lookup, atomic journal acceptance, recovery indexes, workspace state, and large artifact content. A generic database abstraction would leak storage mechanics inward, while embedding large task output in events or database values would weaken bounds and duplicate content. Database transactions cannot by themselves atomically commit filesystem bytes.

## Decision

`milkdrift-persistence` owns narrow durable documents and ports. `milkdrift-redb-store` implements them with named, versioned redb tables and validated byte encodings; redb handles, transactions, and table names never cross the adapter boundary. One write transaction coordinates accepted events, command results, aggregate head, artifact-reference checks, and recovery/discoverability indexes. Expected sequence is checked against the aggregate head inside that transaction.

Immutable revisions are stored as canonical documents and verified on read. Opening storage records an explicit schema version, refuses unknown future schemas, classifies corruption separately from absence, and relies on redb's single-writer ownership. Supported migrations are explicit and restart-safe rather than implicit table guessing.

Artifact bytes use BLAKE3 content addressing under paths derived only from validated digests. Publication streams through a bounded temporary file, verifies size and digest, flushes and synchronizes it, then atomically renames and synchronizes the containing directory before metadata becomes committed. Journal events may reference only committed metadata. A crash before metadata commit leaves removable orphan content; it cannot leave an accepted event pointing to missing bytes. Reads are bounded and verify size and digest. Sensitivity is restricted by default, and authorization is required before export unless metadata explicitly marks content public.

## Rejected alternatives

- An in-memory product backend, because process teardown would discard the system's authoritative responsibilities.
- A generic key/value or repository framework, because it obscures transaction boundaries and leaks database concepts into core code.
- Large blobs in events, workspace JSON, or redb values, because it duplicates data and undermines bounded replay.
- Recording artifact metadata before content durability, because accepted history could permanently reference missing bytes.

## Consequences

Local runs can be reopened from a fresh process, and all database-owned acceptance facts share one crash boundary. Artifact publication has a deliberate two-phase ordering with safe orphan cleanup. The adapter is blocking and must later be owned behind a bounded host executor if called from an asynchronous daemon.

## Reconsideration triggers

Reconsider redb only for demonstrated host, scale, or corruption-recovery requirements it cannot satisfy. A replacement must preserve the narrow ports, exact atomicity, schema refusal, single sequence authority, and artifact publication ordering.
