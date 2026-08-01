# ADR-0006: Require explicit bounded shutdown

- **Status:** Accepted
- **Date:** 2026-07-22
- **Amended:** 2026-08-01

## Context

The application owns native model workers and a synchronous Hugging Face resolver worker. Rust cannot safely terminate a thread while backend code holds mutable native model state. Blocking indefinitely in `Drop` is unsuitable for UI/process teardown, while discarding an unresolved join handle after a timeout or another failure loses observable cleanup and prevents a later caller from confirming termination.

E0 also has a fail-closed backend contract: when finite explicit cleanup is exhausted while native ownership remains, ordinary implicit model destruction is not established as safe. The worker deliberately abandons the complete runtime allocation with `std::mem::forget` after publishing the structured shutdown error. The thread can then terminate and be joined, but the retained backend allocation is reclaimed only when the process exits. E1 must not later infer clean shutdown merely from the absence of a worker handle.

## Decision

Normal application closure must invoke an explicit bounded shutdown operation. Shutdown requests cooperative worker termination, waits for ticketed completion where available, applies configured deadlines, and joins workers whose completion is observed.

Application shutdown explicitly distinguishes `Running`, `Stopping`, `Stopped`, retryable failure, and terminal failure. An unresolved worker's join handle remains owned across signaling failures, timeouts, and other pre-join errors; only an attempted join after observed thread completion consumes it. A later shutdown call retries unresolved shutdown work, including each pending join, under fresh configured bounds. A join timeout is retryable and may be followed by clean success after the previously successful worker shutdown is joined.

A failed ticketed E0 shutdown is terminal because the E0 worker stops after publishing its result. E1 retains the structured `RuntimeError` independently from join-handle ownership, continues bounded joins, returns the normalized structured failure on the first call, and returns that retained failure on every later call. Handle absence is not evidence that backend cleanup succeeded. An endpoint disconnection without an already retained clean shutdown result likewise does not independently prove clean shutdown.

The E0 terminal disposition is named `RetainUntilProcessExit`. It is selected only when explicit shutdown failed and the runtime still owns backend resources. The worker then deliberately forgets the complete runtime rather than invoking an unverified implicit backend drop. This is not a cleanup retry mechanism: no owner remains after the worker exits, and process termination is the reclamation boundary.

Shutdown may return idempotent success only in `Stopped`, after every owned worker has been confirmed stopped and E0 clean completion has been observed.

Startup is transactional across worker creation. If a later worker fails to start after an earlier worker has started, startup attempts cooperative shutdown and join rollback for the partial worker set under configured finite bounds before returning the primary startup failure. If that rollback bound expires, the unresolved worker owner enters a private process-level cleanup quarantine and a later startup retries one quarantined cleanup; rollback must neither introduce an unbounded wait nor detach the retained join handle.

A `Drop` implementation may perform best-effort nonblocking signaling, but it is not the primary shutdown contract and must not hide unbounded blocking. Shutdown and join timeouts are validated before worker creation with a conservative 24-hour ceiling, and runtime deadlines retain checked arithmetic as defense in depth.

## Rejected alternatives

- **Rely on blocking `Drop`:** rejected because destructors cannot return errors and could freeze application closure indefinitely.
- **Discard unresolved join handles after a deadline or error:** rejected because worker completion becomes unobservable and a later bounded retry cannot join the worker.
- **Force-kill Rust threads:** rejected because it can violate native resource, lock, and borrowing invariants.

## Consequences

- Every frontend/process integration must call shutdown on its normal closure path.
- Tests must cover active work, cancellation, retryable timeout, retained handles, clean repeated shutdown, sticky terminal cleanup failure, model-drop suppression, endpoint abandonment, state transitions, partial-start rollback, and completion.
- Uncooperative in-process backend calls can still outlive one bounded wait; their handles remain owned for retry, and process isolation is required for hard termination guarantees.
- After E0 cleanup exhaustion, worker-thread termination and process-level backend-resource reclamation are distinct facts. E1 can join the terminal worker, but only process termination reclaims the deliberately abandoned allocation.
- Shutdown configuration is part of validated application policy.

## Review trigger

Review if inference or resolver work moves to a child process, upstream dependencies add reliable cancellation, or platform lifecycle constraints require a different bounded teardown protocol.
