# Status

This document owns current implementation facts and limitations.

## Implemented

- A stable Rust 2024 workspace with six safe-Rust packages and a curated lint/dependency policy.
- Private-invariant, bounded identities and versioned canonical JSON envelopes for capability descriptors and invocation contracts, including one shared bound large enough for every canonical workspace/artifact reference.
- Honest capability feature, cancellation, idempotency, side-effect, admission, locality, trust-zone, usage, and error representations; idempotent writes require an advertised external key scope, and mutable observations remain separate from immutable descriptors.
- Private immutable blueprint revisions with deterministic semantic digests, explicit ancestry, optimistic base checks, and atomic mutation batches.
- Semantic representations and validation for task, conditional branch, fork, join, reducer, explicit repeat, wait, signal, pinned subworkflow, and terminal nodes.
- Typed interfaces, ports, bindings, safe conditions and paths, closed mutations, bounded diagnostics, canonical schema-v1 fixtures, hostile-input tests, property tests, and privacy compile-fail documentation tests.
- One CI workflow for formatting, checks, tests, Clippy, rustdoc, and dependency policy.
- Versioned run commands with actor, idempotency identity, exact aggregate/sequence guard, boundary timestamps, bounded reasons, and durable exact command results.
- Checksummed schema-v1 append-only run events, pure deterministic projections, resumable event pages, checked snapshots, and immutable query/read models.
- Narrow persistence ports and a production redb adapter coordinating journal append, idempotency, workspace mutations/accounting, run/runnable/timer/lease indexes, and artifact-reference checks in one write transaction. Internal redb JSON values use family-bound, checksummed schema-v1 envelopes behind an explicit atomically migrated document-format-v2 marker: raw v0 is enveloped and backfilled directly, while enveloped v1 gains checked discovery and workspace-value accounting. Missing/lowered journal heads and missing or mismatched summary, workspace, discovery, lease, revision-digest, artifact-manifest/ownership/accounting rows are corruption rather than absence or replay success. Integrity inspection is bounded and resumable across revision, event, artifact, and derived-index families; health always compares runnable/timer/lease identity and ordered cardinalities with durable checked counts, verifies the workspace value/accounting shape, and degrades truthfully while a bounded completeness scan remains unfinished.
- Durable run-root, branch, repeat-iteration, and subworkflow workspace scopes with immutable versioned values, sibling isolation, and bounded fail-closed validation of complete parent/root and successor/inheritance/import provenance chains. A checked global and per-run workspace-value accounting document makes physical value deletion observable even when no surviving value refers to the deleted row.
- A filesystem content-addressed artifact store with BLAKE3 verification, bounded streaming writes/reads, atomic publication, deduplication, default-restricted sensitivity, retention/provenance metadata, and deterministic resumable orphan cleanup that preserves durable references.
- Deterministic eligibility/condition evaluation, physically bounded distinct-run runnable pages with last-scanned continuations, exact paired-index reads backed by durable active-count accounting, independently rotating bounded maintenance cursors, leases/heartbeats, cancellation intent, conservative retry/uncertainty policy, restart discovery, and graceful admission shutdown.
- Structured sequence, branch, fork/join, reducer, repeat, wait/timer/signal, owned subworkflow execution with explicit value import, and terminal behavior through a deterministic capability executor boundary.
- Pure prospective revision reconciliation using stable node/configuration/dependency identities, per-execution structured scopes, persisted classifications/decisions/action facts, and stale-plan guards.
- Focused ADRs for journal/projection ownership, redb/artifact transactions, side-effect truthfulness, and prospective reconciliation.

## Limitations

There is no live capability registry, provider/process/model adapter, causal context builder, secret store, authority-policy engine, peer transport, daemon, wire API, CLI, or desktop application. The included executor is deterministic test infrastructure, not a production provider. Workspace/artifact limits are enforced locally; provider cost/resource observations remain evidence and require Pass 3 policy integration for broader spend/resource authority. Stable filesystem orphan pages retain bounded candidates but enumerate the adapter-owned artifact directories. The synchronous redb adapter must be placed behind a bounded blocking owner when a future async daemon hosts it.
