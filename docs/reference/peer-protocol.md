# Peer protocol v1.0

`milkdrift-peer-protocol` is transport neutral. Every JSON control message uses a `ProtocolEnvelope` with selected `{major, minor}`, one typed message, and at most 32 explicitly ignorable DNS-namespaced extensions. Major 1/minor 0 is the only implemented version. Unknown majors and unknown typed message fields fail closed. Decoding preflights encoded bytes, depth, container items, string/key sizes, duplicates, and document size before domain deserialization.

## Authentication and session

HTTP bearer authentication maps current secret bytes to exactly one configured `PeerId` before a body is trusted. The handshake `claimed_peer` must equal that identity but cannot choose it. Each relationship's configured actions and filters are expanded once into an ordinary immutable authority grant, and every handshake, catalog/provider/health read, invocation, execution observation, cancellation, and artifact transfer is evaluated by the shared authority evaluator. Authentication never supplies fallback access. Handshake returns daemon session identity, selected version, feature intersection, hard limits, heartbeat/idle/execution lease policy, and ready/draining/shutdown state. It contains no secrets or internal configuration.

Non-loopback endpoints must be HTTPS. HTTP is accepted only when `AllowInsecureLoopbackDevelopment` is explicit and the configured host is loopback/localhost. Redirects are disabled, endpoints are operator configured, CORS is absent, bodies/chunks are bounded, and request credentials are resolved at request time for rotation. Fixed one-minute request windows enforce the configured maximum independently for each authenticated peer and action/operation bucket; accepted exact invocation replays bypass fresh-work rate admission.

## Catalog

A complete `CatalogSnapshot` has monotonic generation, issue/expiry boundaries, sorted exact descriptor entries, invocable operation subsets, filtered observations, draining state, and a domain-separated canonical BLAKE3 digest. The server starts from the live capability-host snapshot, then evaluates the relationship grant's capability identity, operation, provider profile, peer, side-effect, expiry, revocation, health, and quota facts before projection. Explicit empty capability/operation allowlists become deny-all scope and advertise nothing.

The consumer verifies digest/TTL and maps each remote `(PeerId, capability, descriptor revision, catalog generation/digest)` to a collision-resistant local identity/revision. Provider/category/schema facts are preserved, locality becomes `remote`, the configured trust zone and a `dev.milkdrift.peer/provenance` extension are added, and the adapter is registered normally. Replacement, disconnect, drain, or expiry closes new resolution for the old generation. Same display names from different peers never merge.

## Invocation and idempotency

`PeerInvocationRequest` binds one local `PeerRequestId`, exact catalog generation/digest, remote `ResolvedCapabilitySnapshot`, provider-neutral `InvocationRequest`, deadline, quotas, constrained opaque delegation, and originating run/revision/node/execution/attempt coordinates. Its canonical request digest covers all those facts. The configured server-side delegation record is cross-checked against authenticated issuer, configured actor, target peer, capability, operation, request, limits, expiry, nonce, and provenance; claims cannot broaden the relationship.

The serving daemon uses one redb transaction to validate exact replay, relationship/catalog generations, the allowing authority decision, per-peer/global active limits, queue capacity, and retained-record capacity. It writes the primary record, request index, durable dispatch index, and accounting together before returning `Accepted`. Exact redelivery returns the same `PeerExecutionId`; same key/different digest returns `idempotency_conflict`. A response loss therefore cannot create replacement provider/process work. This is exact submission idempotency, not a claim that arbitrary external side effects are globally exactly once.

Accepted work is `dispatch_available`. A fixed daemon-owned worker atomically claims it with a generation and lease, then durably records `entered` immediately before calling the exact adapter generation with the originating execution context. Restart or panic requeues a pre-entry claim; an entered claim is never automatically re-entered and becomes explicit uncertainty when terminal evidence is absent. Claims stop during drain, and worker handles are joined or reported retained at the shutdown deadline.

## Observations, reconnect, and cancellation

Semantic observations are separate append-only checksummed rows contiguous from sequence one and map to progress, stream, artifact, terminal, or uncertainty. Pages use an exclusive `after_sequence` cursor and bounded limit; they never load or rewrite retained history. SSE uses the same encoded observations plus independent transport keepalive comments; polling is equally resumable. The service reevaluates the exact peer execution scope on every page and bounded stream cycle; credential rotation/revocation emits `authorization_terminated` and stops future disclosure. Terminal closure is explicit.

Before proven acceptance, clients retry only the same canonical request and query its key. After acceptance, clients resume the same execution and never submit replacement work. Missing accepted records become truthful uncertainty under the existing side-effect policy.

Cancellation names a separate request identity, exact remote execution, sequence, and reason. Acknowledgements are `accepted`, `rejected`, `unsupported`, `too_late`, or `unknown`, with terminal evidence when known. TCP close is never cancellation evidence. Late terminal evidence remains sequenced and cannot create two terminal facts.

## Artifacts

Transfers negotiate exact content digest, size, media type, sensitivity, retention, provenance, source peer, remote execution, direction, expiry, and transfer ID before bytes. Paths and filenames never select placement. Upload chunks are sequential and bounded through the ordinary core artifact publication session. Core temporary inventory supports restart resume/abort and remains invisible until exact size/digest verification and atomic metadata commit. Imported provenance preserves the remote producer and adds origin peer/execution; ordinary content-addressed publication supplies deduplication, retention, and orphan cleanup. Downloads are authorized bounded ranges from the ordinary core artifact read port. Peer/action authority and byte quotas apply before transfer.

Large bytes use raw bounded HTTP content routes and never enter run events or semantic observation JSON. Publication metadata is the visibility boundary. No `peer-artifacts-v1` metadata/blob/temp tree exists.

## Retention

Terminal and uncertain records remain retained by default. An explicit bounded archival request may mark eligible records archived, but it does not delete request-id idempotency, acceptance/cancellation/security facts, append-only observations, or referenced artifact provenance. A configured total-record ceiling is non-evicting: exhaustion rejects new request identities. Destructive expiry requires a future reference-aware policy and is not implied by archival.
