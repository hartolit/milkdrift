# P02 — Browser foundation and the first real Svelte workflow

Owner: daemon transport and production client team, one coordinator for shared contracts/Cargo.
Requires P01 and approved A1/F1r4/F2r4. Read [shared context](context.md),
[production interface](../production-interface.md) and [frontend practice](../frontend-practice-proposal.md).
This folds the old P01 browser foundation into the first complete user slice. It is maintained product
code at `apps/workbench`, not a prototype or permission to duplicate the source compiler.

Implement the explicit-origin browser route in existing daemon HTTP/config ownership: default disabled,
bounded exact allowlist, strict origin/method/header rules, nonauthorizing preflight, correct Vary,
required artifact/range headers and ordinary authenticated requests. Do not broaden loopback binding,
reflect arbitrary origins or invent cookie/URL-token fallback. The selected topology is operator-hosted
static Svelte with approved daemon endpoints in release CSP connect-src and daemon origin allowlists.
Qualify the operator HTTPS path; a development localhost connection is insufficient deployment evidence.

Build with pinned reviewed dependencies, type/lint/test/build scripts and deployment docs. Begin at
zero connections. Bind independent connection state to authenticated host, actor, grant and protocol,
not URL/label. An unavailable owner leaves the others usable. Use one bounded transport/decoder,
one-send effectful requests, rejected redirects and authenticated fetch SSE. Complete catalog snapshots
replace state; resync refreshes authorized views. Durable run cursors retain their separate contract.
Bound malformed frames, chunk/CRLF/multiline handling, retries, caches and subscriptions.

Qualify lossless JSON before effects: preserve 2^53−1, 2^53, 2^53+1 and u64::MAX in guards/generations/
cursors/export/reload/replay; reject malformed/out-of-range values. Do not independently canonicalize
source identity or falsely claim a huge-sequence fixture came from a real store.

Require F1's explicit custody choice before private drafts/effectful requests: trusted personal-profile
bounded IndexedDB without tokens, or session drafts plus successful exact export before submission.
Retain complete request and host/actor/grant binding before send. Storage/export failure prevents send.
Keep tokens in memory. IndexDB is not encrypted shared-device protection. On logout/authority change,
abort readers, reject late replies, clear private observations and quarantine retained requests under
their original binding. Never evict pending/unknown records for logout/quota. Delete only after resolution
or explicit successful recovery export. Preserve the reserved control capacity and native-control/export
escape specified in F1; a full ordinary quota must not force unrecorded cancellation. Connection deletion
and fetch abort do not cancel accepted work.

Implement owner/capability selection, simple complete structured method creation/editing, supplied inputs,
daemon-validated save, start, observation and verified results. Use P01 source operations. The simple
initial editor subset must preserve every unknown/advanced element and offer read-only inspection rather
than saving a reduced definition. A graph and outline have equal supported actions; advanced complete
interaction is required by P08. Legacy graphs are inspectable/runnable; editable conversion requests owner
validation and shows precise refusal. Neither a hidden graph compiler nor layout-derived semantics is allowed.

Local drafts, immutable saved revisions, runtime state and layout are distinct. Two valid same-parent
successors can both save; show ancestry and deliberate selection, not a fabricated latest-head conflict.
Actual envelope/base mismatch or guarded run/layout conflict preserves the draft. Display selected versus
accepted generation, invocation outcome versus result acceptance versus workflow outcome, and exact
context/omission/denial facts. Different briefs produce separately owned inputs/results.

Reconnect uses each operation's original replay and current inspection rules. An independently permitted
new grant may inspect a known run; it cannot rebind the old exact command. Missing/denied lookup does not
prove no effect. Quarantine unknown saved work and present its authorized owner investigation route, never
a new request ID as an uncertainty workaround. Bound and verify artifact bytes/digests; untrusted output
renders inertly. Displaying a response proves no model quality.

## Acceptance and first human handoff

Use actual daemon A/B plus execution-only C, different grants and same labels; exercise allowed/denied
origins, release CSP, auth/redirect/TLS errors, token rotation, lossless numbers, storage denial/quota/export
failure, pending reload, logout/late response, empty snapshots, cross-feed/restart resync and bounded stream
cleanup. Require positive authenticated browser JSON, stream and download, correcting E04's current refusal.

Run one saved new method on two briefs; inspect controlled requests/artifacts; crash after acceptance before
reply; replay without duplicate external entry; vary actor/grant/generation, immutable sibling save versus
actual bad guard, denied metadata/content, truncation/digest mismatch and legacy conversion refusal. Use
actual independent JSON-client and maintained workflow evidence where applicable, plus real-daemon browser
and accessible keyboard paths. No mocks substitute for owner decisions.

Run affected daemon/config/protocol/client and frontend tests/static/build/docs checks. Record actual
browser/environment limits. Commit complete transport/config consumers, then the first complete product
journey and fixes. Hand a runnable retained application to P03. P04–P08 remain required product scope;
do not pretend this early interaction completes ongoing work or knowledge learning.
