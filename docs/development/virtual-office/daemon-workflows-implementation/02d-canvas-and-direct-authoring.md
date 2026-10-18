# 02d — A workflow canvas with direct task and input authoring

**Prepared, not assigned.** The user rejected the implemented 02c interaction as still complicated
and bloated. They want the workspace to be a canvas beside navigation, task details on the left,
and inputs connected to tasks directly. They specifically reject authoring through binding names,
input policies, supplied-input names, task identities/batch aliases, output names and project-path
forms. [The handoff](handoff.md#actual-user-review-and-corrections) records their feedback and the
source findings. This is a replacement of that interaction model, not another disclosure over it.

Read [AGENTS](../../../../AGENTS.md), the canonical product docs, [README](README.md),
[ADR 0051's canvas decision](../../../decisions/0051-daemon-task-workflows.md#browser-workspace-interaction),
[journey](journey.md), [implementation](../../practices/implementation.md),
[documentation](../../practices/documentation.md), [cohesive review](../../practices/cohesive-review.md)
and [workflow](../../workflow.md). Inspect source, consumers, tests, manifests and Git before editing.
Preserve 02a's shared names, 02b's supplied logo/visual direction, 03's execution behavior, existing
stores and tested checkpoints. Stop at this handoff before 04. Prompt 07 retains the full gate.

## Replace the page and form model

Use one canvas for the selected workflow, with workflow switching and discovery in navigation.
Tasks, text/file sources and their connections occupy the main area. A compact, closable selection
panel sits on the left, inside the workspace beside navigation. Selecting another item changes that
panel without replacing the canvas, moving its camera or opening a task page. Keep results readable
there, with bounded previews and explicit full-content access; do not dump the complete definition,
provenance and raw provider response into the default view.

Task cards show a useful title, real state and a short result or waiting/failure reason. Connections
show which information moves to which task. Pan, zoom, fit and selection remain usable as work grows;
live updates must not repeatedly rearrange the graph under the person. Layout is presentation, never
execution order or elapsed time. Keep actual timing available through compact details and a secondary
timeline/history presentation. The accessible list exposes the same operations from the same state.

**New workflow** opens an explicitly unsaved canvas. **Add task** adds a draft card and focuses its
instructions in the same selection panel. Instructions are the only required writing field; suggest
an editable title and show the executor, retaining 02c's deliberate-selection rules. A person can
submit useful instructions without drawing any connection. Add text or a captured file from the
canvas or the task's **Add input** action. Dragging a connector, dropping a supported file onto a
task, and keyboard **Connect to…** all select the same source and destination. Ask a small contextual
choice only when multiple genuine destination inputs or outputs are available.

Generate request identities, aliases and mechanical binding keys. Show human labels from the actual
file, text or task; disambiguate duplicate titles without requiring typed IDs. The user must not
have to create a named-input object, declare an output by spelling its key, separately enter the
dependency implied by a result connection, or stage a JSON command to perform these actions.

## Complete the daemon support that makes connectors truthful

This is not a frontend-only assignment. Current `TaskCapability` exposes only identity/operation;
process profiles can require exact named inputs. `TaskResultDto.artifacts` loses published output
names, workflow reads omit supplied values, and the session has no permitted project-root choices.
The client cannot safely manufacture these facts from capability display names or fixture constants.

Extend the existing adapter/profile, capability-host, control, protocol and daemon read owners with
small typed, bounded authoring projections. Derive input slots and declared output choices from the
configured capability contract, with its generation and content bounds. Models can accept additional
named context; process slots must match their configured inputs. Project only the information needed
to choose a source/output, not argv, environment, secrets or private configuration. Missing/changed
metadata gets a useful in-place explanation with the draft retained. Metadata neither grants
authority nor supplies another validator or scheduler.

Project actual named published outputs from the retained task result, with readable labels, media,
bounds and availability. Use a genuine default output only where its owner defines one; never invent
a universal `result` port, require an optional model output to exist, or wire the concatenated display
preview as if it were an exact artifact. For a draft producer, **Use result** selects a real
owner-defined output and adds its matching declaration before submission, exposing that connector
immediately. Human labels do not change its mechanical key. The current model adapter publishes
`model_response`, conditional `final_text` and `structured_output`; an arbitrary task output name
does not make it publish that artifact. Preserve conditional absence explicitly when choosing a
port. Future connections use these actual declared producer ports.
For existing tasks that have published a named result without declaring it, complete the selected
narrow `TaskOutput` validation extension in the ADR: a new future consumer may select that actual
retained output without revising the producer's definition. Shared admission and runtime validation
must establish the source, permissions and outcome rule; arbitrary artifact IDs are not an alternate
input path. Keep correction rebinding working through the same task-output representation. For a
previously undeclared published source, apply the ADR's finite compatibility rule at both initial
binding and correction redirect; do not let its empty declaration map bypass replacement checks.

Provide bounded authorized reads for queued supplied inputs and their provenance so a fresh client
can rebuild source cards. Inline inputs derive from their owning definitions; shared supplied inputs
remain one daemon-owned source. Show all affected future consumers before changing a shared source.
Started tasks continue to display their frozen input versions. Missing, inaccessible, expired and
off-page sources are distinct from empty text and must not be silently replaced or dropped.

Captured client files require no daemon filesystem grant. When a task explicitly enables file tools,
offer permitted project roots and access modes from authenticated daemon metadata. Root choices
come from the existing configured scope intersected with current authority; an optional relative
subfolder is a meaningful project choice. Rust validates the resolved claim and existing native path
protections. Do not expose a browser folder picker as daemon access, infer writes from a drag, leak
unavailable roots, or add a general file explorer/host administration product. Tools, writes and model
continuation remain explicit choices. Ordinary context/result connections need none of them.

Update Rust DTO owners and generated TypeScript together, with their actual CLI/public consumers,
fixtures, reference and compatibility handling. Existing task records, accepted history and request
receipts must remain readable and exactly recoverable. Do not introduce an independent graph store,
new workflow runtime, inference, provider family or backend-for-frontend.

## Make connections understandable and submission deliberate

Connecting text/file context binds it only to the selected task unless the person deliberately uses
a shared source. A result connection means **Use this result when it succeeds** by default and adds
the corresponding dependency. Removing a wire retains a prerequisite still needed by another wire
or an explicit order-only relationship. **Run after…** selects a task for ordering without importing its result.
For a deliberate exception, the connection panel explains **Use the outcome even if it fails**;
keep optional missing output distinct from a settled failure report. These choices retain the
daemon's existing readiness semantics. A missing required document appears as an empty input source
with **Supply input**, not a form asking for a supplied-input key.

New nodes and wires edit bounded in-memory drafts until explicit submission. Accepted ready tasks
can start immediately: never submit a task on card creation and attach its required inputs afterward.
One task needs one **Start workflow** or **Add task** action. Several draft tasks/connections can be
submitted as one existing atomic create/change request with generated aliases; related revisions use
the same guarded change path. Respect the current 1–32-change transaction bound; refuse overflow
with the drawing intact instead of silently splitting acceptance. **Apply changes** shows the affected tasks and connections in plain
language. There is no separate staging wizard or mandatory JSON preview. A new empty canvas creates
no durable workflow until its first task is accepted.

Unaccepted changes are visually distinct from daemon facts. Cancelling a draft connection has no
execution effect. Changing an accepted connection is a prospective edit, with explicit application;
deleting a line must not silently edit a started task or remove accepted work. Started definitions
remain fixed. **Create correction**, **Use correction for future work**, stop, remove unstarted work,
wait/resume, completion and limits remain available through relevant item/workflow actions, with
their original authority and consequences. A rejected model proposal has a contextual review/dismiss
action, retaining atomic repaired changes plus `dismiss_admission` where needed. Dismissal does not
execute rejected prose or clear a separate human wait. No silent resume, permission expansion or
limit increase.

Preserve bounded drafts across switching, camera/selection changes and Back/Forward, keyed to the
authenticated daemon/actor and exact workflow. Define count/byte bounds for draft nodes, edges,
input text and cached graph data, with explicit refusal and discard. Keep sensitive content and
credentials out of persistent browser storage and URLs. Layout may be reconstructed or kept as
bounded non-secret local presentation; shared layout persistence is outside this correction.
Disconnect, identity replacement and authority loss dispose of inaccessible state.

Freeze each pending batch's target, payload and request ID across navigation. Lost replies offer
receipt lookup/exact replay; conflicts retain the drawing for a fresh explicit decision. Handle task
starts, changed capability metadata and another client's edits during composition. A partially loaded
graph shows its boundary and offers bounded loading; unseen producers are not missing tasks, and
editing a visible edge must not drop other connections. Never auto-fetch all retained work.

## Remove the superseded authoring surfaces

Replace the page/composer arrangement in `App.svelte` and `WorkflowView.svelte` and the product paths
through `TaskComposer.svelte`, `TaskDefinitionEditor.svelte` and `FutureBatchEditor.svelte` as needed.
Keep reusable typed request/recovery logic, not competing form and canvas editors for the same job.
Remove raw binding/policy/dependency/output/path builders from task authoring, including their
advanced incarnation. Preserve their meaningful operations through source, connection and scoped
resource selection. Machine DTOs and the CLI remain available to programmatic clients; copyable
technical diagnostics may exist separately from the authoring flow.

Use the maintained Svelte/CSS owners and exact supplied logo. A focused graph component may be used
if it meets the interaction, licensing, maintenance and accessibility needs; do not build a second
framework. The earlier mockup remains a visual guide only. Its revision/run/controller controls do
not become requirements because the workspace now has a canvas.

Keyboard and touch users must complete the same work without precise dragging or hover. Provide
focusable connectors, source/destination selection, announced connection outcomes, readable
connection lists and Escape/focus return. On narrow screens keep a compact closable panel with a
visible route back to the same canvas/selection, not another long page. Verify enlarged text and
reduced motion. Opening the inspector must leave enough canvas to navigate between tasks.

## Prove the replacement, then hand it back

Run frontend check, lint, unit, build and the complete production browser suite against the actual
Rust fixture. Run affected Rust owner/refusal/recovery suites, DTO generation and public API checks
for changed contracts, scoped Clippy and the actual CLI/daemon journey. Backend fixture logic remains
Rust. Port existing assertions to the new UI without discarding their runtime, authority or recovery
meaning. Prompt 07 remains the integrated gate; known in-scope failures cannot be deferred there.

The production browser must establish:

- Create two workflows; switch canvases; add to the first without affecting the second. Task selection
  keeps the graph visible and changes only the compact left panel. Instructions alone work, and no
  protocol fields are required when text, a file or a result is connected.
- Connect a real captured Cargo manifest to its actual configured process input, run it, then connect
  a genuine named result to a model task. Reconstruct those sources/edges after reconnect, including
  a result from a pre-correction task without a declaration and two tasks with duplicate titles.
- Assemble three research tasks and a composition task before submitting once. Verify actual parallel
  execution, default failure blocking, a deliberate failed-outcome input, queued-input repair and
  correction/rebinding while original failures and frozen inputs remain unchanged. Include a failed
  historical producer with an undeclared but published output: a compatible correction can redirect
  its future consumer, while wrong media or oversized replacement refuses atomically. Review/dismiss
  a rejected model proposal without accidentally resuming an independently waiting workflow.
- Exercise a start racing an edit, stale generation, cycle refusal, changed executor metadata, denied
  source/project access, off-page edges, draft/graph capacity and lost accepted replies across daemon
  switching. No partial accepted graph or hidden permission/scheduling changes may result.
- Complete creation and connection by keyboard and touch-sized controls, retaining selection, camera,
  drafts and focus at desktop/narrow widths. Inspect real renders; no canvas/inspector overflow that
  turns selection back into a full-page form.

Invite the user to create useful work, connect an input/result and move between tasks without a
protocol tutorial. Record their actual actions and objections in the single handoff. Automated
passes do not establish acceptance of this interaction. Update canonical docs, operator/frontend
guidance, sprint handoff and evidence; run documentation contracts/diff checks and make coherent
tested local commits. Leave actual review open if unavailable. Stop before 04.
