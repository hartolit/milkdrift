# ADR-0007: Name the supported portability targets

- **Status:** Accepted
- **Date:** 2026-07-23

## Context

The portable feature crates use `no_std`, but `no_std` alone does not establish that code compiles for every bare-metal or WebAssembly target, nor does it prove that execution allocates no memory. Phase 1 requires concrete, reproducible targets instead of a generic portability claim. Adapter, engine, and application crates intentionally depend on host or vendor facilities and are outside this decision.

## Decision

Use these named targets as the Phase 1 portability contract:

- host correctness: `x86_64-unknown-linux-gnu` with normal `std` tests;
- WebAssembly compilation: `wasm32-unknown-unknown`;
- embedded `no_std` compilation: `thumbv7em-none-eabihf`.

The contract applies only to `domain-contracts`, `tokenization`, `context-planner`, `sampling`, and `task-graph`. CI compiles each crate's library target for both cross targets with the committed lockfile. It does not cross-compile tests or benchmarks because their development dependencies are host-oriented.

Allocation-free behavior is a separate, path-specific property. It may be claimed only where project-owned code has an allocation test or can execute without an allocator by construction; it is not inferred from successful cross-compilation.

## Rejected alternatives

- **Claim generic bare-metal support:** rejected because targets differ in atomics, floating-point support, pointer width, and platform assumptions.
- **Check every workspace crate for `no_std`:** rejected because adapters, engines, and applications deliberately require host/vendor facilities.
- **Cross-compile test and benchmark targets:** rejected because host-only development dependencies do not define the production portability boundary.
- **Treat `no_std` as evidence of zero allocation:** rejected because `alloc` can be used without `std`, and external/native code may allocate independently.

## Consequences

- Portability regressions for the five feature crates fail CI on two named non-host targets.
- Adding a new portable crate requires adding it to the target checks and the portability matrix.
- Supporting another embedded architecture, atomics profile, or WebAssembly runtime requires explicit validation rather than extrapolation.
- Allocation claims remain narrower than crate-level target support.

## Review trigger

Review when a portable crate needs a facility unavailable on either named target, when a product selects a different embedded/WebAssembly runtime, or after the approved dependency DAG replaces the temporary F1-to-F1 restriction.
