# ADR 0008: Generation-safe live capability hosting

- Status: accepted
- Date: 2026-08-26

## Context

The semantic capability crate already described immutable descriptors, exact resolution
snapshots, invocations, observations, and cancellation. It deliberately owned no adapter handle,
health lifecycle, permit, or registry. A mutable registry in the semantic crate would invert
dependencies, while resolving again at execution time could silently move persisted work to a
different provider or descriptor revision.

## Decision

`milkdrift-capability-host` is an outer embeddable package implementing runtime `TaskExecutor`.
It registers adapters by capability identity and nonzero descriptor revision, keeps descriptors
immutable, separates bounded observations and actual held permits, and selects only current,
non-draining, fresh, available generations for new resolution. Selection uses one locked
snapshot and stable priority/identity/revision ordering after semantic, authority, budget, and
capacity filtering.

A persisted snapshot dispatches and cancels only through its exact generation. No fallback is
allowed after that point. Admission is immediate and has no hidden queue. A permit and invocation
owner are installed immediately before adapter entry and released by RAII on success, rejection,
error, or panic. Draining removes a generation from new resolution but retains it for exact
pinned work until an explicit zero-owner reap. Forced removal reports unresolved invocation
identities and makes later dispatch fail with a typed unavailable-generation error.

## Rejected alternatives

- Keeping only the latest adapter handle, because old persisted work would be rerouted.
- An unbounded in-memory queue, because the durable runtime is already the queue owner.
- Using observation load as permit truth, because observations can be stale or inaccurate.
- Letting adapters evaluate authority or mutate run state, because those decisions belong to
  the host and runtime respectively.

## Consequences

Concrete process/model/peer adapters can remain narrow and provider-focused. Registry and
generation counts are explicit configuration bounds. Pre-entry overload/unavailability is
distinguished from post-entry uncertainty, and panic cannot leak a permit. Shutdown first closes
admission, then drains, and refuses graceful completion while owners remain. Adapter startup is a
bounded reserved registry phase: it is never selectable, concurrent duplicates cannot start a
second generation, and shutdown reports the pending registration until that caller completes
cleanup. Every adapter declares its own start, drain, and shutdown semantics and passes the shared
capability-host conformance suite.

## Reconsideration triggers

Add a small host queue only if it has explicit ownership, cancellation, timeout, and shutdown
semantics and demonstrates a need the durable runtime cannot satisfy. Add asynchronous adapter
interfaces only with the same exact-generation and permit-release invariants.
