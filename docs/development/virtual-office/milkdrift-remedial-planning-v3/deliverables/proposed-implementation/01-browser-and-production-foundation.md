# P01 — Connect a retained Svelte client to real independent owners

Owner: daemon transport and frontend foundation pair, one coordinator for Cargo/shared contracts.
Requires P00, approved A1/F1, G1/V01/V06/V11 and R-A01/R-A02. Read [shared context](context.md),
architecture/browser and frontend-engineering specifications, `apps/daemon/src/http.rs`, config
compiler, auth/stream owners, `control-protocol` and the native client as a behavioral consumer.
E04 established current preflight refusal, not successful new transport.

Implement the selected explicit-origin browser route in the daemon's existing HTTP/config owner:
default disabled, bounded exact allowlist, strict scheme/origin parsing, approved methods/headers,
preflight without credentials granting no application rights, and correct origin-vary behavior.
Actual application requests still authenticate/authorize normally. Expose required artifact/range
headers and retain body/stream bounds. Do not accept wildcard credential access, reflect arbitrary
origins, add cookie/URL-token fallback, or silently broaden the loopback listener. The operator's
HTTPS proxy/tunnel retains connectivity/TLS ownership; document and exercise it in the selected lane.

The selected initial topology is an operator-hosted static app with an explicitly approved daemon
endpoint set in deployed CSP connect-src, plus each daemon's origin allowlist. Add connection outside
that set explains the required hosting-policy change. Arbitrary endpoints from a generic public site
are unqualified. Test the release CSP and operator HTTPS path, not only development localhost.

Create the actual maintained Svelte application at the frontend spec's path with pinned dependencies,
build/type/lint/test scripts and deployment docs. No fake login, catalogue or daemon responses count
as completion. Start at zero connections. Implement owner/actor/grant/protocol negotiation and
independent session state; a URL/label is not identity. One failed owner leaves others usable.
Use a single bounded transport module, strict response decoding, rejected redirects, one-send
mutations and fetch-based authenticated SSE. Decode byte/chunk/CRLF/multiline boundaries and bound
frames/reconnects. Complete capability snapshots replace state; resync obtains a fresh authorized
view; durable run cursors follow their existing contract.

Qualify F2's lossless JSON boundary before any effects. Protocol guards/generations/cursors may be
Rust u64 values beyond JavaScript's safe integers; ordinary JSON.parse/stringify can change exact
identity. Use the selected bounded lossless parser/serializer and exact decimal representation,
without independently canonicalizing semantic documents. Cover 2^53-1, 2^53, 2^53+1 and u64::MAX,
malformed/out-of-range values and exact request export/reload/replay. Separate a contract fixture
from real-daemon evidence where an actual store cannot practically reach a huge sequence.

Keep credentials in memory and authorized observation caches per connection. Before the first
private draft/effectful request, require a custody choice: bounded IndexedDB records only after
the user affirms a trusted personal browser, or session drafts with exact export before submission.
Do not infer device trust. Neither choice stores bearer tokens. Persist the complete exact request
and host/actor/grant binding before any send; failed persistence/export prevents submission.
IndexedDB is unencrypted at rest, and application locking is not shared-device protection.
On logout/identity or authority change, abort owned readers, reject late replies, clear old private
views and lock retained drafts/requests to their original binding. Changed grants quarantine records;
never silently rebind or replay them. Pending/unknown records cannot be evicted for quota or logout.
Forgetting them requires resolution or successful explicit recovery export before local deletion.
Reserve bounded control-request capacity so a full ordinary draft/request budget does not obscure
a permitted cancel/stop. When reserved capacity or browser storage is unavailable, require successful
exact export or give the existing authorized native control path; do not erase unknowns or send
unrecorded commands. Test quota exhaustion with ongoing work and a legitimate stop request.
Deleting a connection never cancels backend
work. Authentication errors, incompatible protocol, unreachable/TLS/origin failure and unknown host
identity have distinct recovery instructions. Never infer broad permission from a preset name.

Tests: actual daemon A/B plus execution-only C, same labels and different grants; zero/bad/denied
connections; allowed/disallowed origin, malformed/preflight, credentials not forwarded on redirect;
token rotation; custody choice, storage denial/quota/export failure, reload with pending request,
grant-change quarantine and logout without losing uncertainty; complete empty catalogue;
response after logout; cross-feed/restart resync; run
cursor continuity; bounded malformed/oversized stream and subscription cleanup. Include positive
authenticated browser JSON/stream/download, not just native reqwest. Browser network policy/TLS
failures remain visible and unqualified platforms stay out of support claims.

Run changed daemon/config/control-client protocol tests, relevant frontend transport contracts,
type/lint/build and real-daemon browser tests, plus docs. Commit complete HTTP/config consumers,
then retained client connection/stream path, then review corrections. A config/schema change gets
an exact reader/fixture decision and coordinated current-consumer adoption. Stop when two actual
owners work without a global permission union; no workflow authoring success is claimed yet.
