# Peer protocol v1.0

`milkdrift-peer-protocol` is transport neutral. Every JSON control message uses a `ProtocolEnvelope` with selected `{major, minor}`, one typed message, and at most 32 explicitly ignorable DNS-namespaced extensions. Major 1/minor 0 is the only implemented version. Unknown majors and unknown typed message fields fail closed. Decoding preflights encoded bytes, depth, container items, string/key sizes, duplicates, and document size before domain deserialization.

## Authentication and session

HTTP bearer authentication maps current secret bytes to exactly one configured `PeerId` before a body is trusted. The handshake `claimed_peer` must equal that identity but cannot choose it. Handshake returns daemon session identity, selected version, feature intersection, hard limits, heartbeat/idle/execution lease policy, and ready/draining/shutdown state. It contains no secrets or internal configuration.

Non-loopback endpoints must be HTTPS. HTTP is accepted only when `AllowInsecureLoopbackDevelopment` is explicit and the configured host is loopback/localhost. Redirects are disabled, endpoints are operator configured, CORS is absent, bodies/chunks are bounded, and request credentials are resolved at request time for rotation. Fixed one-minute request windows enforce the configured maximum independently for each authenticated peer and action/operation bucket; accepted exact invocation replays bypass fresh-work rate admission.

## Catalog

A complete `CatalogSnapshot` has monotonic generation, issue/expiry boundaries, sorted exact descriptor entries, invocable operation subsets, filtered observations, draining state, and a domain-separated canonical BLAKE3 digest. The server starts from the live capability-host snapshot, then applies peer action, capability allow/deny, operation, side-effect, expiry, revocation, health, and quota policy. Empty allowlists advertise nothing.

The consumer verifies digest/TTL and maps each remote `(PeerId, capability, descriptor revision, catalog generation/digest)` to a collision-resistant local identity/revision. Provider/category/schema facts are preserved, locality becomes `remote`, the configured trust zone and a `dev.milkdrift.peer/provenance` extension are added, and the adapter is registered normally. Replacement, disconnect, drain, or expiry closes new resolution for the old generation. Same display names from different peers never merge.

## Invocation and idempotency

`PeerInvocationRequest` binds one local `PeerRequestId`, exact catalog generation/digest, remote `ResolvedCapabilitySnapshot`, provider-neutral `InvocationRequest`, deadline, quotas, and constrained opaque delegation. Its canonical request digest covers all those facts. The configured server-side delegation record is cross-checked against authenticated issuer, configured actor, target peer, capability, operation, request, limits, expiry, and nonce; claims cannot broaden the relationship.

The server atomically writes one `StoredExecution` file, syncs it, renames it, and syncs its directory before returning `Accepted`. Exact redelivery returns the same `PeerExecutionId`; same key/different digest returns `idempotency_conflict`. A response loss therefore cannot create replacement provider/process work. This is exact submission idempotency, not a claim that arbitrary external side effects are globally exactly once.

Before adapter entry, the store durably changes `accepted` to `running`. On restart an `accepted` record may enter once. A `running` record is never re-entered because a crash could have happened after the external boundary; it receives one explicit uncertain terminal instead.

## Observations, reconnect, and cancellation

Semantic observations are contiguous from sequence one and map to progress, stream, artifact, terminal, or uncertainty. Pages use an exclusive `after_sequence` cursor and bounded limit. SSE uses the same encoded observations plus independent transport keepalive comments; polling is equally resumable. Terminal closure is explicit. A slow consumer reads bounded pages from the durable log.

Before proven acceptance, clients retry only the same canonical request and query its key. After acceptance, clients resume the same execution and never submit replacement work. Missing accepted records become truthful uncertainty under the existing side-effect policy.

Cancellation names a separate request identity, exact remote execution, sequence, and reason. Acknowledgements are `accepted`, `rejected`, `unsupported`, `too_late`, or `unknown`, with terminal evidence when known. TCP close is never cancellation evidence. Late terminal evidence remains sequenced and cannot create two terminal facts.

## Artifacts

Transfers negotiate exact content digest, size, media type, source peer, remote execution, direction, expiry, and transfer ID before bytes. Paths and filenames never select placement. Upload chunks are sequential and bounded; downloads are bounded ranges. Temporary bytes are synced, exact size/digest verified, then renamed into a digest-derived blob and metadata is atomically published. Existing content is skipped only after size/digest verification. Abort removes incomplete temporary content. Secret/config media types are not automatically forwarded, and peer/action authority plus byte quotas apply before transfer.

Large bytes use raw bounded HTTP content routes and never enter run events or semantic observation JSON. Publication metadata is the visibility boundary: a verified orphan blob left by a fault is not downloadable or deduplicated until metadata is reverified and atomically published.
