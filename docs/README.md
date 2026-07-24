# Documentation Map

This page defines which repository documents are authoritative and how to resolve conflicting guidance. Paths and links are relative to the repository root unless stated otherwise.

## Authority and precedence

When two documents conflict, use the first applicable source in this order:

1. a current, accepted architecture decision record (ADR);
2. the current normative architecture document;
3. the canonical current status document;
4. a component guide;
5. a historical implementation plan;
6. a knowledge note.

Code and executable checks remain the evidence for what is actually implemented. If a normative document disagrees with the workspace or a test, treat that as a defect and update the document or implementation explicitly; do not silently choose whichever is convenient.

## Normative architecture

- [Architecture](architecture.md) — current boundaries, ownership, dependency direction, and classification of rules.
- [Engineering rules](rules.md) — repository-wide implementation and review requirements.

These documents describe current policy. Changes to an accepted architectural decision require a superseding or amended ADR.

## Architecture decision records

- [ADR-0001: Keep `application-runtime` as the frontend-neutral façade](decisions/0001-application-runtime-facade.md)
- [ADR-0002: Use Candle CPU for the first vertical slice](decisions/0002-candle-cpu-first-vertical-slice.md)
- [ADR-0003: Schedule generation beside model execution](decisions/0003-generation-scheduling-ownership.md)
- [ADR-0004: Deliver direct completion before general chat](decisions/0004-direct-completion-before-chat.md)
- [ADR-0005: Retain the current crate folders](decisions/0005-retain-crate-folders.md)
- [ADR-0006: Require explicit bounded shutdown](decisions/0006-explicit-bounded-shutdown.md)
- [ADR-0007: Name the supported portability targets](decisions/0007-portability-targets.md)

ADRs record context, rejected alternatives, consequences, and review triggers. An ADR marked superseded is historical and no longer has precedence.

## Execution and current status

- [Execution plan](execution/execution-plan.md) — active ordered implementation program.
- [Architecture analysis](execution/analyzer.md) — evidence and findings that motivated the execution plan; it is analysis, not normative policy.
- [Current implementation status](project/implementation-status.md) — supported backends/devices, integration depth, validation evidence, known limitations, and active execution position.
- [Phase 4 implementation report](execution/PHASE4_IMPLEMENTATION_REPORT.md) — implemented Candle vertical-slice package, remaining validation gates, and completion rule.
- [Phase 4 Candle smoke](execution/phase4-candle-smoke.md) — pinned opt-in real-model procedure and diagnostic evidence format.

The status page is the canonical answer to “what works now?” A claim of validation must name a reproducible command and the source commit or CI run used.

## Component guides

- [Workspace boundaries](project/workspace.md)
- [Application runtime](project/application-runtime.md)
- [Inference runtime](project/inference-runtime.md)
- [Desktop runtime](project/desktop-runtime.md)
- [Model lifecycle](project/lifecycle.md)
- [Candle backend](project/candle-backend.md)
- [GGUF backend](project/gguf-backend.md)
- [Corrective workflow](project/orchestration.md)
- [Performance evidence](project/performance.md)
- [Dependency and repository policy](project/dependency-policy.md)
- [Portable feature targets](project/portability.md)

Component guides explain implemented behavior within one subsystem. They do not override an ADR, normative architecture, or current status.

## Historical and supporting material

- [Recovered implementation plan](project/implementation-plan.md) — historical planning input retained for context; it does not describe the current roadmap.
- [Rust knowledge notes](knowledge/rust_knowledge.md) — evidence-oriented engineering guidance, not architecture law.
- [Agent persona](persona.md) — collaboration guidance subordinate to this authority map.

`docs/project/` is intentionally retained to avoid a path-only migration. New documents should be indexed here and assigned one of the classes above.
