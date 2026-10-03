# 01 — Create and edit workflows

After 00 is ready, follow [the shared rules](README.md),
[implementation practice](../../practices/implementation.md), and
[documentation practice](../../practices/documentation.md). Read the current handoff and affected
code. Deliver authoring, not the full execution journey.

Start with `apps/cli/src/command/blueprint.rs`, model selection commands, `crates/blueprint`,
`crates/model`, and public workflow/capability operations. Preserve the separate process-sequence
feature; do not turn it into a second general workflow system.

## Build this

A user can name a workflow, add model steps, choose an available model, enter or change prompts,
connect one step's result to another, name the input supplied at run time, and choose the final
output. They can inspect, remove, or reorder steps where valid, then save and reopen the workflow.
Accept prompt text from a file or standard input. Keep a scriptable command route; a wizard is
not required. Ordinary use must not need hand-written graph JSON or calculated hashes.

Use the existing definition and model types. Add or complete public daemon operations where
necessary, rather than putting indispensable construction rules in the CLI. Clients may retain
unfinished edits, but only validated submitted definitions may run. Model selection uses the
caller's permitted catalogue; it must not expose hidden profiles or silently substitute a model.
Keep credentials and permission setup out of workflow files.

Connect the release-notes brief and first result explicitly. Do not copy all previous conversation
or unrelated run data into the second prompt. Use the existing model input and result checks,
including a visible failure for an empty or incomplete required response. 02 supplies actual
per-run values; do not bake each brief into a new definition here.

Two workflows must coexist. Reopening preserves the prompts, steps, connections, and selected
version. Editing a saved definition produces a new version without changing the old one or any
running work. Guard against overwriting existing files and stale edits from another session.
If an imported workflow has features this editor cannot handle, preserve them or refuse the edit
before writing. Never quietly turn a richer workflow into a simple chain.

## Check just this work

Compile changed packages and their affected callers. Test the real authoring commands and public
operations: create two workflows; edit/save/reopen; connect valid inputs; reject invalid connections,
unauthorized model selection, stale edits, and unsupported edits. Check interrupted writes and
unchanged round trips where persistence changes. Confirm authoring starts no model request or run.
Run affected example-reader and CLI parsing checks. Keep tests focused; the full journey belongs to 06.

## Commit points

Commit the shared authoring/API change and required callers once they compile and pass focused tests.
Commit usable step/model/prompt commands as the next working change. Commit safe save/reopen and its
regressions as another checkpoint. Combine or divide these by real code dependencies, not file count.
Update the existing guide with tested commands alongside the relevant change.

Stop when ordinary authoring works without a private test builder or raw graph assembly.
Record the usable commands, commits, focused checks, and 02's starting point in `handoffs/01.md`.
Do not run the full workspace gate.
