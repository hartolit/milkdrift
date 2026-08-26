# ADR 0018: Durable peer acceptance and truthful disconnect uncertainty

## Status

Accepted.

## Context

HTTP response loss can occur after a remote daemon accepts work. Retrying with a new execution would duplicate process/model/tool entry, while treating socket closure as failure or cancellation would invent evidence. The runtime already owns workflow state and exact capability snapshots, so peer transport must not become a second scheduler or shared truth store.

## Decision

One remote peer is an ordinary local `CapabilityAdapter`. Before reporting acceptance, the server persists authenticated owner, canonical request digest, exact catalog/descriptor snapshot, stable remote execution identity, acceptance time, lease, and empty observation log behind `PeerExecutionStore`. Exact key/digest replay returns that record; key/different digest conflicts.

Adapter-entry intent is persisted before the external call. Accepted-before-entry records may enter after restart. Running records are not re-entered after restart because the crash boundary is ambiguous; they terminate with explicit uncertainty. Observations append contiguously and resume by cursor. Cancellation has an independent request/acknowledgement record; connection closure proves nothing. Existing runtime side-effect/idempotency policy decides whether an uncertain attempt may later retry.

## Consequences

Response loss and reconnect do not duplicate accepted work. Non-idempotent uncertainty remains visible for operator/controller resolution. A crash immediately after entry-intent but before actual entry may conservatively report uncertainty and under-execute; this is safer and more truthful than duplicate side effects. No globally exactly-once external-effect claim is made. Peer persistence stays outside `peer-protocol` and no redb/HTTP type enters runtime or persistence cores.
