# Status

This document owns current implementation facts and limitations.

## Implemented

- A stable Rust 2024 workspace with six safe-Rust packages and a curated lint/dependency policy.
- Private-invariant, bounded identities and versioned canonical JSON envelopes for capability descriptors and invocation contracts.
- Honest capability feature, cancellation, idempotency, side-effect, admission, locality, trust-zone, usage, and error representations; mutable observations remain separate from immutable descriptors.
- Private immutable blueprint revisions with deterministic semantic digests, explicit ancestry, optimistic base checks, and atomic mutation batches.
- Semantic representations and validation for task, conditional branch, fork, join, reducer, explicit repeat, wait, signal, pinned subworkflow, and terminal nodes.
- Typed interfaces, ports, bindings, safe conditions and paths, closed mutations, bounded diagnostics, canonical schema-v1 fixtures, hostile-input tests, property tests, and privacy compile-fail documentation tests.
- One CI workflow for formatting, checks, tests, Clippy, rustdoc, and dependency policy.
- Versioned run commands with actor, idempotency identity, exact aggregate/sequence guard, boundary timestamps, bounded reasons, and durable exact command results.
- Checksummed schema-v1 append-only run events, pure deterministic projections, resumable event pages, checked snapshots, and immutable query/read models.
- Narrow persistence ports and a production redb adapter coordinating journal append, idempotency, workspace mutations/accounting, run/runnable/timer/lease indexes, and artifact-reference checks in one write transaction.
- Durable run-root, branch, repeat-iteration, and subworkflow workspace scopes with immutable versioned values and sibling isolation.
- A filesystem content-addressed artifact store with BLAKE3 verification, bounded streaming writes/reads, atomic publication, deduplication, default-restricted sensitivity, retention/provenance metadata, and orphan cleanup.
- Deterministic eligibility/condition evaluation, bounded/fair admission contracts, leases/heartbeats, cancellation intent, conservative retry/uncertainty policy, restart discovery, and graceful admission shutdown.
- Structured sequence, branch, fork/join, reducer, repeat, wait/timer/signal, owned subworkflow execution with explicit value import, and terminal behavior through a deterministic capability executor boundary.
- Pure prospective revision reconciliation using stable node/configuration/dependency identities, per-execution structured scopes, persisted classifications/decisions/action facts, and stale-plan guards.
- Focused ADRs for journal/projection ownership, redb/artifact transactions, side-effect truthfulness, and prospective reconciliation.

## Limitations

There is no live capability registry, provider/process/model adapter, causal context builder, secret store, authority-policy engine, peer transport, daemon, wire API, CLI, or desktop application. The included executor is deterministic test infrastructure, not a production provider. Workspace/artifact limits are enforced locally; provider cost/resource observations remain evidence and require Pass 3 policy integration for broader spend/resource authority. The synchronous redb adapter must be placed behind a bounded blocking owner when a future async daemon hosts it.
