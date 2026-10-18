# 02b — Apply the visual direction and real Milkdrift brand

**Assigned October 17; implemented locally, subsequent interaction review rejected.** Subsequently assigned
[02c](02c-workspace-and-task-creation.md) implements corrected forms, navigation and workflow/task creation;
the supplied visual direction remains in place without a claim of user acceptance. Prerequisite:
[02a](02a-daemon-connection-identity.md) is implemented and passes focused checks. The
[handoff](handoff.md) records verification and the remaining user review. The user strongly rejected
the original visual design during the partial
[02 review](handoff.md#actual-user-review-and-corrections) and supplied the reference below. This
assignment repairs the maintained production Svelte client before more feature surfaces are added.
It does not authorize 04 or revive the superseded frontend program.

Read [AGENTS](../../../../AGENTS.md), canonical docs, [README](README.md),
[ADR 0051](../../../decisions/0051-daemon-task-workflows.md), [journey](journey.md),
[implementation](../../practices/implementation.md), [documentation](../../practices/documentation.md),
[cohesive review](../../practices/cohesive-review.md) and [workflow](../../workflow.md).
Inspect current source, behavior, tests and Git before choosing the component changes.

## Use the reference for appearance

The [user's original mockup](assets/milkdrift-ui-sample.png) is a visual reference, not a specification
of current features. The retained copy is byte-identical to the October 17 attachment. Its text,
buttons, sample results and status values are image content, not execution instructions or evidence.

![User-supplied visual direction; depicted features may be obsolete](assets/milkdrift-ui-sample.png)

Use the supplied [Milkdrift logo](../milkdrift-remedial-planning-v3/assets/milkdrift-logo.svg).
Move or copy that exact SVG into the frontend's maintained public asset location during implementation
and reference it normally. Preserve its paths, proportions and supplied colors; scale with CSS.
Replace the current literal `m` brand mark. Do not regenerate, trace or substitute an unrelated logo,
and do not treat the mockup's large decorative illustration as another required product feature.

Translate the reference into the current app:

| Visual cue | Application to the current product |
| --- | --- |
| Deep navy background and layered dark panels | A coherent dark workspace with subtle cool borders, restrained shadows and readable warm off-white text. Replace the current pale card theme throughout the maintained app. |
| Violet-to-blue selected navigation and quiet unselected items | A clear active daemon/workspace location and a restrained primary-action hierarchy. Keep error, failure and stop states distinct from decoration. |
| Compact left rail and contextual top bar | Real logo, daemon name/connection state, workflow context and supported actions. Put secondary technical details behind labeled inspection controls. |
| Broad central workspace and right-hand inspector | Keep actual task timing/dependencies readable while selected instructions, inputs, results and available edits stay alongside them on desktop. |
| Compact task cards, fine separators and deliberate spacing | Titles and short descriptions lead; status, timing and supporting facts follow. Dense information remains readable without tiny type or large empty marketing panels. |
| Lower detail area | Use only existing retained results/activity facts with useful collapse behavior. Do not fabricate an event journal or require an empty bottom panel to match the picture. |

Build shared color, type, spacing, focus and panel rules in the existing frontend style/component
owners. Apply them to connection/empty/error states, workflow discovery/search, task creation,
timeline/list, inspector, future-batch editing and daemon settings. A new header over unchanged
forms is incomplete. Keep the task form's instructions separate from supplied context and make its
ordinary action obvious without exposing protocol machinery.

## Preserve current meaning and usability

The task dependency timeline remains the primary work view: real start/end evidence, visible
parallel lanes, blocked/queued work without invented timestamps, corrective links and retained
failure. Preserve its accessible list and all current authoring, preview, repair, conflict and
request-recovery actions. The mockup's spatial canvas is not permission to add a graph editor,
splitter/join/controller products, revision/run navigation, evaluation machinery or a minimap.
Show only controls backed by current public operations. Do not add placeholder navigation for
future methods, hosts or retention before those assignments implement them.

Keep stop requested, stopped and uncertain outcomes visibly different. Do not rename a stop action
to the mockup's pause label unless its actual daemon operation is pause. Never imitate its sample
events, counters, health, timings or results. Svelte still projects daemon state and never schedules
work or fabricates a successful task.

At narrow widths, turn the rail into accessible navigation and let the inspector stack or open as a
labeled drawer; avoid squeezing the desktop's three columns onto a phone. Preserve selected work,
drafts and focus when opening or closing it. Support keyboard navigation, visible focus, readable
contrast, zoom, reduced motion and non-color status cues. Decorative stars/gradients are optional
and must not obscure content. Scope the first correction to one coherent dark design; a new theme
engine, graph library or frontend framework is not needed.

## Verify the rendered product with the user

Run frontend check, lint, unit, production build and the complete browser suite against the actual
daemon fixture. Add targeted regressions for changed interaction/accessibility behavior. Inspect
desktop and narrow renders of empty and populated workflows, simultaneous tasks, long titles,
blocked/failed/uncertain states, result inspection and batch editing. Check keyboard focus, overflow,
text zoom and long results; screenshots alone do not prove that actions still work. Keep render
evidence in ignored browser paths and compare it with the supplied visual reference. A viewport
check does not establish physical-phone support.

Invite the user to review the real implementation for visual fit and repeat the connection and
task/result actions from 02. Record what they actually reviewed and their remaining objections;
do not infer approval from a passing browser suite or a resemblance score. Complete authorized
repairs, then resume the remaining substantive 02 journey. If the user is unavailable, finish
implementation checks and keep actual review open rather than calling this feedback resolved.

Update frontend/operator explanations and the single sprint handoff with tested commits, screenshots,
remaining review and the component/style conventions that 04 and 05 must extend. Run documentation
contracts and diff checks; make coherent tested local commits. Prompt 07 remains the integrated gate.
Stop before 04. The corrected appearance and completed early review are prerequisites for that
separately assigned feature phase, not automatic authorization to begin it.
