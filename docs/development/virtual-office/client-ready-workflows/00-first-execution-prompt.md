# 00 — Set up this sprint

Read [the sprint README](README.md), [AGENTS.md](../../../../AGENTS.md),
[implementation practice](../../practices/implementation.md),
[documentation practice](../../practices/documentation.md), and [work rules](../../workflow.md).
This prompt adopts the plan. It does not implement the later features or start a GUI.

## Establish the starting point

Record the checkout's commit, existing edits, and required toolchain. Preserve other contributors'
work. The package's reference commit is a starting reference, not an instruction to downgrade.
Check the current code before treating an earlier finding as unfinished work.

Trace the ordinary route: create/edit a workflow, choose a model, supply inputs, start it, read its
result, change future work, and reuse it. Start with `apps/cli`, `crates/control-client`,
`crates/control-protocol`, `apps/daemon/src/host/commands`, and the existing blueprint, control,
and runtime libraries. Read deeper only where that operation requires it.

The earlier plan found that ordinary `StartRun` could not carry per-run inputs, even though the
runtime could. Confirm whether that remains true. Do not introduce a second input mechanism if
it has already been completed.

Add one small table to the sprint README: operation, existing owner, needed change, and prompt.
Choose a simple way to save unsaved edits and submit a valid workflow. Unsaved edits are not
executable history. Decide which existing daemon operations need completing so a future Svelte
client will not need CLI-private rules. Do not create another draft database or workflow language.

## Update the instructions that would send agents the wrong way

Update the roadmap and active sprint entry to name this work. Reconcile conflicting instructions
in AGENTS or other maintained guidance rather than leaving two competing plans.

In `docs/development/workflow.md`, allow an explicitly assigned multi-phase sprint to use focused
checks during implementation and its named final prompt for the full gate. Normal work outside
that schedule keeps its existing requirements. Make clear that early handoffs are not full-system
acceptance. Reconcile linked guidance, including AGENTS.md's blanket full-gate requirement; do not weaken tests
or edit CI to skip them. Record the small-commit practice in the existing work guidance instead of creating a second rules file.

In the vision and architecture, state: **Svelte is the first GUI; every frontend is a thin client
of the daemon.** Iced may be reconsidered later. Correct contradictory current instructions, not
historical records of earlier decisions. Retain canvas, timeline, and inspector ideas where useful,
but add no GUI code or dependencies. Keep backend code and backend tooling in Rust; make sure the
language rules do not prohibit an explicitly authorized Svelte frontend later. Update status only
for behavior that exists.

## Prepare the shared example

Use the README's two-step release-notes workflow and two input briefs. Keep example names, responses,
and limits out of product code. Identify the existing complete-output check and a supported pause
or review point for the repair example. A terminal failed run must not be reopened.

Identify a usable, authorized model configuration for 06, or record that access still needs supplying.
Do not make model calls now, change permissions, or reconfigure the user's hosts to force access.
Record the intended tests and their owners, not expected successful results.

## Check and commit

Run checks for the changed documentation, links, examples where applicable, and `git diff --check`.
Do not run the full gate or full operator scenario.

Use separate commits for the testing/commit work rules and the adopted sprint/Svelte direction.
Keep closely related changes together where necessary. Leave `handoffs/00.md` with the starting
commit, adopted decisions, commit IDs, checks, and where 01 should begin.

Stop when the scope, ownership, testing schedule, and GUI direction are unambiguous. Do not leave
code stubs or a second design document for another agent to reconcile.
