# ADR-0006: Require explicit bounded shutdown

- **Status:** Accepted
- **Date:** 2026-07-22
- **Amended:** 2026-07-31

## Context

The application owns native model workers and a synchronous Hugging Face resolver worker. Rust cannot safely terminate a thread while backend code holds mutable native model state. Blocking indefinitely in `Drop` is unsuitable for UI/process teardown, while discarding an unresolved join handle after a timeout or another failure loses observable cleanup and prevents a later caller from confirming termination.

## Decision

Normal application closure must invoke an explicit bounded shutdown operation. Shutdown requests cooperative worker termination, waits for ticketed completion where available, applies configured deadlines, and joins workers whose completion is observed.

Shutdown explicitly tracks `Running`, `Stopping`, `Stopped`, and `FailedOrRetryable` (failed-or-retryable). An unresolved worker's join handle remains owned across signaling failures, timeouts, and other pre-join errors; only an attempted join after observed thread completion consumes it. A later shutdown call retries unresolved shutdown work, including each pending join, under fresh configured bounds. Shutdown may return idempotent success only in `Stopped`, after every owned worker has been confirmed stopped and joined. `FailedOrRetryable` is not success merely because a stop request was sent.

Startup is transactional across worker creation. If a later worker fails to start after an earlier worker has started, startup attempts cooperative shutdown and join rollback for the partial worker set under configured finite bounds before returning the primary startup failure. If that rollback bound expires, the unresolved worker owner enters a private process-level cleanup quarantine and a later startup retries one quarantined cleanup; rollback must neither introduce an unbounded wait nor detach the retained join handle.

A `Drop` implementation may perform best-effort nonblocking signaling, but it is not the primary shutdown contract and must not hide unbounded blocking. Shutdown and join timeouts are validated before worker creation with a conservative 24-hour ceiling, and runtime deadlines retain checked arithmetic as defense in depth.

## Rejected alternatives

- **Rely on blocking `Drop`:** rejected because destructors cannot return errors and could freeze application closure indefinitely.
- **Discard unresolved join handles after a deadline or error:** rejected because worker completion becomes unobservable and a later bounded retry cannot join the worker.
- **Force-kill Rust threads:** rejected because it can violate native resource, lock, and borrowing invariants.

## Consequences

- Every frontend/process integration must call shutdown on its normal closure path.
- Tests must cover active work, cancellation, timeout, retained handles, repeated shutdown calls, state transitions, partial-start rollback, completion, and abandoned callers.
- Uncooperative in-process backend calls can still outlive one bounded wait; their handles remain owned for retry, and process isolation is required for hard termination guarantees.
- Shutdown configuration is part of validated application policy.

## Review trigger

Review if inference or resolver work moves to a child process, upstream dependencies add reliable cancellation, or platform lifecycle constraints require a different bounded teardown protocol.
