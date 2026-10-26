# 0052 — Saved canvas authoring and understandable method reuse

**Standing:** selected planning design, 2026-10-23, under the user's request to consolidate the
debates and prepare a side sprint. S01 lifecycle repairs and S02 saved authoring are implemented,
with focused verification complete on October 26; S03–S05 remain unassigned. Actual usability acceptance
remains required. This refines [0051](0051-daemon-task-workflows.md), replacing its in-memory-only
authoring and left inspector, clarifying method activation and adding the contract changes below.
It preserves the daemon-owned workflow, ordinary task scheduler, current-at-activation methods,
project boundary, explicit authority and truthful execution history.

## Why this decision is ready

The user repeatedly rejected interfaces that expose data structures as forms, disrupt the canvas
while editing and leave mechanical translation to the person. Their latest direction is an expandable
daemon → project → workflow tree, empty task placement, physical connections and a right inspector.
The [canvas debate](../development/virtual-office/whiteboard/discussions/product-model/canvas-authoring-debate.md),
[daemon investigation](../development/virtual-office/whiteboard/discussions/architecture/daemon-frontend-contract.md),
[method debate](../development/virtual-office/whiteboard/discussions/product-model/method-authoring-and-use.md)
and [outcomes debate](../development/virtual-office/whiteboard/discussions/product-model/work-outcomes-and-delegation.md)
identify concrete alternatives and consequences. Their disagreements are retained as grounds for
reconsideration, not parallel implementation specifications.

There is enough evidence to select this bounded design. It is not evidence that users will find it
intuitive. The remaining comparisons belong in working user journeys, with repair before acceptance.
Adaptive method groups owning arbitrary descendants, early snapshot customization and scoped human
decision gates are deferred extensions; none is needed for the selected fixed methods inside an
adaptive workflow. No automatic model substitution is permitted. These dispositions prevent unresolved
extensions from becoming hidden implementation choices.

## One continuous workspace

Use one persistent, expandable tree of connected daemons, their projects and workflows. Expanding a
branch changes visibility, not selection. Selecting a workflow restores its canvas. Each expanded
project has a labelled **New workflow** action even when populated. Settings belong to their daemon,
project or workflow context; method insertion is contextual, not another mandatory hierarchy level.
Search a named daemon and show results with ancestry. Lazy loading, pagination and off-page references
remain explicit; disconnected branches never imply a live complete search.

The main area is the canvas, with a quiet name/save-state header and one primary execution action.
Drag Task to place an empty card; click-then-place and keyboard placement are equivalent. Placement
does not open an inspector or take text focus. Click a card to open the right inspector. Moving,
selecting, connecting and editing preserve camera, nearby tasks and unfinished work. No automatic
fit or recenter follows those actions. A narrow-screen drawer and accessible list expose the same
operations and return to the same selection.

Conceptual arrangement; arrows carry data, not elapsed time:

```text
Daemon / Project / Workflow     Workflow name       Saved       Run 2 tasks
                               [Task] [Text] [File] [Method]
                               [Brief] -> [Research] -> [Compare]   Inspector
                                          [unfinished idea]        Instructions
                                                                   Model / agent
                                                                   Uses project setup
```

The inspector follows state: instructions and missing choices for unfinished work, progress/control
for running work, result first for completed work. Titles are suggested from instructions and remain
editable. Technical identities, binding keys, output declarations and dependency arrays are never
ordinary writing fields. Useful instructions-only tasks remain possible; drawing a graph is optional.
Show effective model, project access and tool location in concise summaries, with contextual overrides.

A deliberately chosen workflow model/agent default is saved with authoring preferences, shown on
new cards and overridable per task. When unassigned draft tasks already exist, offer one explicit
action: **Use this model for these N unassigned tasks and new tasks**, highlighting the affected
cards before applying it. Choosing only the default changes new cards; neither action changes
already selected task models, accepted definitions or authority. No available choice is silently
substituted when it becomes unavailable or changes generation. Keep configuration
guidance separate from selection and return from it to the same canvas. Host connections and retention
settings later extend these contexts under the remaining main sprint, without another navigation shell.

## Connections express intent directly

Real output-to-input gestures finish the connection. Highlight compatible targets; if several named
content choices remain, show a chooser at that endpoint. Do not ask again for source and destination.
Connected and missing required ports stay discoverable; reveal other ports on focus, selection or
wiring. A wire selection highlights its endpoints without moving the canvas.

Model-unknown cards may be connected through handles labelled Input and Output. Show the unfinished
line and **Choose output/input** state distinctly. It is saved authoring intent and cannot execute.
Resolve mechanically only when both endpoints have one meaningful compatible choice or a visible
adapter-declared default. Matching media alone cannot choose between Text and Diagnostics. Choosing
a model or changing its contract preserves the intention and marks affected connections for repair.

Successful results are the ordinary data-wire meaning. Optional absence and settled outcomes,
including failure, are distinct contextual choices. Independent ordering is separate: **After success**
by default, with deliberate **After completion** when a settled outcome is enough. Data bindings
derive their own readiness; a person never encodes that ordering twice. Removing a data wire removes
only its derived prerequisite. Corrections, cycle checks and all clients use this same meaning.

New definitions distinguish independent order conditions from data-derived prerequisites. Existing
accepted definitions retain their exact behavior, including older optional/outcome readiness. Their
merged dependencies cannot be assigned invented authoring intent. A future edit shows retained
ordering explicitly and preserves its prior predicate until deliberately changed; frozen definitions
and canonical receipts remain unchanged. One shared semantic owner handles this adoption.

Text and captured files are source cards. Dropping a file supplies content, not filesystem access.
Authorized retained results use the same connection gesture. Replacing an occupied input shows what
will be replaced. Incompatibility, cycles, unavailable sources and capacity issues appear at affected
objects without destroying the drawing. Cross-daemon content transfer remains explicit, never an
accidental wire between navigation branches.

Adapter-owned metadata must expose actual input/media constraints and stable named outputs. Shared
admission rejects known incompatible connections; execution rechecks concrete content. Do not label
arbitrary process bytes as text or silently insert a converter. For new output declarations, choose
stable per-output ceilings up to the existing 2 MiB task aggregate; never divide an existing ceiling
when a second output is connected. Actual publication still enforces the shared aggregate and output
count. Reserve that aggregate once rather than summing mutually competing per-port maxima. Preserve
stricter accepted declarations. All admission, artifact accounting and correction checks adopt this
rule together; this decision promises no greater aggregate output capacity.

## Preserve unfinished work without admitting it

An empty workflow is discoverable under its project. Its authoring document contains incomplete cards,
sources, intended connections, candidate edits and a bounded usual-model choice. These are not queued
tasks, immutable execution definitions or runtime obligations. Creating or moving them consumes no
task admission or invocation allowance. Existing create/start authority remains required for workflow
creation; authoring is not a new way to obtain execution permissions.

Keep one latest mutable authoring snapshot per workflow, separate from accepted history. Admission
leaves references to accepted tasks plus any remaining candidates, not a competing copy of accepted
definitions. Positions are bounded presentation metadata with their own version; camera/selection
remain client preferences. A position change cannot invalidate execution or an authoring submission.
Project identity, actor scope and daemon identity remain attached to every read and pending action.

### Save recovery is distinct from command replay

Authoring checkpoints use authenticated compare-and-replace of the whole bounded snapshot, with
expected authoring generation. They carry no idempotency key and promise current content, not an
exact historical acceptance receipt. A stale write refuses, even if it once supplied those bytes;
it cannot overwrite a later edit. After a lost response, read the authorized current generation and
digest/content. A match establishes that the intended content is saved now, not who saved it or
whether an older attempt committed before being superseded. A difference preserves the local
candidate for contextual conflict resolution. No automatic last-writer-wins merge occurs.

Coalesce completed gestures and editing pauses; keep at most one checkpoint in flight and one bounded
newer local candidate. **Saving / Saved / Not saved** describes confirmed state. A stale response
cannot mark a newer local candidate saved. No per-keystroke receipt or task revision is allocated.
This is a new conditional editing operation, not weakened idempotency for an existing command.

Creation, Run/Apply, method Update and import/export mutations that already promise exact command
recovery retain their canonical request/result receipts. No receipt expires to make autosave work.
Run atomically checks the saved authoring generation, admits the explicitly selected work, records
its exact result and updates the draft-to-task mapping. A lost Run reply reconciles that receipt
before considering a new request, even if another client has since edited the document.

Adopt current canvas bounds as the initial bounded authoring envelope: 2 MiB per document, 200 pending
task/method cards, 400 source cards, 1,024 connections and 600 saved positions. These are operational
baseline limits, not usability findings; the Rust owner advertises them to clients. A Run remains
within the ordinary 32-action bound; a document may contain more unfinished work. Store accounting
charges latest document bytes, source protection and transient replacement capacity to finite
history/storage capacity while retaining settlement/control headroom. Explicit deletion releases
unused draft content; drafts do not expire by age before main-sprint retention is implemented.
Run, retained results and library snapshots keep their actual reference protection. At capacity,
retain the last saved version, refuse extra growth and offer explicit removal/shortening while
continuing reads and controls. This works before rolling retention; increasing caps is not recovery.

## Deliberately run the intended work

The default Run covers the pending plan shown. If an unrelated blank idea should remain, choose
**Choose tasks to run** and name the intended set. The daemon calculates necessary new predecessors
and shows them in the reviewed scope; it never silently adds them at submission. Inspector selection
does not change scope. Highlighting supplements names and changes. An invalid/incomplete included
task blocks that submission; never run a hidden ready subset or split an oversized plan.

Accepted predecessors are existing work. If one has an unapplied candidate edit, choosing its result
does not apply the edit: show **Use current task** or **Apply these changes** at that task. An output
that exists only in the candidate cannot be borrowed from the accepted version. If it starts, retain
the candidate and offer future improvement; never change frozen work. Mixed edits use **Apply changes**
with the same visible scope and single submission location. Control actions such as Stop remain
direct contextual commands, not work accidentally included by selecting a card.

The shared runtime/control owner validates semantic preconditions: affected task versions and start
state, source/binding facts, reviewed project/capability choices, authority and current limits. Pure
progress reports, readiness explanations and layout changes do not invalidate an otherwise unchanged
human edit. Use the existing decision generation for human admission too, retaining affected-task
versions/start checks and current source/project/authority validation. This deliberately remains
conservative about intervening authoring or start decisions; it does not promise automatic merging
of every unrelated edit. All public clients share the policy; frontend retry loops cannot own it.

Return bounded typed issues with authorized task/port/change locations and useful current facts.
Preserve existing retryability/details through browser transport. The UI keeps the canvas and focuses
the repair; it does not parse prose into policy. Keep current atomic admission, without a mandatory
prepare/commit transaction framework. A topology read supplies named ports, relationships and explicit
off-page references separately from result content. Keep bounded snapshots/SSE and unchanged-detail
caching; avoid unrelated session/project reloads on every workflow hint. Measure before adding an
event journal, global graph store or performance claim.

## The daemon must preserve future work

An explicit workflow decision wait remains until authorized Continue. Supplying unrelated context
does not answer it; model-owned rejection waits remain scoped to their source. Missing required input
retains ordinary readiness behavior. No generic hold registry or branch-decision product is added.

After a start freezes but before external entry, pause defers that exact start. Release any prepared
resources and worker ownership correctly, preserve non-entry evidence and frozen identity, and make
it dispatchable after Continue, including restart. It must neither become Failed merely because it
paused nor remain an unentered Running task forever. Entered or uncertain work is never automatically
re-entered. These repairs follow the [executed diagnostics](../development/verification-evidence.md#daemon-lifecycle-design-diagnostics).

Temporary native claim contention becomes a typed busy result and bounded fair waiting before
entry, not terminal task failure. Retain uncertain writers' claims and recheck authority/identity
before retry. Permanent refusal/corruption is not busy. Show a permitted blocker or **Project is in
use** without leaking inaccessible work. Preserve finite workflow limits; no artificial user-created
ordering is required just to serialize legitimate file access.

## Methods are reusable procedures in this same workspace

Methods remain daemon-wide, owner-editable templates; a consumer cannot modify the library merely
by using one. The ordinary method is a fixed reusable segment in an adaptive workflow. Copied
planners may create ordinary workflow follow-ups under their grants, but their descendants do not
silently join a method's completion group or move its exports. Show their cause and workflow state.
A method does not promise “until acceptable” merely because it contains a reviewer.

Create a method from a clean selection or a clearly labelled **Editing method** canvas. Derive
external input boundaries and candidate exports from connections; allow direct exposure of a real
output when no outgoing wire exists. Generate identities, binding rewrites and compatible constraints.
The person chooses what repeats: per-use input, deliberately retained content, exposed result or
start-after condition. No parameter syntax or schema table is required. Preserve distinct meanings
even when inputs happen to have equal values.

Extraction copies selected procedure definitions, not completed states, historical results, retries
or correction/adoption facts. A corrected task may supply improved instructions without copying its
old failure target. Selecting a review and a correction produces two future steps, not an inferred
loop. Private artifacts and the author's absolute project paths are not silently embedded. Unsupported
portable extraction gives a local explanation and preserves the source work. Show included instruction
content; do not claim automatic sanitization. The shared daemon validates every save/extraction.

An incomplete method editing document uses conditional checkpoint semantics above and never changes
the current usable template. **Update method** installs a complete validated body with guarded exact
command recovery. The contextual chooser inserts a live reference into a workflow. Its expandable
current preview is read-only and says **Uses latest saved method when ready**. **Edit method** is an
explicit library action; returning restores workflow context. Try a method through an ordinary
deliberately run use in a chosen project with real effects. Inspection preview executes nothing.

### Activation, changes and results

Select these rules in addition to 0051's atomic snapshot/recovery contract:

- A use activates when its required root inputs are available under ordinary binding rules and its
  explicit ordering conditions hold. Optional future-result bindings wait for their producer to
  settle; explicitly absent optional supplied inputs need not wait. Recheck current root requirements
  before accepting the snapshot. A use with no outstanding gate can activate immediately. Independent
  inner work waits for this boundary; use separate top-level methods when earlier parallelism matters.
- At acceptance select current root and nested bodies in one bounded consistent snapshot. Retain
  parameter bindings, origin and effective reviewed execution settings. Ordinary task starts still
  freeze actual input bytes; expansion does not create another input-freeze owner. Nested uses do
  not select newer content independently later. Keep 0051's depth 8, 64 flattened tasks, 256 nested
  occurrences and 1 MiB combined content bounds, charged against the existing workflow account.
- A queued use records stable interface identities and reviewed model/capability selections plus
  effective location/file/tool choices. Compatible content/topology edits follow automatically within
  those choices and limits. Different models/generations, wider access or changed location require
  a contextual refreshed choice. Authority remains independently necessary. Renames preserve wires;
  removed/recreated or incompatible endpoints block locally. Same-type semantic equivalence is not
  claimed; following the method includes trust in its owner's stated purpose.
- Exports default to release after their producer and all required copied work succeed, including
  accepted correction satisfaction. The author may explicitly expose an early result with a narrower
  visible release condition. The shared owner derives the completion edges once, detects cycles and
  preserves them through local edits/corrections. Inspecting available content is separate from
  releasing it to downstream execution. Optional/outcome exports remain explicit.
- A group's status describes the copied obligations and their named exports, not a new child runtime
  or an objective-quality verdict. Dynamic follow-ups remain workflow work. A reviewer successfully
  returning negative findings has not approved the result. An actual approval claim needs an
  attributable decision; the product does not invent one from prose or green task status.
- Local edits affect only unstarted copies and validate their exports. There is no guaranteed editing
  window after activation; expose that fact rather than relying on a UI delay. Optional early snapshot
  customization and per-use materialize-and-hold are outside this sprint. Ordinary parameter choices
  and eligible future-copy edits remain available.

Library deletion remains refused while queued uses or nested saved references need it. Accepted
expansion holds, atomic materialization, stop/remove races and exact recovery retain 0051's rules.
Local improvements can be compared and deliberately applied by an authorized library owner; success
never automatically updates the method. Cross-daemon export/import remains a bounded content copy
with new local identities, explicit model/tool mapping and provenance, no credentials or live link.
Repeated import creates another copy; updating an existing local method is a separate reviewed edit.

## Completion and control remain understandable

Show execution completion, critique findings and attributed decisions distinctly. **Nothing running ·
1 unfinished task** composes admitted status with saved authoring; it does not require a new Closed
workflow state. Improve a successful-but-inadequate result through future work without calling the
earlier execution failed. Failed/stopped obligation adoption keeps its narrower real meaning.

Explain that granting model follow-ups can create and run work automatically within limits. Repair
of refused action batches is distinct from a method that reviews and fixes a deliverable. Routine
authorized continuation needs no repeated human approval. Workflow Pause remains global; this sprint
does not advertise branch-specific decisions or notifications it has not implemented.

## Owners, adoption and reconsideration

The existing workflow/control owners own authoring and admission meaning; blueprint owns validated
contracts, runtime owns readiness/entry, persistence/redb owns conditional snapshots and transactions,
adapters own real port/media facts, and clients own gestures and presentation. No second scheduler,
method service, graph authority or frontend-only semantic construction is introduced.

Preserve current accepted tasks, frozen starts, exact receipts, project bindings and protected
artifacts. Add authoring state without rewriting those facts. Coordinated protocol/schema changes
must adopt actual current supported stores through checked transactions; unsupported old readers
refuse before mutation. Do not claim every historical development format is compatible or reset
stores to avoid adoption. Update all current producers/consumers and remove replaced semantics from
their ordinary path. Read-only interpretation of older accepted meaning is not another writer.

The [side sprint](../development/virtual-office/canvas-experience/README.md) owns implementation and
acceptance. Reopen the relevant choice if actual users mistake unfinished wires for executable ones,
cannot explain inherited models or run scope, need internal method customization routinely, or find
extraction harder than rebuilding. A failed CAS/concurrency/recovery case reopens its contract before
delivery. The larger adaptive-method and scoped-decision proposals remain separate follow-up topics;
their deferral does not mark the entire product finished. The main sprint still owns remote tools,
rolling retention, combined review and its final integrated gate.
