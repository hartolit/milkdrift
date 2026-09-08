# Executing through a peer

This package connects two configured Milkdrift hosts over authenticated HTTP. On the origin,
`PeerRegistry` turns a remote catalog into ordinary local capability registrations. On the serving
host, `PeerService` durably accepts requests and runs them on its capability host. Workflow
scheduling remains with the origin's runtime; remote acceptance is a separate durable record.

Operators should start with [peer operations](../../docs/operations/peers.md). For a production
consumer trace, read the daemon's [peer construction](../../apps/daemon/src/host/peers.rs) and
[two-daemon tests](../../apps/daemon/tests/two_daemon_peer.rs). The
[protocol crate](../../crates/peer-protocol/README.md) explains the message meanings independently
of these HTTP and worker mechanics.

## Configure and connect

Each direction has an explicit endpoint, expected peer identity, credential source, and relationship
scope. The service expands relationship actions and resource filters into an ordinary authority
grant. Empty capability or operation allowlists expose no work. Allowed capabilities must also fit
the relationship's filesystem, network, secret, effect, and budget scope at execution entry.

`PeerHttpClient::new_with_credential_source` rereads the credential for each request. `new` wraps
the already resolved credential in a static source, which is useful for direct library callers and
tests. The service can likewise receive a request-time `PeerAuthenticator`; payload identity fields
never replace its result. HTTPS is required except for explicit loopback development. The router
itself supplies Axum routes under `/peer/v1`; deployment owns the listener and any TLS termination.

`PeerRegistry::connect` handshakes, verifies the catalog, and registers filtered remote generations
with the local host. Catalog renewal can create a new local generation even when remote executable
facts are unchanged, because the exact catalog/expiry changed. Replacement and disconnect drain
old registrations until their held permits leave. Expiry prevents new entry; it does not cancel
already accepted work. Connection management is explicit—there is no discovery or NAT traversal.

## Accept, execute, and reconnect

`PeerService` constructors create workers with admission closed. Register local adapters, then call
`recover` before exposing ready admission. Recovery requeues claims that never entered the adapter
and retains uncertainty for entered work without terminal evidence. A recovery continuation that
makes no progress is refused instead of looping indefinitely.

The origin's remote adapter remaps its local capability identity to the pinned remote descriptor
and constructs one `PeerInvocationRequest`. The server first checks for exact replay. Fresh requests
must pass catalog and authority checks, then one store transaction binds acceptance, request digest,
relationship/catalog generations, capacity, and dispatch availability. An acceptance response means
that record is durable; a worker later claims it, rechecks authority, records entry, and calls the
exact local adapter through `CapabilityHost::execute_exact_with_context`.

If the acceptance reply is lost, `PeerHttpClient::submit` retries the same request up to three times
for transport/unavailable errors, then looks up the same key. It does not change the catalog or
deadline to manufacture a new submission. Once accepted, the remote adapter polls contiguous
observation pages from its last sequence and reports them through the origin's durable reporter.
HTTP also exposes SSE over the same service pages; keepalive comments carry no execution facts.
Each bounded stream cycle checks authentication and execution authority again.
SSE closure does not include an archived summary; use lookup or observation pages to recover that
disposition when detailed rows have been compacted.

Connection loss after acceptance triggers observation retries within the deadline. Missing records,
deadline expiry, and interrupted observation preserve uncertainty. A terminal observation already
committed by the serving worker outranks a later worker or persistence error. Shared submission
idempotency prevents another acceptance for the same request; it does not make arbitrary external
side effects globally exactly once.

## Artifacts, cancellation, and retained history

Artifact exchange is explicit: `PeerService` verifies that the transfer belongs to an accepted
execution, then `PeerArtifactStore` negotiates metadata and handles chunks. `CorePeerArtifactStore`
uses the ordinary core publication/read ports, preserving sensitivity, retention, and source
provenance while adding peer/execution origin. Uploads resume from exact offsets and become visible
only after content verification and publication. Downloads use verified ranges. The remote adapter
does not automatically copy every referenced input or output to the other host.

Cancellation has its own authenticated route and request identity. Before entry, the service can
prevent invocation and durably complete cancellation. After entry, it forwards the request to the
owning generation and retains the acknowledgement separately from terminal observations. Closing
a socket or calling `PeerRegistry::disconnect` is not a substitute for cancellation.

Serving-worker shutdown closes durable claims and joins workers up to a deadline, reporting retained
handles when it cannot finish. Keep the store and capability host alive for final writes. The daemon's
[shutdown owner](../../apps/daemon/src/host/shutdown.rs) coordinates this with runtime workers.

Retention compacts eligible terminal/uncertain records into durable tombstones. Lookup, exact replay,
and conflict behavior survive; detailed observation rows and their peer artifact links do not.
Consequently, an archived execution can replay its final summary while a new download negotiation
through its old observation links is refused. Core artifact retention remains independently owned.

For implementation changes, `remote` owns catalog registrations and the origin adapter; `client`
and `http` own HTTP framing; `service` owns authorization, execution, recovery, and transfer decisions;
`dispatch` owns fixed workers; `artifact` bridges content storage. The
[peer service suite](tests/peer_service.rs) and remote adapter conformance tests cover these boundaries
with local fixtures. They do not qualify arbitrary external peer deployments.
