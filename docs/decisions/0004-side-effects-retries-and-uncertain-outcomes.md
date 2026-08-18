# ADR 0004: Truthful side effects, retries, and uncertain outcomes

- Status: accepted
- Date: 2026-08-18

## Context

A process can stop after an external service accepted work but before the runtime durably observed the result. A lost connection or expired lease therefore does not prove success or failure. Claiming exactly-once effects, or retrying every failure automatically, could duplicate non-idempotent writes.

## Decision

Every dispatch records a stable invocation identity, attempt identity, idempotency identity when supported, exact resolved capability snapshot, advertised side-effect class, and durable lease before external work begins. Executor reports are observations submitted through typed runtime commands; they cannot downgrade the conservative side-effect risk or directly decide run state.

Pure and read-only work may retry within an explicit bounded policy. Idempotent writes may retry only when the operation advertises idempotency and every attempt propagates the same stable key. Non-idempotent and unknown writes are not automatically repeated after an ambiguous dispatch boundary. Connection loss, process loss, or lease expiry after such a boundary produces a durable uncertain obligation.

Retry policy bounds attempts, retryable error classes, durable backoff timers, recorded jitter, and resource budgets. Cancellation is a request whose acknowledgement and terminal boundary are recorded separately. Operator/controller retry, query, compensation, remediation, retention, or resolution decisions append new events with actor, reason, and evidence; attempt history is never rewritten.

## Rejected alternatives

- Exactly-once external-effect claims, because Milkdrift cannot atomically commit a local transaction with arbitrary remote systems.
- Retrying solely from an adapter's `retryable` boolean, because workflow policy and effect safety remain runtime responsibilities.
- Treating lease expiry or disconnect as failure, because the external outcome can remain unknown.
- Editing a failed attempt after compensation, because remediation is new causal work.

## Consequences

Some runs deliberately stop in an inspectable uncertain state and require authority before progress. Idempotent executors receive stable keys. Recovery is conservative, but it does not invent outcomes or silently duplicate writes.

## Reconsideration triggers

An adapter may provide stronger evidence, receipts, or query operations, but those become recorded facts. Reconsider automatic retry only for a class with a demonstrated end-to-end idempotency boundary; never infer it from transport success alone.
