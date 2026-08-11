# Current execution context

**Status date:** 2026-08-11

```text
Phase 12 and the first four pristine-state remediation areas remain implemented.
A post-closure foundation repair is implemented in the current working tree but has not yet been validated with Rust tooling.
No workflow/workspace product program has been ratified or activated.
Historical Phase 12 and predecessor-tree results do not validate this repaired tree.
```

## Current maintenance scope

The repair closes the foundation defects found after the unfinished independent
closure:

- mixed required `{F16,F32}` and `{BF16,F32}` layouts require the matching
  recognized producer declaration instead of inferring a lossy primary from an
  unordered set;
- ordinary-drop-safe preparation and resource-bearing failed materialization are
  separate associated typestates;
- the project-owned `FailedLoad` guard encapsulates the raw cleanup owner and
  retains it fail-closed when direct error propagation abandons unresolved
  ownership;
- E0 verifies failed-owner plan stability before and after every cleanup attempt,
  reclassifies substitution or mutation as unverified ownership, and preserves
  conservative evidence monotonically;
- Candle sequence planning separates persistent all-layer KV/cache ownership from
  one block's simultaneous transient peak and outer model state; and
- canonical project documentation describes those stronger contracts without
  promoting them to accepted evidence before validation.

This package does not implement workflow/workspace features. The next product
program remains inactive until the repaired foundation receives an exact-tree
greenlight.

## Current implementation boundary

The local product remains the unquantized Llama Safetensors path through Candle,
E0, optional E1 reference services, and the thin Slint host. CPU is
mandatory/default. CUDA remains non-default, explicitly selected, ordinal-based,
and no-fallback.

Configuration declaration, complete observed scalars, required scalar policy,
execution scalar, source identity, final/loading-peak ownership, sequence logical
reservation, and retained ownership certainty are separate facts. Structurally
understood unused tensors remain observed and identity-checked but are not
materialized or included in tensor execution footprints.

Current component and support truth is owned by
[implementation status](../../project/implementation-status.md), not this handoff.

## Validation and evidence truth

- No compilation, formatting, Clippy, test, cross-target, benchmark, or CUDA
  result is currently recorded for this repair. Validation begins only after the
  exact patch is applied to the reviewed base.
- Phase 12 self-hosted CUDA
  [run 31281013243](https://github.com/hartolit/milkdrift/actions/runs/31281013243)
  succeeded on closure commit `181a069ce81525e9c144fe8de051ced8e3c0b9d7` only.
- Phase 12 Quality
  [run 31281013257](https://github.com/hartolit/milkdrift/actions/runs/31281013257)
  passed its canonical native work and then exhausted disk under the superseded
  CI topology. It is infrastructure history, not evidence for this repair.
- The predecessor pristine-state tree has local native/portable evidence recorded
  in implementation status; that evidence also predates this repair.
- External mixed-checkpoint evidence remains absent. Historical reports retain
  their original schema and exact commit attribution.

## Handoff and acceptance order

1. Apply the repair to the exact reviewed base and verify a clean diff.
2. Run the focused portable-contract, Candle, E0, E1, persistence, and benchmark
   tests in [validation](../../project/validation.md).
3. Run `cargo fmt --all -- --check`, `cargo xtask architecture`,
   `cargo xtask hygiene`, and the isolated canonical `cargo xtask verify` gate.
4. Run both named portable target matrices in isolated targets.
5. On the supported NVIDIA host, run the exact CUDA check/Clippy graph and all
   dedicated adapter/E0/fault/E1 hardware suites.
6. Record only results from the exact repaired commit/tree. Update current
   evidence and history after those results exist, not before.
7. Refuse workflow/workspace activation while any correctness, ownership,
   capacity, portability, or documentation contradiction remains.

Canonical owners:

- [Project architecture](../../project/architecture.md)
- [Workspace roles and members](../../project/workspace.md)
- [Dependency policy](../../project/dependency-policy.md)
- [Validation procedures](../../project/validation.md)
- [Performance and measurement registry](../../project/performance.md)
- [Execution plan](execution-plan.md)
- [Execution history](history.md)
