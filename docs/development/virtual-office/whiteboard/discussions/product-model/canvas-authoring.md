# Arrange work first, define it in place

**Consolidation, 2026-10-23.** [ADR 0052](../../../../../decisions/0052-canvas-authoring-and-method-experience.md)
now owns the selected holistic design; the [side sprint](../../../canvas-experience/README.md) owns
its prepared implementation and actual-user checks. The discussion below retains its proposal,
alternatives and review grounds. Selection is not implementation or usability acceptance.

Milkdrift should let someone arrange an idea before making it executable. Choose a workflow,
place empty tasks, join their inputs and outputs, then define each task without leaving the
canvas. The daemon should translate that intent into task definitions and bindings.

This discussion develops the user's October 23 direction. The
[review record](../../../daemon-workflows-implementation/handoff.md#actual-user-review-and-corrections)
owns their feedback; the [whiteboard overview](../../README.md#overview) owns the next action.
The interaction below is a proposal, not an implemented or user-validated design. In particular,
durable unfinished work is an agent recommendation to debate, not an already selected daemon contract.

The [UX debate](canvas-authoring-debate.md) records the user's design criteria, actual exchanges
between three delegated reviewers and their revised positions. Judge each interaction by the
translation, repetition and recovery work it removes from the person. Preserving thought and place
matters as much as reducing clicks; hiding the same work in a smaller form is insufficient.

The subsequent [daemon/frontend debate](../architecture/daemon-frontend-contract.md) challenges
the existing runtime and contracts with source traces and executed pause diagnostics. Its findings
qualify the proposed backend work below: convenience endpoints alone cannot repair lost connection
meaning or a changed output promise, and saved drafts do not require a new completion lifecycle.

## One place to work

Use one expandable navigation tree: **daemon → project → workflow**. Selecting a workflow opens
its canvas. Expanding a daemon or project only reveals children; it does not change the selected
workflow. Show all configured daemon connections, with connection status beside each name.
Duplicate project or workflow names remain understandable through their ancestry.

Conceptual desktop sketch; arrows inside the canvas carry results into task inputs:

```text
 Navigation                    Launch announcement           Saved       Run 2 tasks
 ──────────────────            ─────────────────────────────────────────────────────
 ▾ Studio A                    [Task: drag to place]                Selected task  ×
   ▾ Product                                                      Draft announcement
     • Launch announcement     [Brief] ──▶ [Read brief]             ──────────────────
     • Release notes                          │ Answer             Instructions
   ▸ Research                                 ▼ Context            [Write a short ...]
 ▸ Studio B                              [Draft announcement]      Model or agent
                                                                  [Writing assistant]
                               + empty space to arrange work      Uses Product setup
```

The right inspector is absent until a task is clicked. Opening, changing or closing it preserves
the canvas camera and positions; do not automatically fit, recenter or navigate to a task page.
The October 23 right-side preference supersedes the earlier request for left-side details in
this proposed replacement. The existing [vision](../../../../../product/vision.md#the-client-makes-the-work-understandable) and
[ADR 0051](../../../../../decisions/0051-daemon-task-workflows.md#browser-workspace-interaction)
describe the earlier selection; an adopted replacement must reconcile those owners explicitly.

The tree replaces separate daemon, project, recent-workflow and workspace navigation groups.
Keep project creation in the daemon row menu and settings beside the thing being configured.
Show a labelled **New workflow** action in every expanded project's workflow list, including a
populated project; creating the second workflow must remain discoverable. Keep pagination separate.
A first-use empty branch has an inline creation action.
Workflow search is scoped to a named daemon and displays matches under their project ancestry,
using the existing bounded search and its truncation notice. It must not search only already loaded
children or imply a search of disconnected daemons. Load expanded branches on demand, with bounded
pagination and an explicit way to load more; the visible page is not the complete collection.
An unavailable daemon keeps a clearly marked cached branch where disclosure remains authorized.

The workflow header contains its name, save state and one primary action for the pending work.
Canvas tools stay small and near the canvas. History, connection setup, project settings and
diagnostics appear when requested. Running work and failures appear on their task cards, so a
person can see where attention is needed without reading a second dashboard.

## A complete ordinary interaction

The requested core is empty placement, details on the right and direct port connections. This
walkthrough additionally proposes saving incomplete work. Wiring placeholders before choosing their
models is one comparison candidate. A model/agent chosen once for the workflow could instead give
instruction-empty cards real ports; definition before wiring remains the smaller alternative.

1. Expand **Studio A → Product** and choose **New workflow**. The canvas opens immediately with
   an editable suggested name. The proposed daemon change below saves the empty workflow.
2. Drag **Task** onto the canvas twice. Each drop leaves an empty card at that position, with
   input and output handles. It opens no inspector, takes no text focus and submits no task for
   execution. Clicking the palette and then the canvas is an equivalent placement operation.
3. Connect the first card's output to the second card's input. With a visible inherited model,
   these can already be real ports. In the model-unknown sketching candidate, the relationship
   retains an open endpoint until defined; it does not invent a model contract. Both candidates
   let the person arrange relationships before writing the instructions.
4. Click the first task. The right inspector asks for instructions and a model or agent when one
   has not been deliberately chosen or inherited from a visible workflow choice. Give it a brief
   to read by dropping a file onto its input;
   the file becomes a visible source connected to that task. Select the second task and describe
   the announcement to write. The canvas stays in place throughout.
5. The chosen capabilities supply real input/output labels and compatibility rules. Resolve an
   unfinished wire automatically only when each end has one actual port choice, or a declared
   default is visibly presented, and those ports are compatible. If outputs express different
   content, ask at the unresolved endpoint; type filtering alone cannot choose the intended content.
   Resolution fills mechanical details under the visible successful-result default; it cannot
   silently select optional absence or failure acceptance. It never submits the drawing.
6. Choose **Run 2 tasks**. The daemon validates and accepts the complete pending plan atomically.
   Each card shows its actual state; selecting a completed card opens its result in the same
   inspector. A failed card offers the relevant correction or recovery action there.

Creation needs no name or schema form. A temporary title identifies an empty card; instructions
can suggest an editable title. No generated identifier, binding key, dependency list or output
declaration is a writing field. A task with instructions alone remains useful when its capability
needs no other input; drawing a graph is optional.

The inspector's contents follow the task's state. An unfinished task shows what it needs to become
ready. A running task shows progress and available control. A finished task shows its result first.
A concise execution summary keeps the effective project environment and access visible; expose
overrides only when the user chooses to change them. Propose one explicit usual model/agent choice
for the workflow, visibly inherited by new tasks with task overrides when needed. It removes
repeated selection while leaving each task's instructions empty. This is a new proposed default,
separate from project execution settings; it grants no additional access. Unavailable choices
never silently fall back to another model. Setup guidance returns to the same canvas and selection.

## A connection should finish the connection

Draw from an actual output port to an actual input port and the wire appears immediately when
the choice is valid. The gesture already specifies the source, destination and port; asking for
them again in a form makes the user repeat work. While dragging, highlight compatible inputs and
show their labels. On a compact card, reveal extra ports on selection, focus or wiring rather than
printing every capability field permanently. Connected ports remain discoverable.

In the model-unknown sketching candidate, empty cards have handles labelled **Input** and **Output**.
Their wires express an unfinished intention through a distinct line style and an open endpoint
with an action such as **Choose output**, without teaching a new protocol term. Until real contracts
resolve both ends, those connected tasks cannot run. A model change preserves the relationship but visibly
marks any now-invalid connection for repair. A generic handle must never pretend that every
capability produces the same answer or accepts arbitrary files.

| Situation | Interaction and consequence |
| --- | --- |
| One compatible source and input | Commit the connection with the gesture; generate protocol details internally. |
| Several real outputs or inputs | Offer a small labelled chooser at the endpoint. Do not open an unrelated settings form or re-ask the destination. |
| An input already has its allowed value | Offer Replace or Cancel, showing what would be replaced. Do not silently discard the previous source. |
| Incompatible ports, a self-connection or a cycle | Explain the problem beside the endpoint and retain the existing drawing. |
| One result feeds several tasks | Show separate wires from the same output. Do not duplicate the source or silently change its output allowance. |
| A result must be handled even if its task fails | Offer an explicitly labelled outcome connection. Keep this distinct from making a successful-result input optional. |
| A task only needs to run after another | Provide a separate, labelled ordering relationship when requested. A data wire already supplies the necessary ordering. |
| A wire is selected or removed | Highlight its endpoints and show the affected future input. Preserve independent ordering requirements and frozen inputs of started tasks. |

Default result wires carry successful results. Failure handling is a meaningful decision and
must remain understandable; schema construction is work the system should do. Selecting a wire
can expose its plain-language behavior without turning every ordinary connection into a questionnaire.
For old accepted definitions whose independent ordering cannot be inferred, preserve it and show
the remaining ordering relationship after removing a data wire.

Text and captured files are source cards that can feed more than one task. A retained result can
be selected through an authorized result picker and then connected in the same way. Keep references
outside the visible page explicit; do not expose unrelated work to make the canvas look complete.
Dragging between daemon branches does not authorize a cross-daemon connection. Project folders
and tools use the existing execution settings; dropping a browser file supplies content and grants
no filesystem access.

Keyboard and touch users get the same operations: add at a chosen position, select an output,
choose a compatible input, and edit the selected task. On a narrow display, a dismissible drawer
and a navigable task list preserve the current workflow and selection. A pointer drag must not
also trigger the click that opens the inspector.

## Saving and running are different actions

The recommended experience saves the arrangement and unfinished definitions as work in progress.
It executes them only through an explicit action. Show a quiet **Saving / Saved / Not saved** state
near the workflow name; surface a conflict or uncertain response where it needs attention.
Coalesce completed gestures and editing pauses into bounded saves. Never equate optimistic local
state with a confirmed save. After a lost Run reply, show **Checking whether work started** and
automatically reconcile the exact submitted request with bounded lookup/replay. Do not ask the
person to select a receipt operation or retry it as a new run. Unresolved external effects retain
their distinct uncertainty; receipt recovery does not prove those effects completed safely.

The baseline Run covers the pending plan shown on this canvas. If it is incomplete, identify the
unfinished work; do not quietly omit it and launch the remainder. The debate challenges this
baseline when an unrelated blank idea blocks a useful chain. Compare an explicit **Choose tasks
to run** action that prepares a visible scope on the same canvas, leaving other sketches intact.
Ordinary inspection selection never changes that scope. Necessary new predecessors must appear
in the reviewed set; accepted predecessors are existing work, and their unapplied edits require
an explicit decision. Names and changes accompany the count; canvas highlighting alone is
insufficient. This is a proposed comparison, not an adopted selective-execution feature. The
[debate's queued-predecessor case](canvas-authoring-debate.md#does-one-unfinished-thought-have-to-block-useful-work)
records why that extra interaction could help or become another administrative burden.

Accepted work already in progress continues under its existing rules while the user drafts future
changes. When the pending plan includes edits to accepted work, the primary action becomes
**Apply changes**, with a reviewable count and description of the new tasks and changes it includes.
A queued-task edit becomes effective only through that action. Started tasks retain their
definitions and inputs; correction creates future work.
There is no second submit button inside the inspector.

When no execution remains but saved authoring does, distinguish **Nothing running · 1 unfinished
task** from completed cards, or use **Draft** before anything has run. Compose the admitted-work
status with saved draft presence; the daemon already permits additions after completed work.
The UI cannot infer that an empty scheduler queue means the person's objective is finished,
but this distinction does not itself require a new completion lifecycle.

Deletion before execution removes a draft item and its associated draft wires, with a local undo
opportunity. Removing or stopping accepted work is a different action and must explain its actual
effect. Existing uncertain outcomes remain uncertain until evidence or authorized reconciliation
resolves them. A simpler canvas must not turn that state into a misleading Retry button.

Switching workflows preserves each accessible draft and camera. Switching to another daemon or
actor must never replay a pending request against the new identity. If access is revoked, dispose
of inaccessible cached content under the existing isolation rules and explain why it disappeared.

## What changes in the daemon, and why

The tree, right inspector, placement without editing, contextual controls and direct port gestures
can be built over much of the existing backend. Project-filtered discovery and labelled capability
contracts already exist. Rearranging the screen alone does not require a new hierarchy or scheduler.

The recommended extension is **bounded saved authoring under the existing workflow owner**.
Today creation admits at least one complete task; a blank canvas exists only in browser memory.
Saving a blank task or empty workflow therefore needs a distinct authoring state. Do not weaken
`TaskDefinition`, add dummy instructions or abuse a missing required input to keep a fake task idle.
Those shortcuts confuse unfinished ideas with admitted work and consume cumulative task admission.
An empty authoring document must remain discoverable without implying that the person has finished
it. Project its saved content alongside admitted-work status; blank cards need not become unmet
runtime obligations or require a new Closed/Idle state machine.

| Boundary | Proposed responsibility |
| --- | --- |
| Existing workflow owner | Own the empty workflow and incomplete cards, sources and intended connections. Allocate stable card identities and retain bounded saved content under workflow authorization. |
| Shared Rust control path | Resolve typed connection choices against real capability metadata, generate extra-input keys, construct required dependencies and validate media and output limits. The browser presents this meaning. |
| Existing admission/runtime path | On explicit Run or Apply, validate the complete candidate against current authority and versions, then produce ordinary immutable task definitions or permitted prospective changes. Saved authoring cannot enter the scheduler. |
| Existing persistence owner | Atomically record admission, its exact command result and the mapping from draft cards to accepted work. After admission, retain references and unsubmitted candidates rather than a second authoritative accepted definition. |
| Presentation metadata | Retain bounded card positions separately from execution semantics. Camera and selection can remain client preferences; moving a card must not invalidate a task start or model proposal. |

This must remain usable through the shared daemon path by authorized humans, agents and services.
Complete create/edit requests can remain a convenience for callers that already have a fully
defined plan, reusing the same admission owner. No browser-only workflow authority is needed.

Several constraints affect the design rather than merely its implementation:

- **Authority remains explicit.** Current creation requires both create and start authority and
  captures execution authority. The smallest extension can preserve those requirements for an
  empty workflow while preventing drafts from executing. Admission still checks current authority.
  Saving without start permission would need a separately reasoned authority change; it is not
  promised here. Another client sees saved work only through the applicable workflow permissions.
- **Saving has a capacity.** Existing command receipts are finite and preserve exact replay.
  Saving each keystroke indefinitely would exhaust them. Specify bounded authoring bytes, cards,
  connections and retained revisions/receipts, coalesce saves, and explain refusal while retaining
  the last confirmed content and operational headroom. Do not evict exact replay evidence to make
  autosave appear unlimited. Concurrent writers need version guards, not silent last-writer wins.
  Coalescing reduces writes but does not solve their lifetime: the authoring save/receipt lifecycle
  remains an explicit design obligation before reliable autosave can be promised.
- **A wire cannot hide a policy change.** Connecting another output must not silently shrink an
  existing output allowance. Let Rust return a compatible plan or explain the material choice.
  Stale project or capability metadata keeps the drawing while the system rechecks it. Explain
  material changes at the affected task. Matching names or schemas do not establish that a changed
  executor is equivalent or authorize silently switching generations.
- **Run is atomic within its supported bound.** The existing batch limit is finite. If a plan is
  too large, explain the limit and let the person explicitly change the plan; never automatically split
  it into partially executed batches. The explicit-scope comparison above does not authorize a
  hidden ready-subset fallback or automatic dependency expansion beyond the reviewed work.
- **Existing work survives adoption.** Project accepted workflows from their actual definitions,
  bindings and results. Preserve historical definitions, frozen starts, receipts and uncertain
  effects. Treat saved authoring as an additive, explicitly versioned change where supported;
  never reconstruct history from positions or rewrite it to match the new drawing.

Any adopted implementation needs to cover this boundary together: Rust contracts, shared control,
runtime distinction, storage and recovery, generated clients and CLI access, read projections,
Svelte, tests and canonical documentation. This discussion does not choose a new parallel graph
store, collaboration service or general schema editor.

## Evidence and the strongest smaller alternative

Source inspection used commit `a19b5f0c`. The current canvas already moves cards, draws SVG wires
and retains a selected workflow on screen. The problem is the unfinished abstraction around those
operations, not an absence of canvas rendering:

- [`addCanvasTask`](../../../../../../apps/web/src/lib/canvas.ts) opens the inspector as it
  creates the task; [`CanvasWorkspace`](../../../../../../apps/web/src/components/CanvasWorkspace.svelte)
  then focuses instructions. Its connection handler opens source/destination/input controls after
  a gesture and constructs output declarations, dividing the output allowance among results.
- [`WorkflowCanvas`](../../../../../../apps/web/src/components/WorkflowCanvas.svelte) uses a
  generic outgoing handle and a destination card, rather than distinct destination input ports.
  The [connected-task browser test](../../../../../../apps/web/tests/browser/connected-task.spec.ts)
  expects a destination selector and another Connect action after dragging. Those passes establish
  current behavior, not that the interaction meets the user's need.
- [`App.svelte`](../../../../../../apps/web/src/App.svelte) separates daemon selection, project
  selection and recent workflows. Its project-selection operation clears the selected workflow;
  it cannot also be the action for expanding a project branch. Independent expanded branches need
  independently scoped, bounded discovery cursors.
- [Runtime admission](../../../../../../crates/runtime/src/tasks/admission.rs) rejects an empty
  initial batch. [Task validation](../../../../../../crates/blueprint/src/task.rs) requires a
  real definition. [Authoring metadata](../../../../../../crates/capability-host/src/authoring.rs)
  and [project-filtered reads](../../../../../../apps/daemon/src/host/tasks/reads.rs) already supply
  useful contracts; [storage](../../../../../../adapters/redb-store/src/task.rs) protects finite
  exact receipts. Reuse these owners rather than inventing replacements.

The strongest smaller alternative is to implement the interaction with memory drafts, retaining
the current atomic task submission. It could deliver the tree, blank placement, right inspector
and direct connections without introducing saved incomplete workflow state. That is a credible
choice if people sketch and run in one sitting, accept losing sketches on reload/disconnect, and
do not need to recover them from another authorized client. It also avoids new store capacity and
concurrency obligations. Merely moving the panel or hiding more fields would leave the repeated
connection form and fragmented navigation, so it does not meet the stated direction.

I favor saved authoring because arranging a workflow is work worth preserving before execution.
That is an inference from the proposed experience, not a reported loss incident. A prototype that
shows memory drafts are sufficient could justify narrowing the daemon change. Conversely, one
ordinary reload losing a carefully arranged unfinished workflow would expose the smaller option's
cost directly. Browser storage alone would not provide daemon-owned recovery across clients.

## Questions for debate and evidence that would change the design

1. **Should an unfinished workflow survive leaving the browser?** The recommendation is yes,
   through the daemon. If this is unnecessary for the intended workflow, prefer the smaller
   interaction change and describe the unsaved state honestly.
2. **Can a person infer what a wire passes from its visible endpoints?** Compare visible inherited
   model defaults, model-unknown sketch connections and definition before wiring, using a simple
   pair and a multi-output producer. Prefer the order that preserves thought with fewer unexpected
   repairs; retain blank placement in every version.
3. **Does the tree and inspector leave enough room to think?** A tree can become another dense
   control panel. Test several daemons, duplicate names and a larger workflow. Collapse irrelevant
   branches and reveal ports contextually; if that remains distracting, narrow the presentation
   without restoring competing navigation systems.
4. **Does an unfinished idea unnecessarily block useful work?** Compare whole-plan Run with the
   explicit-scope candidate, including a queued predecessor with an unapplied edit and a start race.
   The person must understand the work being started without learning batch mechanics or losing
   their sketch; do not hide partial admission behind Run.

Proposed acceptance evidence is a hands-on journey: without a tutorial, the requesting user finds
a workflow in the tree, places two empty tasks without an editor opening, defines them in the right
inspector, connects a useful result and runs them without losing the canvas. Also try wiring the
blank cards before definition to test whether the tentative state helps or confuses. Observe
hesitation and unexpected navigation, then repeat with keyboard/touch access and a multi-output task. Automated
gesture tests can verify the transitions; they cannot settle that usability judgment.

If saved authoring is selected, additionally prove empty-workflow and blank-card recovery through
another authorized client and restart; no invocation or spent task admission before Run; exact
recovery after a lost admission reply; stale contracts, denied access, invalid wires, version races
and full capacity retaining saved work; and existing accepted workflows loading without history
changes. These are proposed checks, not experiments performed for this discussion.

## Dated contributions

**2026-10-23 — Willow-20261023-a (agent pseudonym; coordinating task).** Developed the proposed
interaction and daemon boundary from the user's direction and current source. The principal choice
is preserving unfinished authoring; its added storage and authority consequences are reasons to
debate it, not grounds to claim the user has selected it. Repository documentation checks validate
this document's integration, not the proposed experience.

**2026-10-23 — Birch-20261023-a (agent pseudonym; delegated interaction review).** Read the
coordinator's summary of the user's direction and existing source before the design synthesis.
Identified placement opening the inspector and the post-drag connection form as flow problems.
Tree expansion must be separate from the existing project-selection operation, which clears the
canvas. Warned that a tree plus every visible port can be as dense as the current screen, and that
model choice and failure behavior must remain deliberate. Recommended requiring model selection
before wiring real ports initially: tentative wires introduce another state a user must understand.
The coordinating proposal retains that option in the second debate question, with user evidence
needed to justify connections between undefined placeholders. This was source and document review,
not a live usability session.

**2026-10-23 — Alder-20261023-a (agent pseudonym; delegated daemon review).** Inspected the shared
protocol/control/runtime/store path and its consumers. Recommended bounded authoring under the
existing workflow owner while preserving strict admission, exact receipts and one scheduler.
Qualified the proposal: preserving current create/start permission requirements is smaller than
promising a new author-only role, and save-per-keystroke receipts are not an unlimited resource.
Both reviewers subsequently read this proposal; their source and document reviews are not
independent user acceptance.

**2026-10-23 — Willow-20261023-a (coordinating agent pseudonym; UX debate synthesis).** Revised
the proposal after [Juniper, Rowan and Linden's exchange](canvas-authoring-debate.md): make repeated
creation discoverable, propose a visible usual model/agent, absorb exact-request recovery, and
reopen whole-canvas completion as a prerequisite for Run. The linked debate preserves changed
positions and unresolved objections. Its contributions are design reasoning, not user acceptance.
