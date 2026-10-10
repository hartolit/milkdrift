# Frontend evidence and production interaction candidates

Author: Bram. Evidence revision FE1, 2026-10-10. This document records source findings and
candidate interactions before the phase-02 gate. It selects no architecture, library, packaging
or credential policy. It is input to a real production-interface design, not a prototype plan.

The original implementation checkpoint inspected was
`908e7893f5dadb84d12712573c8daaa946829e39`; the working HEAD advanced through planning commits to
`d7caf6da56a0f849abff7a7468ac1001f13af5a0` during this investigation. No production changes were
made by this worker. Commands, readers and consumer tests were inspected, not executed. No Cargo,
JavaScript package installation, live daemon mutation or deployment occurred. Primary web
documentation was retrieved on 2026-10-10; documentation versions are identified below, without
claiming an installed or qualified dependency lock.

## Public boundary already present

The authoritative transport inventory is
[the daemon router](../../../../../apps/daemon/src/http.rs), the closed
[command enum](../../../../../crates/control-protocol/src/command.rs), the
[read types](../../../../../crates/control-protocol/src/read.rs) and the owning handlers.
[Control protocol constants](../../../../../crates/control-protocol/src/lib.rs) currently specify
**2.20**, with exact major/minor negotiation; the version reply identifies the service, not the
particular installation. A Svelte client may consume the HTTP contract without CLI subprocesses
or Rust object construction. The
[independent JSON-client test](../../../../../apps/daemon/tests/control_plane/independent_client.rs)
is existing test-source evidence for an actual-daemon route, not a browser qualification.

| Person's operation | Existing public route or command | Established limit or frontend obligation |
| --- | --- | --- |
| Add and identify a connection | Authenticated `POST /v1/version`; `GET /v1/authority`; health/readiness reads | `AuthorityRead` supplies stable host, actor and grant identity/digest. The client must separate display URL from owner identity and refuse automatic recovery against a replacement owner. A new identity endpoint is not established as necessary. |
| Find definitions and inspect their graph | Paged `/v1/revisions`, exact revision read with optional canonical document, exact revision diff | Full semantic graph is an explicitly requested document, not every list response. Cache by owner and immutable revision. Do not infer absent details from compact lists. |
| Compose simple model work | `author_blueprint`, including ordinary `BlueprintEdit` gestures and save | The convenience editor deliberately supports fresh model steps, named inputs and selected output/completeness rules. Richer definitions can refuse the convenience edit. |
| Compose structured or advanced work | `construct_blueprint` over existing `BlueprintDraft.mutations`; validate/import blueprint routes | Generic public mutation input already exists. A broader GUI requires complete supported mutation forms and lossless round-trip handling; the absence of a convenience gesture is not proof of a missing runtime primitive. Daemon validation and identity remain authoritative. |
| Keep an independent editable definition | `copy_blueprint` | Exact source inspection and destination import authority apply. Identity-bound governing agreements refuse copying; presenting every definition as freely copyable would be wrong. |
| Supply and execute inputs | `POST /v1/artifact-inputs`, then `start_run` | Upload is bounded (524,288 decoded bytes in current protocol). Ordinary inputs are committed artifact identities. A generic large-file uploader or arbitrary server path is not currently supported. |
| Recover a lost workflow-start response | Resubmit the exact command envelope under the original host/actor/grant | [Saved-run client source](../../../../../crates/control-client/src/saved_run.rs) captures authority before submission and checks it before replay. Browser persistence needs a bounded, private recovery representation; it must not create a second execution ledger. |
| See progress and accepted results | Run/result/node/attempt/timeline routes | `RunResultRead` distinguishes truncated view, restricted outputs and current action hints. Terminal output is separate from intermediate attempt output. Missing, denied and omitted information cannot be rendered as zero or complete. |
| Pause, resume, cancel or signal | `pause_run`, `resume_run`, `cancel_run`, `signal_run` | Command acceptance is not proof that external work stopped. Signal identity/correlation is explicit. Closing the panel or aborting an HTTP read is a different operation. |
| Change future work | `submit_proposal`, exact `decide_proposal`, exact `apply_proposal`, proposal impact reads | Proposed, approved and applied are separate states. `prepare_model_repair` is a narrow convenience, not a generic graph rewrite service. Display the exact base and affected future work before committing. |
| Resolve retained uncertainty | `resolve_work`, with query/retry/compensate/retain/evidence-based choices | Runtime policy decides which choice is permitted. An enabled button or transport retry flag does not prove safe external retry. |
| Call a service or a direct operation | Execution catalog, invocation prepare/submit/request lookup/inspection/observations/cancel/output routes | Direct and published calls use serving ownership; their observations are bounded paged reads, not the run SSE feed. Preparation accepts no work and reserves no generation. Save the exact prepared request before submit. |
| Publish or retire a workflow service | `prepare_method`, `publish_method`, `inspect_method`, `list_methods`, `retire_method` | Administrative publication and public catalog/invocation are distinct. Publication requires workflow-enabled service composition/accounting; execution-only hosts remain valid for their direct operations. |
| Inspect and manage configured peers | Peer list/exact read and connect/reload/disconnect/drain/revoke routes | These operate on configured relationships. They do not create a VPN, discover arbitrary hosts or transfer workflow ownership. |
| Inspect/manipulate managed installations | `POST /v1/resources` with `ManagedRequest` schema 3 | [Managed actions](../../../../../crates/capability/src/managed.rs) include prepare/apply/inspect/start/stop/update/preserve/remove/recover and protected evidence/publication/editing operations. No general installation-list or release-from-management action was found in this closed request owner. Names/recipes must come from a justified public source or operator input. |
| Inspect scope | `/v1/authority`, filtered catalogs, run action hints and exact action refusals | Authority projection lists allowed operation names, not every resource-specific verdict. A complete generic “can I do anything to this object?” endpoint was not found. UI hints must remain advisory. |
| Fetch artifacts | Metadata and bounded ranged content; invocation-owned output route for service results | Verify assembled size/digest where promised; do not grant general artifact access from a service-result reference. Cross-origin deployments must expose required response headers. |
| Save diagram placement | Exact layout read and `put_layout` | Positions/annotations/collapse state are revision-associated presentation data. Daemon receipt processing overwrites author and computes digest before layout validation. A JS semantic hash implementation is unnecessary for this route. Optimistic generation conflicts still need visible handling. |
| Observe changes | Run, health and capability SSE routes | Run cursor durability differs from process-local health/capability cursors. Resynchronization is explicit; a complete empty capability snapshot replaces the old catalog. No global cross-daemon event feed exists. |

The layout observation is deliberately traced end to end: the
[layout type](../../../../../crates/control-protocol/src/layout.rs) documents sealing, but
[receipt processing](../../../../../apps/daemon/src/host/receipts.rs) derives author/digest before
the [layout owner](../../../../../apps/daemon/src/host/layouts.rs) validates and persists it. Reading
only the type would have produced a false API-gap claim. This correction was reported during the
investigation and is a useful check on all other purported gaps.

## Browser integration facts that affect every candidate

The router explicitly omits CORS; its ordinary authentication reads `Authorization: Bearer` in
[the response/authentication owner](../../../../../apps/daemon/src/http/response.rs).
[Configuration compilation](../../../../../apps/daemon/src/config/compile.rs) restricts the daemon's
plaintext listener to loopback. A Rust HTTP client succeeding does not establish that a separately
hosted browser app can connect.

Native `EventSource` exposes URL and `withCredentials`, without a custom-header option. Therefore
it cannot directly supply the current bearer header. A fetch-based SSE consumer or a separately
designed authenticated gateway is a candidate; neither needs a new execution protocol. Exact
event framing, decoded-size bounds, last handled cursor, resync and stop behavior still have to
match the public contract. [WHATWG HTML event streams](https://html.spec.whatwg.org/multipage/server-sent-events.html).

Cross-origin authenticated fetch requires the receiving deployment to allow the frontend origin,
methods and relevant request headers. `Authorization` needs explicit CORS handling; opaque
`no-cors` responses cannot serve as an API workaround. Cross-origin redirects can remove the
authorization header, so a connection plan must handle redirects deliberately rather than
treating a changed URL as the same authenticated destination.
[WHATWG Fetch](https://fetch.spec.whatwg.org/).

HTTPS pages, plaintext destinations and potentially trustworthy local addresses have different
mixed-content consequences. A successful tunnel establishes reachability, not browser origin
permission. The supported deployment matrix must name frontend origin, daemon-facing HTTPS
endpoint, proxy, certificate expectations and browser network permissions.
[W3C Mixed Content](https://www.w3.org/TR/mixed-content/).

Chrome's release documentation establishes a local-network permission requirement beginning with
Chrome 142; its current guidance also distinguishes local-network and loopback permissions. This
is browser-specific evidence, not a universal rule for every browser. A client must explain
blocked local access separately from invalid daemon credentials, and qualification must record
actual browser versions rather than rely on an old private-network preflight recipe.
[Chrome 142 release notes](https://developer.chrome.com/release-notes/142?hl=en),
[current Chrome guidance](https://github.com/GoogleChrome/modern-web-guidance/blob/main/skills/modern-web-guidance/guides/security/local-network-access.md).

Credential persistence is not yet selected. A memory-only credential has a different reconnect
and tab-reopen experience from a managed session or native credential store. Browser local storage
does not become secret storage by convention. A gateway would add credential custody and an
explicitly scoped transport owner; it must not become another scheduler or workflow database.
Those are design consequences, not evidence that a gateway is required.

## Setup and library candidates, with documentation versions

No frontend manifest or lockfile was found in this checkout. Exact package versions and licenses
must be recorded and tested when dependencies are chosen. These are documented candidates:

| Candidate | Primary documentation inspected and version | Consequence for this product |
| --- | --- | --- |
| Svelte 5 + SvelteKit 3 + static adapter | [October 2026 announcement](https://svelte.dev/blog/whats-new-in-svelte-october-2026), [v3 migration](https://svelte.dev/docs/kit/migrating-to-sveltekit-3), [static adapter](https://svelte.dev/docs/kit/adapter-static), [SPA mode](https://svelte.dev/docs/kit/single-page-apps) | File routing and static deployment are available without a Node runtime in production. A client-rendered workbench needs a deliberate fallback/deep-link deployment and startup/loading experience. Static build does not itself solve cross-origin authentication. |
| Svelte 5 + Vite client application | [Svelte documentation](https://svelte.dev/docs), [Vite guide](https://vite.dev/guide/) | Smaller framework boundary, but routing, navigation focus and deep-link deployment need an explicit owner. A dev-server proxy is development infrastructure, not evidence of a shipped connection topology. |
| Svelte Flow | [Current project page](https://svelteflow.dev/) advertises **1.6.3**; [component API](https://svelteflow.dev/api-reference/svelte-flow); [1.1.0 accessibility changes](https://svelteflow.dev/whats-new/2025-06-11) | Custom Svelte nodes, edges, viewport and keyboard controls suit an editor candidate. It is not a workflow validator or a BPMN engine. Disable or intercept default deletion where it would misrepresent a semantic edit; keyboard/layout actions must remain distinct from submitted mutations. |
| Cytoscape.js | [Maintainer API documentation](https://js.cytoscape.org/), rolling docs; exact package release not established here | Credible graph visualization/layout alternative. Custom node controls and a parallel accessible authoring path need evaluation. Its graph data model is not automatically Milkdrift's executable model. No screen-reader qualification is inferred. |
| ELK.js as optional layout assistance | [Maintainer repository](https://github.com/kieler/elkjs), rolling README; package release not pinned | Offers layout calculations and worker integration. Layout result changes coordinates only. A worker must be canceled/discarded when its base revision or editing session changes; an old layout result cannot overwrite a newer draft silently. |
| Native HTML controls plus Bits UI where needed | [Introduction](https://bits-ui.com/docs/introduction), [migration guide for 1.x+ on Svelte 5](https://bits-ui.com/docs/migration-guide), [Dialog](https://bits-ui.com/docs/components/dialog); exact current package patch not established | Candidate focus/dialog/menu primitives with custom styling. Library accessibility claims do not qualify the composed application. Native buttons/forms remain preferable where sufficient. |

The current SvelteKit 3 migration document specifies minimum Node 22.17, TypeScript 6,
Svelte 5.57.1, Vite 8.0.12 and vite-plugin-svelte 7. It moves configuration to the Vite plugin.
These are documented compatibility requirements, not this repository's selected toolchain.
An implementation prompt that blindly copies older `svelte.config.js` examples would be stale.
[SvelteKit v3 migration](https://svelte.dev/docs/kit/migrating-to-sveltekit-3).

SvelteKit handles navigation announcements and focus behavior when its documented conventions are
used. Application titles, dialogs, retained selections and error focus remain design work.
[SvelteKit accessibility](https://svelte.dev/docs/kit/accessibility). Bits UI documents dialog focus
containment and return-to-trigger behavior; custom asynchronous confirmation flows must preserve
those properties when the underlying selection or authority changes.
[Bits UI Dialog](https://bits-ui.com/docs/components/dialog).

The attempted Svelte Flow accessibility-guide URLs returned tool errors. Its component API and
dated maintainer release note were accessible and are the evidence used here. Search snippets or
third-party claims were not used to certify compliance.

## Annotated production interaction alternatives

These are comparable full-product directions. Neither reduces the scope to a disposable mockup.
They preserve direct operations, managed resources and restricted service use alongside graph work.

**Alternative F-A: object-led workbench.** A persistent connection rail identifies host, actor,
role and freshness. Within one selected owner, the main navigation separates workflows/runs,
offered operations, and managed installations. A workflow opens a synchronized diagram and
structured outline, with evidence in an inspector. Exact reuse, copy and service-call actions are
discoverable where the relevant object is shown. A global finder is supplementary. Its strength
is predictable authority context for frequent operators; its risk is making newcomers classify
their task before they understand what exists.

**Alternative F-B: work-led discovery with explicit commitment.** A shared finder can return
readable workflows and advertised operations, each labeled with owner and relevant version.
Selecting a result opens its available actions: use an exact definition, create an independent
copy, or call the public service. The committing action states authority/control consequences.
Definition editing and execution inspection remain distinct destinations. Its strength is finding
useful work without knowing backend nouns; its risk is suggesting that visually similar results
share expand/edit permissions. Unknown private items must not be disclosed to fill a uniform list.

Both alternatives require these concrete compositions:

| Situation | Required observable interaction | Backend boundary or missing proof |
| --- | --- | --- |
| Two owners both call a workflow `release` | Owner/actor/revision remain visible in result, open tab and confirmation. A changed endpoint identity isolates the old draft and recovery request. | Existing stable host and authority reads; frontend partitioning and replacement-owner test needed. |
| Unfinished draft while another author saves a revision | Keep the candidate, show its exact immutable base and the other saved revision, offer deliberate comparison/reconstruction. Never silently merge meaning. | Authoring guard compares envelope base with draft base, not a mutable workflow head. Valid sibling revisions may both save; no automatic stale-head conflict is established. |
| One completed branch, one pending edit, one uncertain attempt | Diagram overlays retain occurrence revision and evidence state. A proposed future definition is visibly a candidate; apply follows actual proposal decision. | Existing run/proposal facts; exact overlay/notation mapping must be designed after the product-model gate. |
| A customer calls a private service | Show public contract, accepted invocation, permitted observations and declared results. Internal graph expansion is absent unless authorized by a separate read path. | Service invocation API exists. No generic promise of internal progress or graph access. |
| The connection disappears after Start | Show “response unknown” with the retained request identity; inspect/replay the exact request after owner/authority verification. No automatic new run. | Receipt semantics and saved-run model exist; browser crash/tab-close persistence lane is unqualified. |
| An installation is busy while a child holds editing rights | Explain inspected blocker and relevant pending operation; present only owner-supported handoff/return/recover actions. | Managed request/inspection exists; no invented unmanage button or unbounded busy queue. |
| A person cannot drag or see the graph | Use outline/table forms to add, select, connect and inspect the same semantic operation; add click-based move/connect actions without a drag gesture. | This is a frontend requirement to validate, not functionality provided automatically by a graph library. |

The last row follows separate accessibility obligations: WCAG 2.2 includes keyboard operation,
visible/unobscured focus, contrast and non-dragging pointer alternatives. Keyboard shortcuts alone
do not satisfy the dragging-movement criterion. A standards-shaped diagram with no operable
alternative is insufficient. [WCAG 2.2](https://www.w3.org/TR/WCAG22/),
[dragging guidance](https://www.w3.org/WAI/WCAG22/Understanding/dragging-movements.html).
For complex widgets, roles do not implement keyboard behavior; focus/navigation need deliberate
interaction design. [WAI-ARIA keyboard practice](https://www.w3.org/WAI/ARIA/apg/practices/keyboard-interface/).

## Actual gaps versus unqualified work

**Source-established absent boundaries:** cross-origin browser support in the current router;
browser credential/session integration; a browser application itself; generalized management
discovery/release actions in the inspected managed request owner; a global event feed. Not all
absences deserve implementation. Each must be tied to an accepted user path after the gate.

**Existing APIs needing client work, not automatically new backend endpoints:** stable installation
identity, exact command replay, daemon-derived layout digest, advanced blueprint mutation input,
private-service invocation, bounded artifact reads, run results, prospective proposals and scoped
catalogs. Their usability and complete browser handling remain unqualified.

**Consequential unknowns:** whether the selected UI needs additional bounded structured read
projections to avoid interpreting canonical graph documents; how much advanced authoring requires
daemon-owned conveniences; whether installation discovery is necessary for the initial managed
operations page; which connection topology and credential lifetime the user wants; which graph
library handles the actual nested/large examples acceptably. None is settled by library popularity.

Production acceptance should use the real daemon with controlled external capabilities, including
changed grant/host, wrong version, denied internal access, malformed/truncated stream, resync,
lost write response, concurrent edit, bounded overflow, restricted artifact and failed verification.
Browser tests should cover selected Chromium, Firefox and WebKit versions plus keyboard and
screen-reader/manual checks. Playwright documents those browser projects; this is a test-tool
candidate, not executed interoperability evidence.
[Playwright browsers](https://playwright.dev/docs/browsers).

## Review status

This evidence is ready to inform the product-model/architecture gates. It does not choose F-A or
F-B, SvelteKit versus Vite, direct CORS versus a gateway, or a graph library. After the gate, the
production design must make those choices, state remaining user preferences, trace every committing
action to its owner, and assign actual browser acceptance. Documentation structure, links and source
claims were reviewed locally; the coordinator owns integrated documentation contracts.
