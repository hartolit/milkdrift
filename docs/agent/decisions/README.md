# Architecture decisions

This directory records project-specific architectural decisions. ADRs explain the context, selected decision, rejected alternatives, consequences, and the condition that should trigger review.

Accepted ADRs remain part of the project architecture until superseded. A superseded ADR stays in the repository as historical rationale and links to its replacement.

## Current ADRs

- [ADR-0001: Keep `application-runtime` as the frontend-neutral façade](0001-application-runtime-facade.md)
- [ADR-0002: Use Candle CPU for the first vertical slice](0002-candle-cpu-first-vertical-slice.md)
- [ADR-0003: Schedule generation beside model execution](0003-generation-scheduling-ownership.md)
- [ADR-0004: Deliver direct completion before general chat](0004-direct-completion-before-chat.md)
- [ADR-0005: Retain the current crate folders](0005-retain-crate-folders.md) — superseded by ADR-0009
- [ADR-0006: Require explicit bounded shutdown](0006-explicit-bounded-shutdown.md)
- [ADR-0007: Name the supported portability targets](0007-portability-targets.md)
- [ADR-0008: Separate application coordination, capabilities, and model execution](0008-capability-and-execution-boundaries.md)
- [ADR-0009: Adopt domain, platform, adapter, runtime, and app roots](0009-workspace-physical-taxonomy.md)
- [ADR-0010: Verify backend contracts at E0](0010-verify-backend-contracts-at-e0.md)
- [ADR-0011: Bound workflow output at the service port](0011-bound-workflow-output-at-the-port.md)
- [ADR-0012: Keep local native composition private inside E1](0012-local-native-composition.md)

For current applied structure, see [project architecture](../../project/architecture.md). For the reusable selectable model, see the [architecture blueprint](../../architecture.md).
