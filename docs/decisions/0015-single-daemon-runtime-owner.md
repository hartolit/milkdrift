# ADR 0015: One daemon and one bounded runtime owner

- Status: accepted
- Date: 2026-08-26

## Context

The runtime and redb adapter are deliberately synchronous, stateful owners. Putting them behind an
`Arc<Mutex<RuntimeService>>` in async route handlers would spread durable ownership across reactor
tasks, permit unbounded waiting, and risk holding a global lock while process/model work streams.
Letting each client open redb would create multiple command, recovery, and scheduler authorities.

## Decision

One `milkdrift-daemon` process owns the redb/artifact root, runtime, control service, authority
evaluator, capability registry, adapter generations, effect workers, external-command receipts, and
layout state. ADR 0022 supersedes the original sidecar storage detail: receipts and layouts now use
narrow redb-backed application ports. Startup is ordered: validate configuration and secret
references; open storage; recover runtime and application state with admission closed; register and
health-check adapters; recover peer work; start bounded effect workers; resume admission; then
report readiness.

All synchronous runtime, redb, artifact, control, and registry queries cross one bounded
`sync_channel` into a dedicated owner thread. Axum tasks own only HTTP/SSE work and receive a stable
overload response when that queue is full. External adapter entry stays on fixed
`EffectWorkerHost` threads; the runtime owner only claims, records, and observes effects. A bounded
blocking notification interval drives scheduler/effect maintenance without busy polling.

The daemon is the single shutdown owner. It closes mutation admission and readiness first, begins
peer/runtime drain, applies the configured drain/cancel/retain effect policy with a deadline, joins
workers and the owner thread, and then drops storage. Crash recovery uses the existing durable
runtime semantics plus ADR 0022 application receipts; no indispensable execution truth lives in
HTTP tasks or stream buffers.

## Rejected alternatives

- One redb/runtime instance per handler or client, because durable ordering and recovery would have
  multiple owners.
- An async mutex around `RuntimeService`, because framework task scheduling would become part of
  runtime correctness and external work could monopolize the lock.
- Unbounded async or blocking queues, because overload would become memory growth and shutdown
  latency.
- Moving Tokio/Axum types into runtime or persistence, because transport does not own semantics.

## Consequences

HTTP concurrency is intentionally higher than durable-operation concurrency. Commands retain
runtime ordering and optimistic checks, while queue capacity and effect concurrency are explicit
configuration. Long process/model streams cannot block the reactor or hold the owner boundary.
Application state shares the daemon's redb ownership and no longer has a whole-file sidecar. Storage
format migration remains deliberately unsupported as specified by ADR 0022.

## Reconsideration triggers

Replace the owner thread only if the persistence/runtime ports gain a proven transaction-safe
concurrent implementation with equivalent bounded overload, recovery ordering, and shutdown
tests. Async convenience alone is not sufficient.
