# Production interface proposal

**Historical F1/r3 build annotations.** The current
[F1/r4 and F2/r4 interface contract](../deliverables/production-interface.md) supersedes graph-source
and R-C01 reconvergence authoring, finite R-FE02 planning, method-only learning, and old phase
assignments below. Current source is NP3; ongoing work is WorkCommitment; U17 permits evaluated
nonexecutable knowledge. P01 delivers the full source owner, P02 includes browser foundation and
the early connected application, P04 ongoing work, P07 knowledge and P08 combined use. Retain
compatible layout, resource/public-invocation, custody, recovery and accessibility detail below;
do not implement the superseded remedies as additional owners.

Author Bram; F1/r3, 2026-10-10. This is a build specification and conditional recommendation under
[G1/P1](vision-baseline.md), following the independent [BP1 position](product-position-bram.md),
actual cross-examination and [product alternatives](product-alternatives.md). It is not an
implemented application or a usability result. Production source inspected is
`908e7893f5dadb84d12712573c8daaa946829e39`; planning HEAD at drafting was
`9878f7a73cb7e1fe410a7d7bc16e126c9f979f7a`. [Frontend evidence](frontend-evidence.md) owns the
source/API inventory. [Frontend engineering](frontend-engineering.md) owns transport, storage,
toolchain and bounds. Architecture and notation retain their own decisions; this file owns the
human interactions and their acceptance tasks.

## Product composition and credible alternative

Select an intent-led workbench with an explicit owner context. Its primary destinations are
**Work**, **Operations**, **Resources**, and **Improve**. Work combines saved definitions, private
drafts and executions; Operations combines direct tools and public services with explicit type
labels. Resources and Improve expose their distinct lifetimes. Pending decisions and unresolved
requests form an attention list available from every destination. Capabilities, peer relationships
and authority details live in the connection inspector, not as extra global application roles.
Author, caller, reviewer and operator describe a person's current task; they confer no grants.

A shared finder searches already authorized, bounded owner lists. A result says what it is, where
it lives, its exact revision/generation, and when its information was checked. Selecting it opens
the P1/r2 committing choices: use an exact locally available definition, make an eligible independent
copy, invoke an advertised operation, or inspect what the current grant permits. Results from
several owners never appear as one merged authority. Destination-specific routes remain available
for experienced operators. Restricted results are omitted or refused according to the owning API;
the client does not show a guessed private graph or an inventory of undisclosed objects.

The credible alternative remains FE1's object-led workbench: separate definitions/runs/catalogs
under each owner, with common discovery secondary. It costs an initial classification but reduces
the chance that a service and editable definition appear interchangeable. Prefer it if actual users
repeatedly predict hidden edit rights after choosing a public call. Do not retain common discovery
solely because reviewers agreed on its labels. Canonical structured regions and executable standard
replacement remain larger alternatives in BP1, not alternate navigation arrangements.

## Annotated shell and responsive layout

At 1280 CSS pixels and above, a 216-pixel navigation rail holds brand, selected connection and four
destinations. The top 56-pixel context bar names the owner, actor, freshness and current object.
The work area has a resizable outline (240 pixels initially), diagram or main content, and an
optional 360-pixel inspector. A bottom evidence drawer opens only on request. These dimensions are
defaults, not fixed widths that defeat text enlargement. Every resize also has buttons and numeric
size controls. At 900–1279 pixels, the inspector replaces a panel; below 900, outline/content/details
become explicit tabs. At 320 CSS pixels, forms and text reflow without page-wide horizontal scroll;
the diagram has its own labeled pan/zoom viewport and a fully usable outline alternative.

| Annotation | Visible content and interaction | Meaning that must persist |
| --- | --- | --- |
| A: connection selector | Name plus stable owner suffix, actor, connected/stale/locked status; Add connection; inspect connection | A user label and URL are not identity. Switching selection never changes the owner of an open draft, request or run. |
| B: object heading | Human title; Definition / Private draft / Run / Invocation badge; owner-qualified exact identity in details; breadcrumb back | Duplicate titles are allowed. A badge distinguishes proposed work from accepted facts without relying on color. |
| C: primary action | Save revision, Start run, Invoke, Review change, or Publish, according to destination; secondary details show exact target | No generic Run button that silently switches between public invocation and workflow start. |
| D: outline and diagram | Same selection, node/edge identities and validation diagnostics; toggle Diagram / Outline; find node | A diagram is a view of the selected draft or revision. Selection never becomes workflow state. |
| E: inspector | Purpose, input/result bindings, capability/pin, conditions/bounds, authority consequences; occurrence/evidence tabs during runs | Definition fields and historical occurrence facts are separate labeled sections. |
| F: attention drawer | Owner-grouped approval, failed verification, retained uncertainty, stale request, conflict; count only known items | This is a bounded projection of current authorized reads and local recovery records, not a global scheduler or audit database. |
| G: history/evidence | Exact event window, attempt, result, proposal or receipt; older/newer page controls and omissions | A partial page is labeled. No unseen history is inferred as success or zero failures. |

URLs contain a local connection alias and exact object identities, never credentials, prompts,
artifact content or draft JSON. A copied link requires a matching authorized connection on the
recipient's device. Resolving a link does not silently bind an alias to a different stable owner.
Browser back/forward restores the view and selection, never resubmits a mutation.

## Connection and first useful work

With zero connections, show the unchanged symbol, a short purpose statement, Add connection and
Open saved request. No fabricated sample dashboard is shown. Add connection accepts a display
name, an explicit origin/base URL and a bearer token. The token belongs only to the current tab's
session. Connection verification negotiates `/v1/version`, reads `/v1/authority`, then requests
only permitted health/catalog data. Version/authentication/network failures remain distinct.
The browser may hide the precise CORS/network cause; say “The browser could not read this host”
with an operator diagnostic route, not “wrong token” without an HTTP refusal.

The initial supported topology is a user/operator-hosted static workbench with a configured set
of approved endpoint origins. Its delivery CSP and each daemon's CORS policy both apply. Add
connection can use an endpoint in that deployed set; outside it, show “This workbench deployment
has not enabled this origin” and the operator setup path. A general public website accepting any
private host is not yet a qualified deployment. The early human checkpoint can reopen this setup
cost and choose the fixed-target proxy alternative.

Successful verification displays the stable host identity, actor and grant summary before any
work is submitted. Existing owner/actor/grant identity restores the relevant local records.
Changed grant or replacement host opens the quarantined-record explanation in engineering F2;
it does not silently clear or authorize old content. A person may retain several independent
connections and continue on B when A is unreachable. A workflow-enabled host exposes Work;
an execution-only host exposes its Operations and authorized Resources, with a precise explanation
that workflow creation/publication is unavailable. Hiding an inapplicable route is not a grant.

Version/authority discovery itself requires a bearer. The person therefore trusts the selected
origin and its TLS/operator setup to receive that credential before durable host comparison is
possible. Stable host checking protects later saved-command/private-record reuse; it does not
promise the bearer was withheld from a replaced daemon at an already trusted origin.

The retained first journey is: connect → New work → name a method and declare one named input →
add a model task using an authorized capability → bind that input and a declared output → inspect
the generated graph and validation → Save revision → upload a bounded input → Start run → inspect
the accepted result → run the same exact method with a second input. The person can choose another
available model or a maintained template; no local inference or provider credential is placed in
the browser. Saving does not start execution. The start sheet shows exact owner/revision, named
artifact inputs, available allowance facts and a recoverable request identity before submission.

At first private draft or effectful request, a compact storage choice explains this browser's
local retention: explicitly affirm a trusted personal browser to keep bounded drafts/recovery
records, or use session drafts and exported recovery files. Device trust is never inferred and
neither option silently preauthorizes the other. Tokens are never saved. The choice is remembered
as nonsecret metadata. Storage
refusal/quota exhaustion presents the export path before the effect, not a reassuring success
toast after a failed recovery write. Exported files contain private request material, visibly
identified at export, and no bearer credential. Engineering F2 defines exact recovery constraints.

## Authoring, diagrams and prospective change

An editor has a base revision, a private candidate and validation status. The draft banner says
“Only in this browser” until a daemon stores a new immutable revision. Authoring uses existing
`author_blueprint` conveniences where its supported fresh-model contract fits; advanced forms
submit `construct_blueprint` mutation drafts. Both get canonical identity and final validation
from Rust. A draft is not made executable by client graph checks. Accepted older revisions remain
openable and never acquire the candidate's geometry or behavior silently.

| Author intent | Form/diagram interaction | Contract and refusal treatment |
| --- | --- | --- |
| Run an external action | Task form selects capability/operation, inputs, context, result contract and bounds; model-specific fields only for model operations | Existing Task mutation. Catalog snapshot helps selection; actual admission rechecks generation/authority. Unavailable selection is retained and diagnosed, never replaced silently. |
| Choose one path | Branch form lists condition ports in the actual priority order, an explicit default and data references | Existing Branch; port-key priority is visible, not implied by edge position. Reordering must change the owning definition fields deliberately. |
| Continue after a choice | Show the proposed N1/R-C01 merge interaction only once the executable remedy is delivered | Current direct reconvergence refuses, as the [construction probe](merge-construction-evidence.md) proves. Before that, explain the existing pinned-child/separate-terminal construction and its extra boundary; do not draw an executable ordinary merge. |
| Perform parallel work | One compositor sheet asks branches, completion policy and separate result construction; advanced view exposes each owner | Existing Fork, Join and Reducer semantics remain distinct. A meaningful owner-side convenience may construct the complete mutation batch; the UI must not own cancellation or reduction rules. All/Any/FirstSuccess/Quorum and loser cancellation consequences use N1 wording. |
| Wait for time or input | Time wait or typed-signal choice; signal type, correlation, delivery mode and payload schema visible | Wait and SignalWait remain different. Signal delivery uses `signal_run` with stable signal identity. A signal accepted by the owner is not proof every wait resumed. |
| Repeat bounded work | Select exact body revision, inputs, condition/timing and hard bounds; inspect one iteration's occurrence separately | Existing Repeat; no arbitrary back-edge authoring. Bounds come from the definition/owner, not zoomed canvas cycles. |
| Use an existing method | Finder shows exact locally stored pin and interface; expanded definition is inspection, Edit source is a distinct navigation | Existing Subworkflow/instantiate/upgrade mutations. Upgrade compares expected old pin and new interface; running accepted children retain exact meaning. |
| Finish a scope | Terminal outcome and declared result bindings, distinct from incidental task completion | Existing Terminal. End-of-scope does not certify remote physical cancellation. |
| Connect input/result data | Select source output and target input in a two-step form or click-based port picker; preview type/provenance and control dependency separately | Existing control/data/context edges and interface mutations. Incompatible wires show owner diagnostics; dragging is optional. |
| Change interface/agreement/metadata | Explicit structured fields with human consequence preview and daemon validation | Existing SetInterface/SetAgreement/SetMetadata. Agreement changes are never tucked inside an ordinary repair and may refuse under governing obligations. |

Node types, edges, control/data activation and runtime overlays follow the reviewed notation
profile; F1 does not invent a second notation. A selected node is focusable and has a short accessible
name including type and actual state. Tab moves between the graph widget, outline and inspector;
arrow-key navigation is documented and does not trap focus. Outline rows expose the same Add,
Edit, Connect, Inspect and Remove operations as graph menus. Delete/Backspace in the renderer must
not immediately mutate meaning: open the exact removal edit, its related-edge consequences and
validation. Undo/redo affects only the private candidate; accepted history requires a new revision
or prospective proposal. Layout undo is separately labeled.

Collapse stores presentation only. A child shows owner/pin/contract and whether expansion is
authorized. A public service has no fake collapsed private graph. Large graphs disclose hidden
offscreen nodes and selected neighborhood; Fit selection, Find, zoom controls and an outline
remain available. Long labels wrap to two lines with a full inspector label; exact identities are
copyable without relying on truncation. Control/data edge styles and explicit labels differ beyond
color. No moving particle animation is used as execution evidence.

Compare revisions in a synchronized outline and diagram: added/removed/changed entities and input,
obligation or capability changes. Unchanged locations may align visually; alignment does not imply
semantic equivalence. The authoring guard checks the envelope's exact base against the draft's
immutable base, not a mutable workflow head. Two authors may save distinct valid descendants of
the same base. Display that divergence and retain the selected exact revision; do not call the
other save an automatic conflict or silently choose the newest as authoritative. A real base/guard
mismatch preserves the candidate and offers Compare/Reconstruct against an explicitly chosen base.
There is no automatic semantic merge. Presentation layout
conflicts offer keep local placement or load current placement, with a new guarded `put_layout`.

Goal entry has a deliberate planner choice, selected context and allowance, then **Request a plan**.
It produces a separately inspectable candidate using an ordinary authorized model/tool path.
Invalid/incomplete output stays as model output with diagnostics; it never appears as an accepted
plan. Current generic construction/proposal readers exist, but a reusable goal-to-definition
planner consumer and its output contract are not established by the narrow model editor or a
single example. Remedy R-FE02 below is required for the advertised goal journey. A human can edit
or reject the candidate; agent preauthorization is a separately inspected owner policy. Neither
Svelte nor a goal string supplies an agent's authority.

For an active run, **Change future work** opens base/observed sequence, the proposed change,
affected future occurrences, retained accepted facts and active/uncertain obligations. The flow
is prepare candidate → `submit_proposal` → inspect owner reconciliation → exact `decide_proposal`
→ exact `apply_proposal`. These are separate durable states, even when one authorized person can
perform all steps. A pending decision links to its exact digest/revision. A changed run sequence
requires reread; do not approve a visually similar replacement with the previous decision ID.
`prepare_model_repair` is shown only at its supported paused failed-model hold. Other changes use
the full proposal path. Fixed verifier/evidence targets appear as protected obligations and an
attempted weakening receives the actual refusal, not a “repair succeeded” result.

## Run, invocation and recovery interaction states

| State | Presentation and allowed next action | Meaning preserved |
| --- | --- | --- |
| Initial loading | Skeleton labels, cancel observation, owner context remains visible | No fabricated zero counts or success states. |
| Authorized empty list | “No permitted items returned”; create/filter actions if applicable | Does not prove no private items exist. |
| Read denied | Explain operation/object scope if disclosed; retry after a new authorized session; retain exact link | Do not probe adjacent IDs or substitute another owner's data. |
| Stale/disconnected | Last checked time and stale badge; local drafting allowed; write disabled until authority refresh | Work may continue remotely. Closing/aborting observation does not cancel it. |
| Request recorded, sending | Show saved exact ID; disable duplicate gesture; navigation safe | An optimistic spinner is not accepted work. |
| Response unknown | Persistent attention item; inspect a known object where authorized, use serving request lookup, or perform operation-specific exact replay | There is no generic command-receipt lookup. Missing/denied read does not prove nonacceptance; never create a replacement ID automatically. |
| Conflict | Show exact failed guard and refreshed state; Compare/Reprepare as appropriate | A changed request uses a new command ID; a historical replay keeps the original bytes. |
| Waiting/approval required | Name the wait/decision owner and permitted action; inspect exact policy/evidence | No client timer, approval button or graph color advances the daemon. |
| Capability unavailable | Preserve selected pin/generation and diagnose readiness/grant; explicit prospective change | Automatic fallback to another tool/provider could change accepted work. |
| Failed verification | Read actual evaluation and failed criterion; retain candidate/evidence; propose permitted repair | Failure is useful evidence; the candidate is not published. |
| Cancellation accepted | “Cancellation requested”; continue bounded observation; show retained/unknown effects | Acceptance does not certify physical stop or release held resources. |
| Retained uncertainty | Exact attempt/use and last durable evidence; owner-permitted Query/Retry/Compensate/Retain/Resolve actions | A new retry is a consequential decision, distinct from replaying a command receipt. |
| Completed result | Accepted output references, provenance and limits; Download/Use again/Select evidence | Intermediate attempt output and omitted/restricted result data are labeled separately. |
| Truncated/overflowed view | Explicit count/window and more/previous controls; stop/refresh on protocol overflow | Never silently drop evidence while presenting a complete history. |

Runs show current plan plus historical occurrences pinned to their actual revision. Selecting an
occurrence opens its attempt, bounded context/provenance, evidence and output. The accepted result
panel consumes `/result`; it does not find the “last green node” and call its output final. Proposal
and controller decisions remain reachable when the graph is collapsed. Streaming reconnect first
uses the last handled cursor; resync replaces the relevant projection from reads. Direct/service
invocations use their own observation pages, not invented run SSE identities.

## Reuse, service publication and independent hosts

| Interaction | Existing public operations and inputs | Recovery, restricted view and needed remedy |
| --- | --- | --- |
| Reuse exact method on A | Read exact local revision/interface; construct `InstantiateSubworkflow`, or `UpgradeSubworkflow` with expected pin | A readable revision on B is not automatically present on A. Explicit export/import requires source disclosure and destination validation; identity-bound copy refuses. No cross-owner transaction is implied. |
| Independent copy | `copy_blueprint` with source revision, new workflow ID and name | Explain new lineage and no authority/run transfer. Governed identity refusal offers inspect/reference choices where valid, never stripping the agreement. |
| Invoke public service | `/v1/execution/catalog`; `/v1/invocations/prepare`; exact prepared POST `/v1/invocations` | Show advertised owner/capability/generation, input/result contract and allowance. Save request before submit; recover through request lookup/exact submit. Caller sees only public observations and permitted outputs. |
| Inspect implementation when separately permitted | Existing run/revision reads under their own authority | R-A04 adds the safe invocation→internal run/revision relation. Until delivered, no guessed internal ID or automatic expansion; public call remains usable. |
| Offer a workflow as service | `prepare_method` draft → review derived agreement/service grant/limits → `publish_method` exact document and expected prior version | Must be workflow-enabled with configured service authority/accounting. Preparation creates no publication. Refusal preserves draft. Published generation/receipt is the recovery anchor. |
| Inspect/retire offering | `list_methods`, `inspect_method`, `retire_method` with capability/generation/expected version | Admin views differ from public catalog. R-A05 must establish bounded narrow-grant listing behavior. Retirement removes future selection, preserves accepted calls. |
| Connect to another host | Add independently verified client connection; configured peer operations in connection details when granted | Client connection is not peer trust. No automatic relay, host role upgrade or workflow migration. Direct peer-mapped tools require the provenance supported by their existing adapter. |

The publication sheet names public inputs/outputs, documentation, exact implementation, service
grant, limits, accounting and workspace obligations. It uses owner-returned preparation output
for the final commit. Editing it afterwards invalidates the reviewed preparation; reprepare and
use a new command ID. A service caller never sees a publication administration form simply because
the catalog item came from a workflow. Owner/actor/generation and public-versus-internal view remain
visible after leaving the finder and after reconnect.

## Direct operations and managed resources

Operations supports standalone model/process/tool calls and execution-only hosts without creating
a synthetic workflow. Prepare input bindings and bounded uploads, inspect actual capability and
generation, submit exact invocation, observe, inspect output and cancel through the invocation
routes. “Use in a workflow” is separate and available only when a selected workflow owner can
resolve a supported capability with the necessary provenance. An execution host is not made a
workflow host by a UI toggle. Three-host relay remains a separate product/adapter decision.

Resources opens authorized installation and approved-recipe discovery through **R-A03**, then exact
inspection. Until that remedy is delivered, exact-name inspection is available as an advanced
operator route; it is not adequate completion of the full discoverable resource journey. A recipe
row describes approved purpose, generation/digest and prerequisites, not raw engine arguments.

| Resource task | Owner operation at `POST /v1/resources` | Required interaction consequence |
| --- | --- | --- |
| Prepare and install | Managed schema 3 `prepare`, then `apply`, exact recipe reference and expected version | Preview and effect are distinct. Display installation identity, durable disposition and request key; prepare does not reserve success. |
| Inspect/start/stop | `inspect`, `start`, `stop` | Show actual services, pending work and blockers. Stop drains owned services; closing the client has no lifecycle effect. |
| Update a tool/setup | `update` with exact approved recipe and explicit interruption choice | Show old/new generation and holds; busy refuses without hidden maintenance queue. Reinspect after resolution. |
| Preserve/remove | `preserve` sets future disposition; `remove` applies the saved disposition after owner checks | Consequence panel shows exact owned objects and retention. Removing a connection is never this action. There is no promised release-from-management API. |
| Repair interrupted maintenance | `recover` for the recorded pending operation | Reconnect to exact pending identity; do not reconstruct a fresh installation or overwrite its files. |
| Transfer editing to child and return | `handoff` / `return` with exact inspected lineage/claims; `resume_parent` deliberate | Show parent/child, hold and quiescence facts. A canceled child without proven stop does not restore editing eligibility. |
| Reconcile retained use | `resolve` with inspected use ID and expected claim | Fencing a use does not claim its external execution succeeded or failed. |
| Evaluate/publish protected output | `evaluate` immutable candidate; inspect `evidence`; `publish` exact host evaluation | Keep failed/incomplete verification visible. Uploaded reports and browser-computed scores cannot substitute for trusted evidence. |

The busy installation view links authorized child occurrences and affected work where evidence
permits. Missing internal access remains a restricted blocker explanation; it does not invent a
child tree. Current editing, preserved files, immutable artifacts and active service state use
different labels because their lifetimes differ. Updating a recipe may itself require operator
configuration outside the UI; show that boundary precisely instead of offering arbitrary mounts
or shell execution as a setup shortcut.

## Knowledge and evaluated improvement

Improve begins from a selected method/run/resource result or the bounded authorized learning
receipt list in R-FE01. Its sequence is **Select sources → Declare evaluation → Propose candidate →
Run declared comparisons → Compare → Decide prospective promotion**. Each step opens its exact
retained receipt and applicable authority. A single attractive “Learn” button cannot hide which
source or held-out evidence was given to the proposer.

`learning` commands use the strict request owner in
[learning/request.rs](../../../../../crates/control/src/learning/request.rs). `select_sources`
accepts method, managed workspace, guidance artifact, at most 32 supplementary artifact identities,
and exact permitted run pages; the owner resolves immutable references. The UI shows selected
page ranges and known omissions before freezing selection. Mutable working notes can be exported
as a new immutable selection, never edited retroactively in an old task's context.

`declare` fixes held-out pairs, criteria, target/configuration and allowance before proposal
generation. Only the evaluator's authorized view displays held-out content. `candidate` binds
the declaration, ordinary structured model proposal/provenance, expected benefit, applicability
and counterevidence. These are hypotheses. Existing ordinary runs/invocations perform the declared
work; `compare` derives results from retained journals. Missing, failed, over-budget and inconclusive
comparisons remain first-class outcomes. UI charts describe the actual bounded study and do not
claim general model superiority.

Promotion is separate: `promote` consumes an eligible exact comparison, full publication template
and expected publication version; optional `preauthorize` fixes the executor/template in advance
and `auto_promote` checks the resulting exact policy/comparison. New study, target or template
requires new authorization. `inspect` recovers by `{actor, command}` receipt reference after restart.
The product must allow rejection and preservation of a candidate without a “clean up failures”
step. Denied evidence is visibly omitted/refused, not silently substituted with accessible examples.

## Visual language and accessibility contract

Use the unchanged [Milkdrift SVG](../assets/milkdrift-logo.svg) in a 32-pixel dark brand slot with
wordmark text. Keep its aspect ratio and path bytes; no recolor, animation, new metaphor or font
binary. A favicon may use the same full symbol after small-size inspection; do not redraw it.
The icon beside visible Milkdrift text is decorative; an image-only brand link has a meaningful
accessible name. Copy the approved source to the chosen static asset path with byte comparison.

| Token | Proposed value and purpose |
| --- | --- |
| Canvas / panel / raised | `#121821` / `#1A2330` / `#222E3C`; sharp layered surfaces without blur/glow |
| Text / secondary / border | `#F8EAD8` / `#BAC4CE` / `#596879`; secondary information remains readable |
| Accent / selected / focus | `#B7CCDE` / `#2E4964` / `#FFD58A`; 2-pixel focus with offset |
| Success / attention / failure | `#A2C998` / `#E0BC75` / `#EFABAA`, always accompanied by symbol and text |
| Typography | System sans; 16-pixel body, 14-pixel dense table, 12-pixel minimum optional metadata; 1.5 body line height; 20/28-pixel headings; system monospace for identities/code |
| Spacing and controls | 4-pixel unit, 8/12/16/24 spacing; 44-pixel ordinary targets; compact controls preserve at least WCAG target rules and spacing; 4/8-pixel corner radii |
| Motion | 100–160 ms panel/selection transition only; reduced-motion disables it; no animated status that replaces text |

Tokens are a concrete candidate, not a measured accessibility result. Validate each actual
foreground/background/state combination, focus under dialogs, forced colors and 200/400% zoom.
Use semantic headings, forms, buttons, tables, fieldsets and live regions before custom ARIA.
Announce route titles and consequential state changes, not every streamed event. Keep focus on
the selected occurrence across bounded refresh; move it to an error summary after invalid submit
and return from dialogs to a still-existing trigger or the object heading. Destructive or effectful
confirmation names exact owner/object and result; routine reversible field edits do not need a
confirmation ceremony. Keyboard and non-dragging pointer authoring are separate requirements.

The target is WCAG 2.2 AA for the supported app, qualified by automated checks plus keyboard,
screen-reader and pointer review of actual journeys. Neither Svelte Flow nor a headless primitive
library certifies the result. Text-only/outline operations must cover every supported mutation,
including wiring, nested calls, branch priorities, joins/reducers, signals and proposed changes.

## Explicit remedies and independent review requests

| ID | Gap and complete owner obligation | Delivery and falsification |
| --- | --- | --- |
| R-FE01 | Add bounded authorized learning receipt discovery to the existing durable receipt/control owner; return exact `{actor, command}` references and permitted summary fields, with stable cursor/retention behavior. Existing `learning.inspect` remains the detail authority. No browser-maintained study database. | P07: a fresh browser finds an authorized old failed/inconclusive study without pasted JSON, while a narrower grant learns no forbidden study existence. Source inspection found Inspect but no list; architecture must check archival/query costs before choosing route shape. |
| R-FE02 | Supply a maintained Rust-authored goal-planning consumer/template and versioned structured output contract using existing authorized external model/tool execution and ordinary construction/proposal validation. Define context/allowance, malformed/refused output, no model response, exact provenance and restart recovery. | P04 or an earlier coherent slice: goal → inspectable candidate → human edit → saved/run revision, plus rejected candidate. A generic model picker or Slotbook-specific proposal fixture alone does not establish the reusable journey. If existing consumer proves sufficient, document that path and retire the remedy rather than add a planner service. |
| R-A01–05 | Browser admission; client identity/recovery; resource discovery; private invocation relation; scoped method-list behavior | Architecture owns contracts and evidence; F1's consumers determine necessary fields and refusal states. |
| R-C01 | Ordinary exclusive reconvergence | Product/notation/runtime owner must deliver complete activation/data/cancellation and migration/evidence before the editor offers the construct. |

No essential interaction is excused as an eventual API mystery. Exact bounds/read projections
needed by forms are implementation review items under these owners. A generic permission endpoint,
global event bus, mandatory broker, unrestricted resource release and standards interchange are
not silently added by this proposal.

## Real human checkpoints and demanding acceptance

The first checkpoint is P03, immediately after the retained production P02 journey connects to
the actual daemon. The user receives a meaningful goal and two inputs, not a taxonomy tutorial:
connect, author/run, find output, change a model/input, recover a lost response, and explain which
work persisted after closing the tab. Include same-named objects on two configured owners and a
restricted view where the implemented slice supports it. Missing P1 reuse/service interactions
must be tested in the first P05 delivery and again P08; P03 cannot certify them in advance.

Capture observed action, prediction, outcome and hesitation separately from opinions. Classify
feedback as presentation friction, missing public behavior, mistaken product assumption or
model/provider limitation. Resolve the cause and rerun the affected task. The checkpoint can
reopen product meaning, notation, API or connection architecture; it is not limited to styling.
No target completion time or success percentage is claimed before measurement, and scripted
browser automation is not substituted for the real user's review.

P08 uses one connected compound project, not isolated menu demonstrations: A owns a method with
decision/reconvergence, parallel review and bounded repair; it invokes B's restricted published
service and an eligible tool on C; a child requires an already-held working area; a review fails
and a prospective change adds investigation; an authorized evaluator selects failures and declares
a held-out comparison; a negative candidate is retained and an eligible different candidate can
be promoted separately. The investigation then changes its own future process while fixed
acceptance obligations remain visible (S21/S22/S26/S28/S29).

During that project remove an observation connection, change a grant, restart a daemon with a
saved exact request and return in a fresh browser. Verify recovery, denied internals, cancellation
uncertainty, editing hold/return, result provenance, failed verification, and actual graph/outline/
machine-input equivalence. Every accepted fact must be recoverable through its real public owner;
every refusal must offer an honest next action. P09 performs full-system repair and evidence after
the human corrections. Controlled external capabilities are permitted fixtures; fabricated daemon
responses cannot qualify these product paths.

F1/r2 incorporates Cyra's concrete CSP/topology and full-storage cancellation critique and Ada's
operation-specific changed-grant recovery correction. [NP2](notation-profile.md) now owns the
low-zoom Choice/Merge marks, separate compositor statuses and positive selected-arm future-repair
oracle I requested. The interface preserves public-invocation context through authorized drilldown
and clears private links after lost internal-read authority. A full ordinary recovery store must
keep the reserved control-request lane or explicit export/authorized native stop route visible;
quota is not a silent reason to hide cancellation. [BR1](review-first-bram.md) records review scope
and exposure. Terminology, density, colors, graph usability, goal generation and learning
discoverability remain unproven until implementation and actual use.

F1/r3 corrects the same-base concurrent-save premise from BR3: current immutable authoring permits
valid sibling revisions; only an actual envelope/base mismatch or another owned guard supplies a
conflict. P02's revised interaction and positive sibling-save/negative mismatch cases were actually
reread. This changes the consumer explanation, not the backend or the selected product/notation
semantics. The supplied logo and all production files remain unchanged.
