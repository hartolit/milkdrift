# Execution documentation

This directory contains the project's execution inputs and chronological closure record. It does not own current architecture, component behavior, product support, or reusable engineering doctrine.

## Active execution inputs

- [Architecture analysis](analyzer.md) — the evidence and findings that motivated the active program.
- [Execution plan](execution-plan.md) — the ordered implementation program and phase gates.

These two files are intentionally preserved as source artifacts. They are exempt from opportunistic restyling or consolidation during ordinary documentation cleanup; revise them only when the analysis or execution baseline itself changes.

## Historical execution record

- [Execution history](history.md) — consolidated closure evidence for completed phases, including acceptance matrices, recorded validation provenance, and measurements that remain useful historically.

Closed phases append to `history.md`. Do not add another `PHASE*_COMPLETION_REPORT.md` or `PHASE*_IMPLEMENTATION_REPORT.md` unless a future artifact has a genuinely independent long-lived purpose that cannot be represented as a history section.

## What does not belong here

- current project architecture → [`../project/architecture.md`](../project/architecture.md)
- current product support and validation state → [`../project/implementation-status.md`](../project/implementation-status.md)
- repeatable validation procedures → [`../project/validation.md`](../project/validation.md)
- component/backend behavior → [`../project/README.md`](../project/README.md)
- reusable architecture/rules/knowledge → top-level docs and [Rust systems knowledge](../knowledge/rust_knowledge.md)

Execution documents may link to those owners, but should not become parallel copies of them.

## Agent handoff

When an execution environment cannot run the Rust toolchain, source changes may be delivered as a patch with copy/paste apply and validation commands. The local operator runs the canonical gate on the resulting tree and returns failures verbatim. Historical evidence from a different tree must not be used as proof for the new patch.
