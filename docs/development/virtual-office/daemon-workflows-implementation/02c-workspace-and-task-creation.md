# 02c — Make workflows and ordinary task creation understandable

**Assigned; implemented locally, actual user recheck pending.** The user reviewed the 02b delivery and rejected its interaction design:
too many fields/forms, disruptive navigation, and no discoverable way to create multiple workflows.
The [handoff](handoff.md#actual-user-review-and-corrections) records the actual feedback and source
findings. Preserve 02a's naming behavior and 02b's supplied brand and visual direction. Complete this
correction and the remaining actual 02 review before a separately assigned 04; do not replay 03.

Read [AGENTS](../../../../AGENTS.md), the canonical product docs, [README](README.md),
[ADR 0051](../../../decisions/0051-daemon-task-workflows.md#browser-workspace-interaction),
[journey](journey.md), [implementation](../../practices/implementation.md),
[documentation](../../practices/documentation.md), [cohesive review](../../practices/cohesive-review.md)
and [workflow](../../workflow.md). Inspect current source, callers, tests and Git before editing.

## Deliver one understandable workspace

A person can create two separate workflows, switch between them, and add another task to either
without leaving its workspace or understanding the batch protocol. They can tell which daemon and
workflow will receive the work before submitting. Existing tasks, results and current location stay
visible while they compose. This is the acceptance case; recoloring or renaming the current forms
alone is incomplete.

Before this correction, `App.svelte` exposed **+ New task** only on workflow discovery. `TaskComposer.svelte` sent
`POST /v1/workflows`, creating a workflow with one initial task. Adding to an existing workflow went
through `WorkflowView.svelte` and `FutureBatchEditor.svelte`: alias, full definition, staging, JSON
preview and submission. The backend already supports multiple workflows and ordinary guarded task
additions. Runtime admission requires 1–32 initial tasks; an empty durable workflow is not supported.

Use these two distinct actions:

| Action | Visible experience | Public operation |
| --- | --- | --- |
| **New workflow** | Open a compact first-task composer in the workspace. Show that its destination is a new workflow on the selected daemon. | Create the workflow and first task together through the existing create request. Until accepted, this is an explicitly unsaved draft; cancellation creates no workflow. |
| **Add task** | Open the same compact composer beside or within the selected workflow. Keep its timeline/list and location available. | Add to that exact workflow through the existing generation-guarded change request. It must not create another workflow. |

Keep bounded workflow navigation and **New workflow** available while inspecting or composing work.
Use a compact daemon switcher, the current/recent workflows and **All workflows** in the desktop
rail, with access to settings. Bound that list, keep the current selection visible when outside it,
and leave detailed search/pagination in discovery rather than duplicating that whole interface in
the rail. Do not pretend recent items are the complete collection. On narrow screens use
labeled navigation and a composer that returns to the same selection. Avoid a second required
discovery page, a creation wizard or a stack of modal forms. Browser Back/Forward should restore the
visible daemon/workflow location without submitting or replaying a mutation, or confusing another
daemon's state. Authenticated reads may refresh the restored location.

## Reduce ordinary authoring to the work itself

The compact composer has one required writing field: **What should this task do?** Keep instructions
separate from optional supplied context. Offer **Add context** and attachments when needed; do not
require a second brief, description, alias or technical identifier for every task. A configured
capability may need particular inputs; preserve those bindings and its truthful refusal without
pretending every capability can run from instructions alone. The current capability DTO exposes
identity and operation, not an input-form schema or preferred executor; do not invent either from
its display name.

Derive a visible, editable draft title from the instructions using a deterministic bounded rule.
Let the person override it before submitting without a separate naming page, and retain that manual
override when instructions change. Derive the initial workflow name from that draft unless
explicitly overridden. Never silently rename accepted work,
infer dependencies from prose or use a model call merely to produce a title. Enforce the existing
UTF-8 byte limits, including non-ASCII titles.

Show the selected executor compactly, with an accessible change action. A sole permitted capability
can be displayed as the selection; multiple capabilities need a deliberate choice or the person's
previous explicit choice for this authenticated daemon. Validate a reused choice against the current
session and show it before submission. Key the selection by `(id, operation)`, not catalog position,
so a refresh or reorder cannot silently change it. Do not choose the first catalog entry, enable model
continuation, add tools or grant filesystem writes to reduce clicks. Missing capabilities get a
clear explanation in place, with the draft intact.

Generate mechanical request IDs and batch aliases internally. Use daemon defaults for limits and
preserve the existing finite-work completion choice with a concise visible explanation and an
advanced override. Put dependencies, named input/output rules, paths, tool policies and limits in
secondary controls. Preserve those capabilities and the complete batch editor for deliberate complex
edits; ordinary single-task creation needs neither staging nor a raw JSON confirmation. Its visible
composer is the review before its single submit action.

Use the existing public request owners end to end. An ordinary addition with context/files must
bind only the inputs selected for that task. Do not overwrite a shared `brief` or `file` value used
by other queued work just because the initial-task composer used those names. Prefer task-local
inline/uploaded bindings where the contract permits them; explicit replacement of shared workflow
inputs remains a separate operation. Deliberate missing required inputs still create queued work,
with repair available through the inspector. No client schedules work or edits accepted history.

## Keep navigation, drafts and recovery coherent

Build one maintained composer presentation with explicit new-workflow/existing-workflow targets;
remove the obsolete full-page ordinary creation path. Share appropriate definition controls with
advanced editing without rendering every protocol field in the compact path. Keep styles in the
existing owners and preserve the supplied SVG. No new framework, theme engine, graph library or
second backend command path is needed.

Keep drafts bounded and keyed to the connection/authentication session, daemon identity and intended
target. Switching workflows, opening navigation or inspecting a result must not discard or retarget
a draft. Explain the chosen
draft capacity and provide an explicit discard action; refuse overflow rather than silently losing
work. Credentials and sensitive draft/input text remain out of URLs and persistent browser storage.
Disconnect, identity replacement and authority loss dispose of inaccessible state. Restore focus
when the composer closes, and retain keyboard, narrow-layout, enlarged-text and reduced-motion use.

A pending submission keeps its exact frozen target, payload and request identity through navigation.
Prevent duplicate clicks. A lost reply offers receipt/replay recovery without submitting changed
content under the same key. A generation conflict keeps the draft for a new explicit decision and
never silently retries against newer state. Refreshes must not steal focus, clear a draft or display
an old result under another workflow. Discovery must distinguish loading, empty, error and truncated
results. Adding work to a completed workflow remains possible where the public contract permits it;
adding work to a waiting/blocked workflow must not secretly resume it or enlarge its allowances.

## Verify the user path, then hand it back

Run frontend check, lint, unit, build and the complete production browser suite against the actual
Rust daemon fixture. Keep existing assertions for advanced batches, inputs, failures, correction,
authority and exact recovery as the ordinary path changes. Test the new path at desktop and narrow
widths, including keyboard focus and navigation with unfinished and pending drafts. Any necessary
backend fix belongs in its Rust owner with its affected suites; changing APIs to bypass the existing
guards is not a simplification. Prompt 07 remains the full integrated gate.

Prove through the production UI:

- Create workflows A and B using **New workflow**. Both remain discoverable after reconnect.
- Add a task to A from A's workspace. A gains a task, B is unchanged and no third workflow appears.
- Once an executor is selected, ordinary work needs one required writing field, opening the composer
  and one submit action. Optional context stays separate; no alias, stage or JSON preview is required.
- Switch A → B → A while composing and while recovering a lost reply. The original draft/request
  retains its daemon and workflow target; another connection cannot display or submit it.
- Add tasks with distinct context to the same workflow without changing another queued task's
  inputs. Exercise files, intentional missing input, a completed workflow, an explicit wait,
  permission refusal and stale generation. Existing work/results and authority stay truthful.

Invite the user to create two workflows, add a second task to the first, switch and inspect results
without coaching through internal terminology. Record their actual actions and remaining objections
in the single handoff. Agent action counts and browser passes establish implementation behavior,
not user acceptance. If the user is unavailable, finish authorized repairs and leave review open.

Update frontend/operator explanations, status, evidence and this sprint's handoff. Run documentation
contracts and diff checks, and make coherent tested local commits. Stop before 04. This assignment
does not authorize later feature prompts.
