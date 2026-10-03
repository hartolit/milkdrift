# Make workflows usable from the CLI

## Goal

Create a workflow, choose a model, write prompts, connect steps, run it with an input, read the
result, and use it again. Ordinary use should not require writing graph JSON, calculating hashes,
or understanding the database.

Finish that path and fix the awkward or duplicated code it exposes. Do not rebuild the working
host, container, publication, or learning systems from the previous sprint.

These files replace the earlier prompts at the same paths. The source reference is
`a04b55c0e0a3b56a1074da8861ab1d581e16a05c`; 00 checks the actual checkout. Never reset to that reference.

## Svelte first; the daemon owns the behavior

**Svelte is the first GUI.** A desktop wrapper may use Tauri. Iced is only a possible later client.
This sprint builds no GUI and adds no frontend dependencies.

Svelte, the CLI, and future interfaces use the same public daemon operations. The daemon and its
Rust libraries decide permissions, valid workflow changes, execution, recovery, and saved results.
Clients do not duplicate those rules, access the database, or invoke the CLI to use the product.

An interface may keep unsaved edits, arrange a canvas, display progress, and help fill in forms.
Its checks can improve feedback, but the daemon still validates submitted work. No essential
operation may require CLI-private helpers or make Svelte reproduce Rust-only rules or hash calculations.

## Work in order

| Prompt | Work |
| --- | --- |
| [00 — Set up the sprint](00-first-execution-prompt.md) | Confirm the starting point, update work rules, and record the Svelte direction. |
| [01 — Create and edit workflows](01-workflow-authoring.md) | Add steps, choose models, write prompts, connect steps, save, and reopen. |
| [02 — Run with inputs and reconnect](02-inputs-execution-and-recovery.md) | Use different inputs and recover a lost connection without duplicate work. |
| [03 — Read results and fix future steps](03-inspection-and-prospective-repair.md) | Explain failures, retrieve results, and safely change work that has not happened yet. |
| [04 — Save and reuse workflows](04-reusable-methods.md) | Reuse/copy workflows and simplify existing publication and evaluation commands. |
| [05 — Keep the interface thin](05-client-boundary-and-contraction.md) | Remove duplicated rules and test another client without the CLI. |
| [06 — Test the full system and fix problems](06-integrated-acceptance-and-closeout.md) | Test the combined result, fix failures, and finish the sprint. |

Each phase finishes its implementation and focused checks. Its handoff is not a claim that the
whole system passed. Run sequentially; do not start a GUI automatically after 06.

## One example throughout

Use a two-step release-notes workflow: one model drafts notes from a change brief; another receives
the draft and original brief, then reviews and revises the notes. Use two different briefs through
the same saved version: a host release and a game's networking update. Keep example data outside
product code. Create a second independent workflow too.

Controlled model responses make failure tests repeatable. An empty or incomplete required result
must fail. Repair eligible future work through normal permissions; an ended run stays ended.
In 06, run both briefs through an authorized real local model as well. Report complete output
separately from editorial usefulness. Existing Slotbook and managed-host tests remain required;
this smaller example does not replace them.

## Shared working rules

Follow [AGENTS.md](../../../../AGENTS.md), [implementation practice](../../practices/implementation.md),
[documentation practice](../../practices/documentation.md), [work rules](../../workflow.md), and
[office procedure](../README.md). 00 resolves conflicts with the testing schedule below.

Implement each change where it belongs, including callers, tests, and documentation. Fix an
underlying problem when discovered, not in 06. Remove the replaced implementation. Do not add a
wrapper around a broken rule, leave stubs, or make unrelated architectural changes. Preserve
working direct calls, permissions, input isolation, resource ownership, and recovery.

Follow the required reading order once per agent session, then read the current prompt, relevant
handoff decisions, and affected code. Do not reload every prompt and old log after every commit.
Keep one short `handoffs/NN.md`: changes, commit IDs, checks/log paths, blockers, and next starting
point. Put raw logs under ignored `target/client-ready-workflows/`, not in handoffs or chat.

No new inference engine, scheduler, container backend, cluster management, hardware campaign,
self-improvement engine, or speculative frontend framework. Use existing public transports.
An authorized attached model remains valid; do not require containerized inference.

## Test as you build; test the full system in 06

| Phase | Checks |
| --- | --- |
| 00 | Changed documents, links, examples where applicable, and whitespace. |
| 01–05 | Compile affected packages and callers. Run focused behavior, failure, and affected example tests using controlled services. |
| 06 | Complete operator journey, full workspace gate, required local-model checks, and fixes. |

Write and run a regression with its fix. A small daemon/CLI test for one changed operation belongs
in that phase; the complete journey does not. Choose checks by affected behavior and callers,
verify filters discover the intended tests, and reuse successful results until changes affect them.
**Do not repeat the full workspace gate at every commit or handoff.**

Keep CI and test coverage intact: no skip markers, disabled jobs, or weaker assertions. Local
commits need no push. Push only when separately authorized; existing CI may then run as configured.
The final full check is still required even when earlier focused tests passed.

## Commit working changes regularly

Each prompt gives natural commit points. Complete a coherent change, run its focused checks,
review the diff, and commit before starting the next distinct change. Do not wait until the end
of the prompt. Keep a shared API change with its required callers when splitting would break them.

Start with `git status --short`. Stage explicit files or hunks, review `git diff --cached`, and
check whitespace. Include related tests. Do not stage someone else's changes, credentials, private
inputs, or generated logs. Use messages explaining the working change.

Preserve the checkpoints: no automatic squash, amend of earlier work, rebase, hard reset, or force
push. Fix a committed mistake in a new commit. Revert only when safe for other contributors and
record why. If interrupted with unfinished edits, record the last tested commit and remaining work;
do not pretend it is complete. A final documentation commit does not replace implementation commits.

## Finish

06 verifies the combined product and fixes known in-scope defects. Guides contain tested commands;
reports distinguish passed checks from missing access or model limitations. Required tests cannot
be silently skipped. Keep a blocked sprint open rather than calling it finished or starting another
to avoid a fix.
