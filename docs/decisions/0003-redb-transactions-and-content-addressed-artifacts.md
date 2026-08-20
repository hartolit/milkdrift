# ADR 0003: Redb transactions and content-addressed artifact ownership

- Status: accepted
- Date: 2026-08-18

## Context

The durable runtime needs immutable revision lookup, atomic journal acceptance, recovery indexes, workspace state, and large artifact content. A generic database abstraction would leak storage mechanics inward, while embedding large task output in events or database values would weaken bounds and duplicate content. Database transactions cannot by themselves atomically commit filesystem bytes.

## Decision

`milkdrift-persistence` owns narrow durable documents and ports. `milkdrift-redb-store` implements them with named, versioned redb tables and validated byte encodings; redb handles, transactions, and table names never cross the adapter boundary. One write transaction coordinates accepted events, command results, aggregate head, artifact-reference checks, and recovery/discoverability indexes. Expected sequence is checked against the aggregate head inside that transaction.

Immutable revisions are stored as canonical documents and verified on read. Internal table documents use canonical schema-v1 envelopes whose BLAKE3 checksums bind both family and payload. The explicit internal document-format marker is v3. Raw v0 and enveloped v1/v2 stores are semantically validated and migrated in one restart-safe transaction; unknown future schema/document versions are refused. One mandatory checked integrity-root document anchors fixed-depth domain-separated authenticated catalogs. Exact membership and absence checks bind run/event/command, revision, discovery, workspace, snapshot, and artifact rows to that root while `RUN_HEADS` remains the sole event-sequence authority. The catalogs are integrity metadata, not copied semantic counts.

Artifact bytes use BLAKE3 content addressing under paths derived only from validated digests. Publication streams through a bounded temporary file, verifies size and digest, flushes and synchronizes it, then atomically renames and synchronizes the containing directory before metadata becomes committed. Journal events may reference only committed metadata. A crash before metadata commit leaves removable orphan content; it cannot leave an accepted event pointing to missing bytes. Reads are bounded and verify size and digest. Sensitivity is restricted by default, and authorization is required before export unless metadata explicitly marks content public.

Administrative integrity pages and orphan-cleanup effects are bounded, deterministic, and resumable through opaque exclusive cursors. Integrity cursors bind the authenticated root, record family, and artifact-content verification mode. Derived-index phases cross-check both directions of journal discovery, workspace ancestry, revision digests, artifact manifests/ownership, and cumulative unique-digest byte accounting. Runnable, timer, and lease queries enumerate authenticated catalogs and validate the corresponding identity/ordered rows; a symmetric deletion therefore cannot masquerade as an empty active set. Workspace reads and writes iteratively validate the selected value's complete successor/inheritance/import chain plus every owning scope's parent/root chain under explicit depth limits. Per-run `WorkspaceUsage` remains canonical budget state and authenticated workspace-domain membership binds it to its immutable budget. Product-created artifact paths are recorded durably before create or rename and retained until unlink and directory synchronization complete; cleanup pages that inventory directly, rechecks publication ownership, and performs bounded filesystem work outside the redb writer transaction.

## Rejected alternatives

- An in-memory product backend, because process teardown would discard the system's authoritative responsibilities.
- A generic key/value or repository framework, because it obscures transaction boundaries and leaks database concepts into core code.
- Large blobs in events, workspace JSON, or redb values, because it duplicates data and undermines bounded replay.
- Recording artifact metadata before content durability, because accepted history could permanently reference missing bytes.

## Consequences

Local runs can be reopened from a fresh process, and all database-owned acceptance facts share one crash boundary. Artifact publication has a deliberate intent/create/rename/commit ordering with safe, resumable orphan cleanup; a caller can continue large integrity and cleanup walks without reprocessing the same logical keys. Pre-v3 migration performs one bounded-protocol legacy residue discovery before publishing v3; current-format cleanup does not claim arbitrary externally injected files. The in-database integrity root detects partial deletion, insertion, rekeying, and payload mutation within an owned current-format database. Detecting replacement or rollback of the entire database, or an adversary able to rewrite every row and recompute an unkeyed root, requires a future host-held keyed or monotonic anchor. The adapter is blocking and must later be owned behind a bounded host executor if called from an asynchronous daemon.

## Reconsideration triggers

Reconsider redb only for demonstrated host, scale, or corruption-recovery requirements it cannot satisfy. A replacement must preserve the narrow ports, exact atomicity, schema refusal, single sequence authority, and artifact publication ordering.
