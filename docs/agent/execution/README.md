# Execution documentation

This directory contains the project's execution inputs and chronological closure record. It does not own current architecture, component behavior, product support, or reusable engineering doctrine.

## Execution inputs

- [Current execution context](current.md) — concise, mutable handoff for either the active phase or an explicitly parked state.
- [Architecture analysis](analyzer.md) — preserved evidence and findings that motivated the completed program.
- [Execution plan](execution-plan.md) — the ordered implementation program and phase gates.

`current.md` is derived operational context: it may restate selected facts so an execution agent can start quickly, but it links to canonical project owners and must record either the active phase or that no phase is active.

The analyzer and execution plan are intentionally preserved as source artifacts. They are exempt from opportunistic restyling or consolidation during ordinary documentation cleanup; revise their substance only when the analysis or execution baseline itself changes. Mechanical path corrections after repository moves are permitted.

## Historical execution record

- [Execution history](history.md) — consolidated closure evidence for completed phases, including acceptance matrices, recorded validation provenance, and measurements that remain useful historically.

Closed phases append to `history.md`. Do not add another `PHASE*_COMPLETION_REPORT.md` or `PHASE*_IMPLEMENTATION_REPORT.md` unless a future artifact has a genuinely independent long-lived purpose that cannot be represented as a history section.

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
