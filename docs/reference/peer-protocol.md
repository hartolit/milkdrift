# Peer protocol v1.5

Version 1.5 carries bounded publication ancestry and nested process/model allowances alongside
origin-bound invocation authority. Terminal usage can report the internal admissions and artifact
bytes needed to settle a published service reservation. Direct calls retain request-bound input
transfer and host-invocation artifact provenance.
Both peers use this exact version in a coordinated deployment;
previous protocol generations and snapshot formats are refused. Current durable acceptance/replay
and tombstone contracts remain exact across restart.

`milkdrift-peer-protocol` is transport neutral. Every JSON control message uses a `ProtocolEnvelope` with selected `{major, minor}`, one typed message, and at most 32 explicitly ignorable DNS-namespaced extensions. Major 1/minor 5 is the only implemented version. It includes typed archived replay and observation-history dispositions and the exact queried request identity in every lookup result, so clients can bind authenticated responses to the URL they requested. Peers implementing earlier minors are rejected instead of guessing the changed shape or meaning. Unknown majors and unknown typed message fields fail closed. Decoding preflights encoded bytes, depth, container items, string/key sizes, duplicates, and document size before domain deserialization.

## Authentication and session

For two-daemon configuration and diagnostics, follow [peer operations](../operations/peers.md).
The [protocol package guide](../../crates/peer-protocol/README.md) follows a remote invocation;
this reference owns its message and retention contract.

HTTP bearer authentication maps current secret bytes to one configured `PeerId`; the handshake's
`claimed_peer` cross-checks that identity and cannot choose it. Each relationship's actions and
filters expand into an ordinary immutable authority grant. The service uses the shared evaluator
for session negotiation, catalog reads, invocation, observations, cancellation, and artifact transfer.
Authentication alone grants none of those operations.

Handshake returns the daemon session, selected version, lower offered hard limits, lease timing,
and lifecycle state without secrets or internal configuration. The current HTTP service reports
its supported feature flags rather than intersecting them with the request: resumable observations,
artifacts, and archived replay are enabled; incremental catalogs are disabled. The transport uses
complete catalog snapshots even though the protocol crate defines incremental-update messages.

Non-loopback endpoints must be HTTPS. HTTP is accepted only when `AllowInsecureLoopbackDevelopment` is explicit and the configured host is loopback/localhost. Redirects are disabled, endpoints are operator configured, CORS is absent, bodies/chunks are bounded, and request credentials are resolved at request time for rotation. Fixed one-minute request windows enforce the configured maximum independently for each authenticated peer and action/operation bucket; accepted exact invocation replays bypass fresh-work rate admission.

## Catalog

A complete `CatalogSnapshot` binds its generation, issue/expiry times, sorted descriptor entries,
invocable operations, and observations with a canonical BLAKE3 digest. The service requires a live
relationship and authorized catalog/health/profile reads, then projects current non-draining
generations with available observations through the relationship's capability scope. Empty
capability or operation allowlists advertise nothing. Invocation acceptance separately checks the
adapter's full resource requirements, current catalog, and capacity; a catalog entry is no reservation.

The consumer verifies digest/TTL and maps each remote `(PeerId, capability, descriptor revision, catalog generation/digest)` to a collision-resistant local identity/revision. Provider/category/schema facts are preserved, locality becomes `peer`, the configured trust zone and a `dev.milkdrift.peer/provenance` extension are added, and the adapter is registered normally. Renewal replaces the adapter even when the remote descriptor is unchanged; exact catalog replay preserves its registration. Replacement and disconnect drain old registrations, which are removed only after their host permits leave. Expired adapters report unavailable health and refuse new entry. Health observation timestamps retain the original measurement time. Same display names from different peers never merge.

## Invocation and idempotency

`ServingInvocationRequest` binds a caller-scoped `PeerRequestId`, exact catalog generation/digest,
remote `ResolvedCapabilitySnapshot`, provider-neutral `InvocationRequest`, deadline, quotas and
accepted authorization. Peer requests carry a targeted delegation with explicit direct or workflow
origin. Workflow origin retains real run/revision/node/execution/attempt coordinates. Client
authorization comes from the serving authentication owner and is refused on peer routes. The
canonical serving-v2 digest covers all these facts. The configured delegation is cross-checked
against the authenticated issuer, actor, target, capability, operation, limits and expiry. Durable
replay and accounting keys include the target host, caller realm and principal.

A controlled origin reserves the adapter's enforced request allowance in its ordinary final-entry
transaction. Delegation binds that reservation identity and the same limits to the exact accepted
request. Serving preparation must fit the allowance before entry. Process calls restrict model-token
and billed-cost use to not applicable/zero; this does not meter provider calls hidden inside a process.
Authenticated terminal usage settles the originating reservation, and imported output bytes charge
it atomically with publication. An uncertain outcome leaves the full unconsumed allowance outstanding,
including after restart; a wait timeout or missing reply does not settle it.

The serving daemon resolves an existing request identity before applying fresh-work availability, catalog, or capacity checks. It then uses one redb transaction to validate exact replay, relationship/catalog generations, the allowing authority decision, per-peer/global active limits, queue capacity, and hot-history capacity. That transaction may compact an eligible oldest-first terminal page before writing the primary record, request index, durable dispatch index, and accounting. Exact redelivery returns the same `PeerExecutionId` as either `accepted` hot history or `archived` compact history; same key/different digest always returns `idempotency_conflict`. A response loss therefore cannot create replacement provider/process work. This is exact submission idempotency, not a claim that arbitrary external side effects are globally exactly once.

Accepted work is `dispatch_available`. A fixed daemon-owned worker atomically claims it with a generation and lease, then durably records `entered` immediately before calling the exact adapter generation with the originating execution context. Restart or panic requeues a pre-entry claim; an entered claim is never automatically re-entered and becomes explicit uncertainty when terminal evidence is absent. Claims stop during drain, and worker handles are joined or reported retained at the shutdown deadline.

## Observations, reconnect, and cancellation

While an execution is hot, semantic observations are separate append-only checksummed rows contiguous from sequence one and map to progress, stream, artifact, terminal, or uncertainty. Pages use an exclusive `after_sequence` cursor and bounded limit; they never load or rewrite retained hot history. Every append advances a rolling observation-chain digest. SSE uses the same encoded observations plus independent transport keepalive comments; polling is equally resumable. The service reevaluates the exact peer execution scope on every page and bounded stream cycle; credential rotation/revocation emits `authorization_terminated` and stops future disclosure. Terminal closure is explicit.

After compaction, lookup and observation responses explicitly carry `history: archived`. The compact summary retains acceptance identity, immutable execution provenance and authority summary, cancellation facts, accounting, the observation count and rolling digest, and either the final terminal observation or a typed uncertain disposition. Intermediate progress is no longer available. A bounded manifest retains at most 256 named output observations with their original remote sequences and immutable references; the 257th output is refused before append. A client that encounters archival materializes any missing outputs through the same currently authorized transfer path and then consumes the final/uncertain disposition without reinvoking the capability. Local report sequences remain contiguous even when remote progress was compacted.

Before proven acceptance, clients retry only the same canonical request and query its key. After acceptance, clients resume the same execution and never submit replacement work. Missing accepted records become truthful uncertainty under the existing side-effect policy.

Cancellation names a separate request identity, exact remote execution, sequence, and reason. Acknowledgements are `accepted`, `rejected`, `unsupported`, `too_late`, or `unknown`, with terminal evidence when known. TCP close is never cancellation evidence. Late terminal evidence remains sequenced and cannot create two terminal facts.

## Artifacts

`GET /peer/v1/executions/{execution}/observations/{sequence}/artifact` returns an exact
`ArtifactMetadataOffer` for an owned durable output observation. The service checks the authenticated
relationship, execution ownership and download authority, including sensitivity and byte scope.
Unknown/non-output sequences and archived detail are unavailable. The offer binds the serving peer,
execution, content reference and expiry; the client verifies those before negotiation. Subsequent
chunks recheck current relationship authority. The origin imports verified output bytes through
its configured core artifact port before forwarding the observation. A missing or failed transfer
after acceptance preserves uncertainty and never causes a fresh peer submission.

Transfers bind either an accepted execution or an exact prospective input request, plus content
digest, size, media type, sensitivity, retention, provenance, authenticated source, direction,
expiry and transfer ID. Paths never select placement. Input staging rechecks the current catalog,
invocation allowance and upload authority before accepting bytes; it creates no execution record.
The origin freezes authorized artifact inputs and the causal manifest before entry, then stages
them against the final delegated request before submission. Workspace-value references are not
portable inputs and refuse before remote entry; use explicit inline or artifact references.

Upload chunks are sequential and bounded through ordinary artifact publication. Temporary inventory
supports resume/abort and stays invisible until size/digest verification and atomic metadata commit.
Imports wrap each foreign producer/cause in `PeerClaim`, retaining the authenticated source without
pretending foreign run IDs are local records. Nesting is bounded to four imports. Input ownership
is the host/peer pair, limited to 1,024 logical artifacts and the configured import byte ceiling
(10 GiB in daemon composition). Exact replay still succeeds when that cumulative quota is full.
Outputs bind the serving host invocation; controlled origin imports charge its existing reservation
in the same transaction as publication. Downloads use authorized bounded artifact reads. Transfer
failure after acceptance retains uncertainty and does not authorize a replacement invocation.

Large bytes use raw bounded HTTP content routes and never enter run events or semantic observation JSON. Publication metadata is the visibility boundary. No `peer-artifacts-v1` metadata/blob/temp tree exists.

## Retention

Active executions are never compacted. Terminal and uncertain executions remain hot through the configured observation-history horizon. A bounded oldest-first archival batch atomically inserts a compact tombstone, switches the authoritative execution/request location, removes hot observation rows and their peer observation-artifact mappings, removes the terminal/hot primary row, and advances independent counters. A crash exposes either the complete hot form or the complete tombstone; dual or missing placement fails closed. The new-invocation transaction performs the same reclaim step when the hot terminal bound is full, so completed history cannot permanently consume execution capacity while storage remains writable.

The tombstone is durable identity truth, not deletion: exact replay, digest conflict, acceptance time, execution provenance, authority summary, cancellation, resource accounting, terminal/uncertain disposition, observation count/digest, and archival generation/time survive. Core artifact bytes, metadata, ownership, retention, and producer provenance remain governed only by the ordinary artifact store and are not deleted by peer compaction. Tombstones have no automatic destructive expiry; operators rotate a fully retained store generation when physical disk policy requires it.
