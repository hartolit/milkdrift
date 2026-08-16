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
- [ADR-0007: Name the supported portability targets](0007-portability-targets.md) — package set amended by ADR-0021
- [ADR-0008: Separate application coordination, capabilities, and model execution](0008-capability-and-execution-boundaries.md) — superseded by ADR-0021
- [ADR-0009: Adopt domain, platform, adapter, runtime, and app roots](0009-workspace-physical-taxonomy.md) — package set amended by ADR-0021
- [ADR-0010: Verify backend contracts at E0](0010-verify-backend-contracts-at-e0.md)
- [ADR-0011: Bound workflow output at the service port](0011-bound-workflow-output-at-the-port.md) — superseded by ADR-0021
- [ADR-0012: Keep local native composition private inside E1](0012-local-native-composition.md) — superseded by ADR-0013
- [ADR-0013: Use Candle as the sole local execution engine](0013-candle-only-local-execution.md) — device dimension amended by ADR-0019
- [ADR-0014: Keep project-owned operational tooling Rust/Cargo-native](0014-rust-cargo-native-operational-tooling.md)
- [ADR-0015: Use an exact reviewed domain dependency DAG](0015-exact-reviewed-domain-dependency-dag.md) — enforcement amended to explicit roles plus Cargo-derived acyclicity; package set amended by ADR-0021
- [ADR-0016: Use a virtual workspace and a focused `xtask`](0016-virtual-workspace-focused-xtask.md) — exact benchmark-target verification amended
- [ADR-0017: Keep stable Clippy lints mandatory and nursery exploratory](0017-stable-clippy-gate-exploratory-nursery.md) — mandatory gate amended by ADR-0021
- [ADR-0018: Separate benchmark roles and govern measurement artifacts](0018-benchmark-and-model-fixture-policy.md) — observer roles and maintained-target registration amended
- [ADR-0019: Add explicit feature-gated CUDA execution with application-owned selection](0019-explicit-cuda-execution-foundation.md) — Phase 12 loading/scalar clauses amended by ADR-0020
- [ADR-0020: Use transaction-bound prepared model loading](0020-transactional-prepared-model-loading.md) — byte ownership and sequence-rate clauses amended by ADR-0022
- [ADR-0021: Keep only present canonical package responsibilities](0021-canonical-present-scope.md)
- [ADR-0022: Make deterministic byte ownership typed and non-contradictory](0022-memory-accounting-and-byte-ownership.md)

For current applied structure, see [project architecture](../../project/architecture.md). For the reusable selectable model, see the [architecture blueprint](../../architecture.md).
