# ADR 0018: Durable peer acceptance and truthful disconnect uncertainty

## Status

Accepted.

## Context

HTTP response loss can occur after a remote daemon accepts work. Retrying with a new execution would duplicate process/model/tool entry, while treating socket closure as failure or cancellation would invent evidence. The runtime already owns workflow state and exact capability snapshots, so peer transport must not become a second scheduler or shared truth store.

## Decision

One remote peer is an ordinary local `CapabilityAdapter`. The serving daemon stores peer execution through the narrow persistence port implemented by its existing redb owner. One transaction checks exact request replay, relationship and catalog generations, per-peer/global active counts, dispatch capacity, and retention capacity, then writes the accepted primary record, request index, queue index, and accounting. Exact key/digest replay returns that record; key/different digest conflicts.

The durable phases are `DispatchAvailable`, `DispatchClaimed`, `Entered`, `CancellationRequested`, `Terminal`, and `Uncertain`; record existence is durable acceptance. Claims have worker, generation, and lease facts. Entry is a distinct CAS immediately before the adapter call. Fixed daemon-owned workers claim the durable queue and are joined or reported retained at shutdown. Restart requeues claims without entry evidence and converts entered claims without terminal evidence to uncertainty. A worker panic follows the same boundary rule.

Primary records retain the exact canonical request, delegated origin run/revision/node/execution/attempt, allowing authority decision, cancellation facts, bounded accounting, and retention state. Observations are separate contiguous checksummed rows with an artifact-reference index, so cursor reads are bounded and appends do not rewrite history. Terminal/uncertain rows use a time index. Explicit archival marks retained facts archived without deleting the idempotency tombstone, security facts, observations, or provenance; reaching the configured total-record ceiling rejects new identities rather than evicting evidence.

Cancellation request, acknowledgement, adapter support, terminal evidence, and connection closure are independent facts. Cancellation before entry is durably terminalized without invoking the adapter. After entry, disconnect cannot prove cancellation. Late terminal evidence follows the same idempotent append path and may resolve uncertainty without creating a second terminal fact.

Peer artifact transfer is an adapter over the ordinary core `ArtifactStore` and authorized read port. Incomplete inbound bytes use the core publication session/temp inventory, remain invisible until exact size/digest verification and commit, resume or abort through that owner, and retain sensitivity, retention, remote provenance, and origin peer/execution. Outbound ranges read core artifacts. There is no peer-owned blob, metadata, or temp repository.

## Consequences

Response loss and reconnect do not duplicate accepted work. Non-idempotent uncertainty remains visible for operator/controller resolution, and no globally exactly-once external-effect claim is made. The origin still owns workflow truth; the serving daemon owns only its durable remote execution record and ordinary local artifacts. HTTP/redb types remain outside the transport-neutral protocol contracts.

This pre-release change advances redb physical schema 4 to 5 and internal document format 7 to 8. Older/future stores are refused. Daemon startup also explicitly refuses the obsolete `peer-executions-v1` and `peer-artifacts-v1` prototype directories; no partial importer is claimed.
