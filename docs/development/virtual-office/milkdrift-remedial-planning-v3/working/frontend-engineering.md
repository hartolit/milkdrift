# Frontend engineering proposal

**Historical F2/r3 technical annotations.** The current
[F1/r4 and F2/r4 interface contract](../deliverables/production-interface.md) governs canonical NP3
source, WorkCommitment, U17 knowledge assessment and changed delivery order. P01 is the full source
owner; browser foundation, package verification/locking and the first connected app are P02;
ongoing work is P04, knowledge P07 and combined use P08. Preserve the compatible narrow frontend
owners, custody, exact wire/recovery, bounded streams/storage and testing details below. Old package
versions are candidates to verify, not compatibility evidence. Old graph/finite-planner/learning
contracts and phase numbers must not create parallel owners or override r4.

Author Bram; F2/r3, 2026-10-10. This is the technical consumer of
[production interface F1/r3](production-interface.md), [architecture A1/r3](architecture-and-federation.md)
and [notation NP2/N1/r2](notation-profile.md), following their actual review. It specifies future
implementation, not installed dependencies or passing tests. Source checkpoint and web evidence
are in [FE1](frontend-evidence.md). This worker ran no Cargo, browser application, package install,
dependency qualification or production edit. The frontend practice for later adoption is
[the proposal](../deliverables/frontend-practice-proposal.md).

## Package, build and deployment choice

Place the retained production application at **`apps/workbench`**, with a private npm package and
one `package-lock.json`. Select static SvelteKit 3, Svelte 5 and TypeScript 6. Use **Node 24.21.0
LTS** and **npm 12.2.0**, pinned in package/toolchain documentation and CI. The official pages
retrieved on 2026-10-10 identify those Node/npm versions. npm's clean-install command requires a
matching lock and does not rewrite it. [Node downloads](https://nodejs.org/en/download),
[npm 12.2.0 ci](https://docs.npmjs.com/cli/v12/commands/npm-ci/).

The current Kit 3 migration document sets minimum Svelte 5.57.1, Vite 8.0.12, TypeScript 6 and
vite-plugin-svelte 7, with configuration in the Vite plugin and the new `#lib` alias. P01 must
verify the selected actual package versions/peer constraints and record exact direct versions in
the manifest plus full transitive resolution in the lock. Do not fabricate an installed patch
version from a compatibility minimum or copy the old Kit configuration. A failed compatible
resolution reopens this finite setup choice before authoring proceeds.
[SvelteKit 3 migration](https://svelte.dev/docs/kit/migrating-to-sveltekit-3).

Use the static adapter and client-rendered application routes, a deployed fallback document and
explicit base-path/deep-link tests. Serve immutable assets with hashes and short-lived HTML so an
old shell does not silently load incompatible modules. No production Node server, server action,
SSR session, frontend database service or mandatory local broker is selected. Static delivery
avoids shared server session state, but does not solve CORS. Kit earns its routing, navigation
focus/title and build conventions; ordinary Svelte+Vite remains the smaller alternative if these
facilities prove unused. [Static adapter](https://svelte.dev/docs/kit/adapter-static),
[Kit state management](https://svelte.dev/docs/kit/state-management).

The delivered browser route is A1's opt-in exact-origin daemon CORS with bearer fetch; remote
connections use operator-managed HTTPS termination to the loopback daemon. Same-origin fixed-target
proxy deployment remains a credible alternative and is tested on its own terms. A development
proxy never qualifies the separate-origin release build. No service worker is needed initially;
in particular none caches authenticated replies or replays writes offline. A future desktop
wrapper must consume the same public owner contracts and undergo its own credential/reachability
review; it cannot supply missing browser behavior by assumption.

## Narrow frontend owners

The planned organization is a set of explicit consumers, not a general application framework:

| Area under `apps/workbench/src` | Responsibility and allowed dependencies |
| --- | --- |
| `routes` | Route composition, title/focus, connection-qualified parameters; depends on features, never builds effect envelopes itself. |
| `lib/connections` | Connection metadata, per-tab bearer, negotiated identity, freshness, request epoch and lifetime; owns no workflow facts. |
| `lib/protocol` | Exact supported wire types/decoders, bounded fetch and SSE framing, route/path escaping, request serialization; consumes only public contracts. |
| `lib/recovery` | Versioned exact request files, local records, owner/authority recheck and user-visible recovery state; no autonomous execution retry. |
| `lib/features/work` | Definition/draft/run/proposal projections, meaningful forms and accessible outline; uses protocol/connection/recovery. |
| `lib/features/operations` | Public service/direct call and invocation observations, separate from run identity. |
| `lib/features/resources` | Installation/recipe projections and exact lifecycle/editing/evaluation commands. |
| `lib/features/improve` | Source/declaration/candidate/comparison/promotion receipt projections and inputs. |
| `lib/diagram` | Reviewed node/edge renderers, viewport, selection and layout adapter. Takes semantic projections and emits editing intents; owns no validation/compiler. |
| `lib/ui` and `lib/styles` | Small proven shared controls, tokens, feedback patterns and accessibility; no transport or grants. |

Use route/feature-local Svelte state with explicit connection context. A small session registry
coordinates connection lifetime; it is not a giant mutable global store of every object. Components
receive decoded projections and callbacks rather than performing arbitrary fetches. A single
transport implementation handles each public protocol. No copied bearer handling in feature
folders, generic `common` package or event-bus workflow engine. Unmounting a route cancels that
route's observation subscription; it does not dispose another route's connection or run.

Private undo/redo belongs to one draft; committed model state belongs to the daemon. Compound
authoring collects all required inputs and uses the reviewed existing mutation or owner-side
construction boundary. It must not reproduce hashing, branch activation, reconciliation, grant
evaluation, result reduction or semantic graph rewriting in JavaScript.

For saved definitions, exact base binding is not a global latest-head compare-and-swap. The current
authoring handler compares envelope base with draft base; immutable storage permits valid divergent
descendants. A concurrent save therefore calls for visible ancestry and explicit chosen revision,
not an invented stale-head conflict. Active run sequence/adoption guards and layout generations
are distinct actual optimistic controls. If a future product needs one mutable shared editing head,
that is a new owner/contract decision, not a client cache preference.

## Wire compatibility and exact numbers

Initially maintain narrow TypeScript wire projections and decoders next to consumers, checked
against Rust-produced public golden documents and actual daemon contract tests. Backend source,
readers and fixtures remain authoritative. Do not introduce a schema/TypeScript generator across
the entire repository before its maintenance benefit is demonstrated. If later adopted, the Rust
owner emits deterministic artifacts and CI checks drift; generated types still need runtime input
validation. A TypeScript assertion is not a decoder.

Negotiate the exact supported protocol before ordinary requests. Current control is 2.20; new
R-A/R-C contracts require coordinated protocol/schema review and actual owning constants at
implementation. Decode closed tags, required/optional fields, bounded strings/arrays/depth and
error shapes; refuse unknown executable meanings rather than stripping them on save. A recognized
read-only representation may show raw bounded data with a clear compatibility explanation, but
must not turn an unknown node into an editable generic task. Local record versions are separate
from control/blueprint/layout versions.

Rust `u64` sequences, generations and guards can exceed JavaScript's exact `number` range. The
transport must preserve integer tokens through decode, rendering, persistence and re-serialization;
never use `Response.json()` followed by a cast for these envelopes. Select a pinned, reviewed
lossless JSON parser/stringifier as the initial mechanism, with `lossless-json` as the evaluated
candidate. Its maintainer documents preserved numeric text, duplicate-key rejection and BigInt
support; exact package patch and supported browser behavior remain P01 lock/test work.
[Maintainer documentation](https://github.com/josdejong/lossless-json).

Convert validated integer fields to an exact internal decimal/BigInt representation; convert to
`number` only after a safe-range check for bounded display/layout values. Opaque nested JSON keeps
numeric meaning when round-tripped. Serialize numeric protocol fields as exact numeric tokens,
not quoted strings unless the protocol says strings. No global BigInt monkey patch. Contract tests
must include `2^53-1`, `2^53`, `2^53+1`, `u64::MAX`, duplicates, depth limits and malformed tags,
including exact replay after browser reload. This is client correctness, not a new server fact.

Read response bodies incrementally with explicit byte bounds before parsing; declared Content-Length
is advisory. Decoder overflow cancels observation and reports incomplete data. Map owner diagnostics
to fields where possible and retain a bounded exact diagnostic detail. A transport failure, protocol
error, authorization refusal, stale optimistic guard and application failure are different types.
Logs and telemetry contain operation kind, bounded diagnostic code and locally chosen correlation
ID; no bearer, request body, output, private artifact URL or automatic full-object console logging.

## Connection state and observation ownership

Each connection has a monotonically changing local epoch plus negotiated stable host, actor,
grant ID/revision/digest and revocation generation. Authorized cache keys include that partition
and exact object/revision/generation. A response is accepted only if its request epoch and partition
still match; abort alone is insufficient because late promises may resolve. Duplicate URLs or
two routes to one host never union grants. Endpoint labels are display metadata.

Bearer tokens stay only in per-tab memory, never localStorage/sessionStorage/IndexedDB, history,
URLs, files or service workers. Reload requires token provisioning again; the product does not
invent refresh tokens or an identity provider. Fetch uses `credentials: omit`, `redirect: error`
and an explicit connection origin. A reconnect first negotiates and rechecks identity. Changed
host isolates the old connection; changed authority invalidates authorized read caches/cursors and
quarantines private records. A capability snapshot, including an empty snapshot, replaces the
previous catalog. It does not merge permissions or change accepted pins.

The current version/authority endpoints themselves authenticate, so the bearer is delivered to
the selected trusted origin before durable host comparison. TLS/origin and operator endpoint
trust govern that credential delivery; stable host checks subsequently guard saved commands and
cache/draft reuse. They are not proof that a replaced daemon at the same trusted origin never
received the bearer. A stronger pre-authentication identity guarantee would require a separately
designed protocol/custody boundary; none is silently invented here.

Use a fetch-based SSE consumer with incremental UTF-8/framing limits, bounded queued events,
last fully handled cursor and explicit stop. The existing route owns event names/cursor/resync
meaning. Run cursor durability differs from process-local health/capability stream incarnation;
do not share one generic cursor across them. On resync, stop rendering the incomplete projection,
read a fresh bounded snapshot and resume according to that feed's actual contract. Direct invocation
observations remain paged requests. Network reads may use bounded backoff/jitter; no mutation is
automatically retried with a new identity. An abort button is labeled Stop observing, distinct from
the exact cancel command.

Current public routes offer run listing, but no server-wide direct-invocation history list was
found. Operations therefore labels its retained request links **Known on this device** and offers
exact request/execution lookup or imported recovery files in a fresh browser. It must not call that
view all host history. A new bounded invocation-discovery route requires an actual accepted user
need and source-owner review; it is not inferred from the desired card layout.

## Private draft and exact-request retention

The proposed retained mode requires a **trusted personal browser profile**, explicitly chosen at the first
private draft/effectful request: keep bounded private drafts and exact recovery records in IndexedDB.
No default selection infers trust or writes private content before this choice. Once affirmed,
retained mode is the normal path for that profile. It improves crash, tab-close and reload recovery.
The alternative is **session/export mode** for
shared or ephemeral profiles: private drafts remain in memory unless exported, and each effectful
submission requires an exact recovery file to be exported before sending. Neither choice changes
daemon durability. The first-use choice is a concise practical explanation, not a perpetual modal.

IndexedDB contents are private application data **unencrypted at rest**. Reauthentication gates
normal UI disclosure; it does not encrypt stored bytes or protect against an unlocked profile,
same-origin compromise, browser extension access or device backups. Memory-only credentials limit
their lifetime but do not prevent active script compromise. Do not claim local key storage beside
ciphertext would remove this risk. Native credential vaults or user-held encryption keys would be
different products with independent recovery requirements, not an implied current feature.

Only unfinished local inputs, exact request envelopes and minimum returned recovery references are
persisted. Do not persist fetched graph/history/result bodies as an offline execution database.
Known result references may be reopened under fresh authority; outputs are fetched again. Drafts
may themselves contain prompts and selected private context, so the storage choice covers them
explicitly. Export never includes a bearer. Returned secret-like or untrusted model strings remain
data; they cannot change the export path or dispatch action.

| Event | Required local behavior | Recovery limit |
| --- | --- | --- |
| Before effectful send | Validate complete envelope; assign stable identity once; freeze serialized request and original authority; durably store or finish explicit export; then send | If storage/export fails, no send. This includes start/cancel/proposal/publication/resource/learning and direct serving requests, each retaining its own public identity/guards. |
| Reply lost | Mark response-unknown; offer supported serving lookup, current authorized known-object inspection or operation-specific exact replay | No generic command receipt lookup exists. Missing/denied object reads do not prove absence. Original bytes/identity/guards are never refreshed. |
| Reply accepted/refused | Retain minimum accepted receipt or exact refusal and terminal local status; allow authorized reinspection | Local status does not replace durable server evidence; a malformed/oversized response is not a proven refusal. |
| Logout | Erase bearer, abort feeds/requests, clear authorized in-memory views; keep private records locked under original binding | Logout neither cancels backend work nor revokes grant. Unknown requests are not erased. |
| Grant change | Invalidate reads/cursors, quarantine drafts/requests; allow only nonsensitive count and original-binding explanation until appropriate authentication | No silent rebinding/replay under new grant. Read recovery or administrative resolution may be possible through separately authorized owner paths; UI must not promise old-grant replay will work. |
| Host changes at same URL | Do not send or reveal old private record to replacement; retain locked original-owner record | Recover at verified original owner or through an explicitly supported migration; URL similarity is insufficient. |
| Forget connection | Remove session/endpoint metadata; retain locked records or offer exact export then deliberate local deletion | Pending/unknown must be recoverable by successful export before removal from this application's store. Browser/OS deletion outside app cannot be prevented. |
| Clear resolved records | Show exact selected terminal records and local-only effect; delete deliberately or apply stated retention bound | No backend cleanup, history rewrite or resource deletion. |
| App upgrade/rollback | Versioned record migration preserves bytes/original binding; unsupported records remain exportable and locked | Never reset the DB on migration failure or drop unknown records to make an old client boot. |

Opening an imported record only validates its format and presents its original binding; it never
automatically sends. Workflow saved-start replay requires exact authority equality as in the Rust
saved-run consumer. Current-authority inspection of a known accepted run may be possible after a
grant change, but does not prove whether an unknown command was accepted. Direct serving recovery
has a different real contract: `invoke_client` can find the retained request, require current
Inspect authority and compare an exact resubmission against its stored original authority basis.
It may therefore recover across changed grants when the server permits it. Offer an explicit
operation-specific recovery action with the original record unchanged; do not make workflow
equality a universal frontend ban or silently replace original authority. The server decides
current disclosure. Quarantined private contents stay out of ordinary views until the appropriate
recovery/read authority is established. [Direct owner](../../../../../crates/capability-host/src/serving/direct.rs).

The browser cannot guarantee a download is backed up forever. In export mode, require completion
of its download/write flow and an explicit acknowledgement that the file is retained before send;
where a supported file API provides write completion, use it, while retaining normal download as
the portable option. Document that clearing browser data and losing export files can remove the
client's only unknown-request locator. Already known run/invocation identities remain inspectable
from their owners under current authority. This limitation must be part of the early human task.

Two open tabs may use the same recovery record. IndexedDB transactions allocate/update local
record state atomically; a short local ownership lease prevents accidental double gestures, but
the server's exact idempotency remains decisive after a tab crash or split. Broadcast messages
contain invalidation IDs only, no credentials/private content. A tab must independently authenticate
before reading a partition. Do not depend on a browser lock for workflow or effect correctness.

## Initial bounds and performance qualification

These are proposed client operating bounds, separate from protocol limits. P01/P02 measure and
may revise them with evidence; changing a client cache size never relaxes a daemon contract.

| Surface | Initial bound/strategy | Full/overflow behavior |
| --- | --- | --- |
| Connections | 32 saved metadata entries; 4 actively observed connections; independently authenticated inactive entries | Explicit pause-one-to-activate choice; no silent disconnection of work or deletion of another owner's recovery. |
| Requests | 4 concurrent ordinary reads per active connection, queue 16; mutation gestures serialized per affected object | Queue full reports busy and offers retry; no unbounded background fan-out. Exact pending submission is never evicted. |
| Streams | At most 2 per active connection: selected run plus capability feed; bounded health polling when needed; at most 8 total | Close unused views/feeds; show freshness. A background run continues regardless of observation allocation. |
| Protocol payloads | Owning limits, currently document 1,310,720 bytes, page maximum 1,024, depth 72; upload decoded bytes 524,288 | Reject before send where known and on streamed receive; owner still validates. No silent truncate-and-submit. |
| Lists/history | Request 128 items initially, retain at most 5 pages per view; virtualize long lists with explicit older/newer navigation | Counts/window shown; fetch next page deliberately; cursor-expiry resync distinct from empty history. |
| Read cache | 32 MiB in-memory total, 8 MiB per active owner partition; byte-account serialized projections | LRU may evict reproducible reads, not drafts/pending requests. Re-fetch under current authority. |
| Private drafts | 20 saved drafts, 8 MiB total, each within protocol document bound; retain until deliberately saved/exported/discarded | Block another saved draft and offer explicit disposition; no silent unsaved-work eviction. |
| Exact requests | 128 records/32 MiB total: routine lane 112/28 MiB, reserved control lane 16/4 MiB for cancellation/pause/authorized resolution; each within owning bounds; unknown/pending protected | Prune only resolved records under visible 30-day/count policy. Full routine lane leaves control capacity. If that also fills, offer exact export or an authorized native-client stop path; never silently hide Cancel, send unrecorded effects or evict unknowns. |
| Artifact views | No persistent content cache; bounded owner ranges and rendered preview, explicit download with metadata/size checks | Show incomplete/restricted state. Large output is not parsed as executable HTML or unbounded JSON. |
| Diagram | Keep permitted full semantic document; render viewport/neighborhood and outline windows; initially target 500 visible elements and measure actual largest supported fixture | Windowing never removes nodes from saved document. If renderer cannot present a valid graph, outline remains complete and graph says its limit; fix rather than claim full renderer support. |
| SSE | Use each protocol's frame/body bounds; additional client queue capped at 128 handled-or-pending events | Pause/abort and resync on overflow; never drop events and keep a complete badge. |

Measure first route usable, first graph/outline inspection, interaction latency, memory after
repeated owner switching, and stable heap after stream teardown at small/medium/max-supported
fixtures. Set budgets from P01 measurements on recorded browser/device configurations, then gate
regressions. Arbitrary unmeasured millisecond promises are not acceptance evidence. Long layout
work must be cancellable/discarded when its base changes and must not freeze critical request
recovery controls. Document the workload, viewport, node count and bytes, not only a screenshot.

## Components, diagram and untrusted content

Select **Svelte Flow 1.6.3** as the conditional renderer, with custom N1 nodes/edges and explicit
adapters to presentation state. This release is advertised by its maintainer site; qualification
still requires the actual locked package. Disable default destructive key behavior and automatic
mutation handling until it passes through the draft-intent owner. Built-in keyboard options are
inputs to review, not an accessibility certificate. Cytoscape.js remains an alternative if complex
graph performance or routing disproves the selection; a semantic outline is required in either.
[Svelte Flow](https://svelteflow.dev/), [component API](https://svelteflow.dev/api-reference/svelte-flow).

Use native HTML controls first. Add narrowly selected Bits UI primitives only for proven composite
widgets that need them, with exact package/license/peer review in P01. Do not install a component
suite merely to restyle buttons. Manual layout is retained and deterministic simple initial
placement is presentation-only; add ELK.js only if actual nested fixture routing justifies its
worker/bundle cost. It may compute coordinates, never executable edges or owner boundaries.
Versioned dependency updates rerun affected actual interactions and visual/keyboard checks.

Store F1 visual tokens as CSS custom properties. Copy the approved logo unchanged to
`apps/workbench/static/brand/milkdrift-logo.svg`; P01 byte-compares it to the supplied asset.
No external font or icon network request is needed; use system fonts and a small consistent
reviewed icon set with text labels. Dark tokens are the initial theme; forced colors and system
contrast preferences remain functional, and a later light theme needs its own contrast evidence.

Render outputs, artifact names, logs, model proposals and diagnostics as text or explicit typed
components. No raw HTML, SVG injection or Markdown HTML passthrough. If rich Markdown becomes a
real requirement, adopt a reviewed sanitizer/renderer and an allowlist with unsafe URL schemes
refused; artifact execution remains separate. Downloads use untrusted names only after safe local
filename handling. Frontend CSP restricts script to packaged content and disallows unsafe eval;
operator `connect-src` allows exactly the approved deployment origins. Dynamic arbitrary-host
selection and a strict fixed CSP may conflict: the supported origin set is operator configurable,
and adding an endpoint outside it requires setup, not a hidden network bypass. CSP is documented
per topology and tested with the static release, including graph style requirements.

Initial deployment is an operator/user-hosted static build with an approved endpoint set. The app
receives that nonsecret deployment policy to explain Add connection refusals; it cannot relax the
HTTP response CSP at runtime. A public arbitrary-host workbench is a separate unqualified topology,
not the initial promise. Cyra's review made this qualification explicit in F1 as well.

## Checks, fixtures and CI contract

P01 creates these package scripts and records their actual tool versions. The following are
**future exact invocation commands**, not claims that scripts or package presently exist:

```sh
npm --prefix apps/workbench ci
npm --prefix apps/workbench run check
npm --prefix apps/workbench run lint
npm --prefix apps/workbench run format:check
npm --prefix apps/workbench run test:unit -- --run
npm --prefix apps/workbench run build
npm --prefix apps/workbench run test:browser -- --project=chromium --project=firefox --project=webkit
```

`check` owns Kit-generated type setup plus `svelte-check`/strict TypeScript; `lint` owns ESLint with
Svelte/TS rules; `format:check` owns Prettier with the Svelte plugin; `test:unit` owns Vitest; `build`
produces the static release; `test:browser` owns Playwright projects against that release. P01
chooses compatible exact dev-tool patches after checking their primary docs. Browser binaries are
installed explicitly by CI using the locked Playwright tool, not downloaded by an unreviewed app
runtime. [Vitest CLI](https://vitest.dev/guide/cli.html),
[Playwright CLI](https://playwright.dev/docs/test-cli).

Backend fixtures, daemon lifecycle, controlled external providers and state seeding remain Rust
tools under the repository's maintained evidence owner. Playwright TypeScript drives browser
interaction; it does not implement a fake daemon, JS workflow scheduler or substitute shell/Node
backend scripts for Rust. The Rust fixture process emits bounded per-test connection descriptors
and ephemeral tokens through a local protected file/pipe; they are never committed or included
in test reports. The harness owns port allocation, readiness, cleanup and exact failure evidence.
P01 defines the actual Rust harness command with the evidence owner; npm browser tests consume
its descriptor and must fail if no real fixture is present. No fallback to response mocks.

| Check lane | What it must prove | What it cannot certify |
| --- | --- | --- |
| Unit/component | Exact decoders/number roundtrip, stale epoch rejection, bounded queues, draft guards, storage failures, dialog/outline behavior, stream framing | Real daemon/browser CORS, runtime semantics or human comprehension. |
| Rust↔TS contract | Golden supported wire forms, refusals, exact numeric guards, semantic draft reload, protocol mismatch and request byte preservation | Real external effect or cross-browser accessibility by itself. |
| Real-daemon browser | Separate-origin allowed/denied CORS, bearer fetch SSE/resync, two owners, actual commands/results/proposals, direct/resource/publication/learning paths as delivered | General deployment portability beyond recorded topology/browser versions. |
| Recovery | Crash after send/before receipt, failed IndexedDB write, export mode, duplicate tab, same URL/new host, grant change, old/new app local record version | Impossible recovery after all user-held exact locators are deliberately destroyed. |
| Accessibility | Automated rules, keyboard and non-drag pointer journeys, focus/zoom/forced-colors and actual screen-reader review of graph/outline/errors | Full WCAG conformance from axe or library marketing alone. |
| Visual | Stable snapshots of real fixture states, long labels, sparse/dense graph, all refusal/uncertainty/approval states and narrow layouts | Semantic progress from a screenshot. |
| Human P03/P08 | Actual task interpretation and compound outcome; retain unexpected failures and reopen assumptions | Statistical usability claims from one session or model/provider superiority. |

CI runs frontend checks whenever frontend, consumed protocols or fixture contracts change. Changes
to backend contract run focused Rust suites with frontend consumers; executable full-system gate
remains the named final P09 under workflow policy, with each earlier prompt finishing its focused
checks. P09 reruns after human-driven corrections. Test diagnostics store versions, owner-safe
identities, timing and sanitized failure traces, with private payload capture opt-in to an explicit
protected evidence lane. Do not weaken tests or introduce invented responses to make a release
snapshot pass.

Package README explains purpose, actual build/dev commands, supported browser/topology matrix,
operator CORS/TLS/CSP setup, token provisioning, personal-profile versus export mode, source-owner
boundaries and troubleshooting. Canonical current facts move into their owners only during P00
adoption/implementation; these planning files must not become permanent duplicate API manuals.

## Review status and reversal

F2 selects mechanisms conditionally; npm/Node docs and library APIs are primary research, not a
resolved lock or executed browser qualification. R-A01 success, exact numbers, local storage loss,
grant change, advanced graph/outline equivalence and actual user comprehension can reverse choices.
Prefer Vite if Kit adds unused state/deployment complexity; prefer another renderer if Svelte Flow
cannot meet complete accessible authoring; prefer the fixed-target proxy if intended users cannot
operate direct origins coherently. These reversals preserve the public owner and exact recovery
contract, not a promise that every frontend dependency remains fixed forever.

F2/r3 incorporates BR3's end-to-end authoring/storage source correction and actual P02 recheck:
same-parent revisions can both be stored, without a mutable shared head. The frontend displays
that ancestry and preserves exact selection. P01 exact-number/custody/topology checks and P05's
operation-specific direct recovery remain unchanged from the reviewed r2 contract.
