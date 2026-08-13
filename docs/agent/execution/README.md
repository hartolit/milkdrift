# Execution documentation

This directory contains the project's current execution handoff and chronological
closure record. It does not own current architecture, component behavior,
product support, or reusable engineering doctrine.

## Current authority

- [Current execution context](current.md) — concise, mutable handoff for either the active phase or an explicitly parked state.
- [Execution plan](execution-plan.md) — the ordered implementation program and phase gates.

`current.md` is derived operational context: it may restate selected facts so an execution agent can start quickly, but it links to canonical project owners and must record either the active phase or that no phase is active.

These are the only current execution authorities in this directory. Completed
prompt files are not instructions for a new agent.

## Historical execution record

- [Execution history](history.md) — consolidated closure evidence for completed phases, including acceptance matrices, recorded validation provenance, and measurements that remain useful historically.
- [Architecture analysis](analyzer.md) — preserved source evidence and findings that motivated the completed program.
- [Completed prompt archive](archive/README.md) — completed work-package prompts retained for decision provenance, not active execution.

Closed phases append to `history.md`. Do not add another `PHASE*_COMPLETION_REPORT.md` or `PHASE*_IMPLEMENTATION_REPORT.md` unless a future artifact has a genuinely independent long-lived purpose that cannot be represented as a history section.

Archived prompts may link to the durable outcomes they produced, but agents do
not need to load the archive unless tracing a historical decision. Mechanical
path corrections are permitted; otherwise preserve archival text as historical
evidence.

## What does not belong here

- current project architecture → [project architecture](../../project/architecture.md)
- current product support and validation state → [implementation status](../../project/implementation-status.md)
- repeatable validation procedures → [project validation](../../project/validation.md)
- component/backend behavior → [project documentation](../../project/README.md)
- reusable architecture/rules → [architecture blueprint](../../architecture.md) and [engineering rules](../../rules.md)
- reusable Rust knowledge → [Rust systems knowledge](../knowledge/rust_knowledge.md)

Execution documents may link to those owners, but should not become parallel copies of them.

## Agent handoff

When an execution environment cannot run the Rust toolchain, source changes may be delivered as a patch with copy/paste apply and validation commands. The local operator runs the canonical gate on the resulting tree and returns failures verbatim. Historical evidence from a different tree must not be used as proof for the new patch.
