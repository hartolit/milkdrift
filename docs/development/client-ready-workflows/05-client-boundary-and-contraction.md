# 05 — Keep the interface thin and remove duplicate rules

After 04 is ready, follow [the shared rules](README.md),
[implementation practice](../../practices/implementation.md), and
[documentation practice](../../practices/documentation.md). Check how the implemented operations
fit together. This is not a place for earlier agents to leave known unfinished features.

## Keep decisions in the daemon

Trace the changed operations through CLI, public client/API, daemon, and the existing Rust owners.
Start with `apps/cli`, `crates/control-client`, `crates/control-protocol`, and the daemon handlers.
Look for the same defaults, validation, input assembly, request recovery, or status rules written
more than once. Move each shared rule to its proper existing owner and migrate all callers.
Delete the replaced path; do not wrap it with yet another layer. Preserve genuinely different
direct, workflow, and published-call behavior.

The first GUI is **Svelte**. It must not need CLI subprocesses, a database connection, Rust-only
private helpers, or its own permission/workflow engine to do what the CLI does. Complete a missing
public operation instead of planning to duplicate it in JavaScript later.

A frontend can keep unsaved edits and display choices. The daemon validates saved definitions,
enforces permissions, applies allowed changes, starts work, and reports authoritative state.
Do not force every mouse movement or draft keystroke through the daemon; the boundary concerns
product decisions, not presentation. Do not add Svelte, Tauri, or Iced code during this sprint.

## Prove another client can use it

Add a small maintained test or example using the daemon's public operations without invoking the
CLI. Rust is fine, but the test must not depend on private domain helpers to supply decisions a
browser client cannot obtain through the API. Use ordinary request payloads and public responses.

Cover authoring/saving, starting with inputs, recovering the same request, reading/downloading the
result, and reuse or permitted repair. These can be focused cases; 06 runs their complete journey.
An external model fixture is allowed. Creating an in-process privileged runtime, fabricating
successful records, or opening the daemon's database is not.

This proves an independent client path, not browser authentication, CORS, accessibility, or desktop
packaging. Those remain GUI work. Do not create a generic SDK generator, new transport, or speculative
frontend framework. Keep the public surface only as large as these supported operations need.

## Check and explain the finished route

Run the focused non-CLI cases, affected CLI automation, and relevant API/dependency-boundary tests.
Search for removed helpers and conflicting current Iced instructions. Fix wrong or missing guide
steps. A newcomer should follow one maintained CLI guide, with administrator setup explained once.
Do not repeat the whole-system test or full workspace gate.

## Commit points

Commit each completed shared-rule cleanup with all callers and focused tests. Do not put unrelated
cleanups into one large commit. Commit the independent-client examples/tests separately where
possible, then the remaining guide corrections. Keep earlier working commits in history.

Stop when clients use one implementation of the product rules and the next GUI will only expose it.
Record the substantive removals, public operations, commits, and focused results in `handoffs/05.md`.
List the full-system checks 06 still owns without describing them as already passed.
