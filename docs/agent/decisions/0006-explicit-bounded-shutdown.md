# ADR-0006: Require explicit bounded shutdown

- **Status:** Accepted
- **Date:** 2026-07-22

## Context

The application owns native model workers and a synchronous Hugging Face resolver worker. Rust cannot safely terminate a thread while backend code holds mutable native model state. Blocking indefinitely in `Drop` is unsuitable for UI/process teardown, while detaching every worker silently loses observable cleanup and error handling.

## Decision

Normal application closure must invoke an explicit bounded shutdown operation. Shutdown requests cooperative worker termination, waits for ticketed completion where available, applies configured deadlines, joins completed workers, and reports timeout or detachment behavior.

A `Drop` implementation may perform best-effort nonblocking signaling, but it is not the primary shutdown contract and must not hide unbounded blocking. Deadlines must be constructed with checked arithmetic or validated upper bounds.

## Rejected alternatives

- **Rely on blocking `Drop`:** rejected because destructors cannot return errors and could freeze application closure indefinitely.
- **Detach workers on every close:** rejected because normal resource cleanup and completion become unobservable.
- **Force-kill Rust threads:** rejected because it can violate native resource, lock, and borrowing invariants.

## Consequences

- Every frontend/process integration must call shutdown on its normal closure path.
- Tests must cover active work, cancellation, timeout, completion, and abandoned callers.
- Uncooperative in-process backend calls can still outlive the bounded wait; process isolation is required for hard termination guarantees.
- Shutdown configuration is part of validated application policy.

## Review trigger

Review if inference or resolver work moves to a child process, upstream dependencies add reliable cancellation, or platform lifecycle constraints require a different bounded teardown protocol.
