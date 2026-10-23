# S03 — Actual early-user review and complete repair

**Prepared, not assigned.** Prerequisite: [S02](S02-saved-canvas-authoring.md)'s usable production
experience and focused checks. This assignment owns the outstanding actual-user checkpoint from
[main 02](../daemon-workflows-implementation/02-early-user-review.md). Its completion requires the
user's participation and a useful input-dependent outcome; screenshots or agent checks cannot supply
that evidence. S04 methods waits for this checkpoint. No automatic successor is authorized.

Read [AGENTS](../../../../AGENTS.md), canonical docs, [README](README.md), the side-sprint handoff,
[ADR 0052](../../../decisions/0052-canvas-authoring-and-method-experience.md), the main sprint's
[journey](../daemon-workflows-implementation/journey.md) and
[recorded review](../daemon-workflows-implementation/handoff.md#actual-user-review-and-corrections).
Use [cohesive review](../../practices/cohesive-review.md),
[implementation](../../practices/implementation.md), [documentation](../../practices/documentation.md)
and [workflow](../../workflow.md). Inspect the current source, startup instructions and Git context;
review the actual S01/S02 result, not an older checkpoint or a mock of the selected interaction.

## Prepare real work the user can inspect

Run the built daemon and Svelte client in an isolated authorized development root. Preserve existing
user stores, project files and services. Prepare usable URLs, existing authorized credentials and a
short brief plus input file. Choose an already configured model/agent or trusted Rust tool that
produces useful output from that input. Do not invent credentials, spending authority or containment.
A controlled endpoint remains appropriate for fault tests, but cannot provide the sole substantive
content of this product checkpoint. Physical phone evidence requires actual physical phone use.

Use the animation-research brief or a concrete useful equivalent the user recognizes. Keep instructions
separate from the supplied brief so the review exercises actual input flow. Provide an independently
recognizable second project/workflow and a second authorized daemon for the existing owner-isolation
check. Prepare these through normal operations; the person must not need IDs, request files, database
editing or fixture-only helpers. Explain the intended task, without coaching them through a protocol.

At least one real result must depend on the supplied content. Retain the brief, selected capability,
actual output and relevant project-file effects sufficiently to assess that relationship. A model's
plausible-looking reply alone does not prove the connection; a prewritten fixture reply does not
establish useful product work. Qualify what was inspected rather than claim general model quality.

## Observe one continuous journey

Have the user perform these connected actions, adapting the brief without dropping their purpose:

1. Connect to the intended daemon and identify its project. Expand other branches without losing the
   current workflow; create two workflows and find them again in the same tree.
2. Place empty tasks and arrange a research/comparison idea before defining it. Confirm placement
   opens no inspector; choose a task, write instructions and connect supplied text or a real file.
3. Choose a usual model deliberately, apply it once to the unassigned draft tasks through the
   explicit scoped action, and explain which existing and new cards use it. Connect a real result
   into another task; resolve meaningful port choices without entering keys, identifiers or schemas.
4. Leave an unrelated empty idea on the canvas, choose the intended work to run and identify the
   tasks/changes included. Inspecting another card must not unexpectedly change execution scope.
5. Inspect useful output and, when tools write files, locate the actual project files. Move between
   tasks while preserving the canvas, relevant context and readable result details.
6. Leave unfinished authoring, close the client while admitted work continues, then return in a fresh
   session. Find the draft, accepted work and earlier result without manually reconstructing state.
7. Switch daemon/project and identify the destination before another action. Resolve a missing input,
   inspect an actual failure and use Pause/Continue and Stop with their truthful effects explained.
8. Improve a successful-but-inadequate result through future work without rewriting the old execution.
   Retain an unfinished thought while settled work shows its outcome; do not infer objective quality
   from a green status. Exercise ordinary authorized follow-ups without repeated manual launch.

Ask the user to explain what is saved, what will run, which model and project are used, and where
the next action belongs. Observe hesitation, unnecessary repeated decisions and lost context as
findings, not merely visual preferences. Inspect desktop and narrow layouts and accessible controls;
qualify the actual devices/input methods used. A narrow browser render is not a physical-phone claim.

The difficult variation includes a saved candidate edit while its predecessor starts, a deliberately
paused workflow, and a controlled lost submission reply. Verify the real behavior with focused tests;
where practical, let the user observe the recovery and explain it. Do not manufacture an uncertain
external effect or require the person to operate receipt lookup/replay. S01's pre-entry protections
must hold under the reviewed UI, including restart; independent entered work remains honestly reported.

## Repair the complete responsibility

Trace findings through `apps/web/src/App.svelte`, `components/CanvasWorkspace.svelte`,
`components/WorkflowCanvas.svelte` and `lib/{canvas,transport}.ts` under `apps/web/src`, then their
actual public callers: `apps/daemon/src/http/tasks.rs`, `crates/control-protocol/src/tasks.rs` and
`crates/control-client/src/tasks.rs`. Admission/readiness belong to `crates/control/src/tasks.rs`
and `crates/runtime/src/tasks`; durable authoring belongs to persistence/redb owners. Inspect the
affected implementation rather than assigning every usability finding to CSS or a future phase.

Fix broken discovery, input meaning, misleading execution state, lost drafts, unrelated edit conflicts
or confusing control at its owner. Retain useful work, accepted history and exact recovery. Concrete
findings may revise an engineering default with its reason and affected docs/tests; do not silently
add adaptive methods, branch decisions or another runtime. If the central interaction fails, reopen
that decision explicitly and repair it before marking this checkpoint complete.

For each executable repair, compile affected callers and run discovered behavior/refusal/recovery
regressions. Use the real Rust daemon fixture for browser checks; frontend scripts are
`npm run check`, `npm run lint`, `npm run test:unit`, `npm run build` and `npm run test:browser`.
Inspect repaired renders and rerun affected user actions with the person when understanding or flow
was the finding. Run scoped Rust/API checks where contracts change, documentation contracts and
diff checks. [S05](S05-final-review-and-gate.md) owns the side sprint's full gate; known defects do
not wait for it. Coherent checked commits belong to the assigned implementation, not this preparation.

## Record the actual outcome and stop

Record reviewer participation, environment/source identity, brief, capability, observed actions,
results, concrete findings, repairs and limitations once in the existing main
[actual-user review record](../daemon-workflows-implementation/handoff.md#actual-user-review-and-corrections).
The side-sprint handoff records completion or the remaining gate and links to that record; main 02
uses the same outcome. Keep verified checks and their limits in canonical verification evidence,
and bulky redacted captures under ignored evidence paths. Do not duplicate the review chronology.

If the user or a substantive configured capability is unavailable, complete independent preparation,
tests and repairs, then leave the gate pending with the exact missing observation. Availability is
not permission to substitute agent approval or canned results. Required unresolved functionality or
usability also keeps S03 open and S04 waiting. On actual completion, reconcile main 02's status and
stop ready for the separately assigned S04; this review does not finish the entire product.
