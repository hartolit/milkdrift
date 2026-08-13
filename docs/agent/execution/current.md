# Current execution context

**Status date:** 2026-08-13

```text
Phase 12 and the first four pristine-state remediation areas remain implemented.
The post-closure Candle sequence-accounting repair passes its required focused Rust validation matrix in an isolated target directory.
The E1 cleanup-coordination repair is implemented and its required focused local validation passes in an isolated target directory.
The orchestration-boundary repair is implemented: task-graph is generic and allocation-free, while the corrective flow is validated template data interpreted by one bounded executor.
Its focused host, WASM, embedded, architecture, hygiene, and documentation gates pass locally in an isolated target directory.
The CI resource-topology repair is implemented: the local composite and hosted native components share one metadata-owned plan, and hosted profiles now use separate standard-runner targets with centralized cleanup.
No workflow/workspace product program has been ratified or activated.
The new component/portable plans pass local source-tree validation; remote Quality, current CUDA hardware, external-model, and final execution evidence remain separate and pending.
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

The orchestration repair removes corrective/model policy from `task-graph`, keeps
only generic topology, attempt state, readiness, cancellation/blocking, and
identity-only artifact provenance there, and measures those prepared paths for
zero allocation. `corrective-workflow` now owns borrowed bounded definition data,
typed corrective artifact/operation/policy vocabulary, a six-stage reference
template, deterministic ready-node scheduling, transactional artifact/event
commit, rollback, cancellation, and explicit release. A second structurally
different definition executes through the same scheduler.

This package repairs incubating workflow foundations but does not activate the
general workflow/workspace product program. Persistent workspaces, arbitrary
plugins/nodes, providers/peers, recursive durable runs, effect authority, and a
visual editor remain unimplemented.

The E1 repair replaces the overlapping incompatible-unload and retained-inspection
trackers with one private checked coordinator, keeps lower and E1 retry counts
independent, stores complete public cleanup evidence in `ApplicationState`, and
uses compact cleanup transition events. It changes no durable persistence schema
and moves no cleanup policy into Slint or E0.

The verification repair exposes `cargo xtask verify-component` for structure,
check, test, Clippy, docs, exact maintained benches, and exploratory nursery
linting. `cargo xtask verify` consumes the same six canonical plans. Quality gives
each heavy profile a fresh Ubuntu 24.04 runner and unique `RUNNER_TEMP` target;
portable, policy, nursery, and link work remain separate. These are implemented
source-tree facts, not an accepted hosted run.

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

- The required formatting, package checks/tests, strict Clippy, rustdoc,
  architecture, hygiene, and diff checks pass locally in an isolated target. The
  E0 fault-injection boundary target also passes. These are source-tree results,
  not canonical, portable, CUDA, remote, external-model, or performance evidence;
  the package completion record names the resulting commit and tree.
- The orchestration package separately passes its exact requested focused matrix:
  format; all-target checks for `domain-contracts`, `task-graph`, and
  `corrective-workflow`; graph and corrective tests; strict Clippy;
  warning-denied rustdoc; `wasm32-unknown-unknown` and
  `thumbv7em-none-eabihf`; architecture; hygiene; and diff whitespace. The graph
  suite includes its harness-free zero-allocation contract.
- Phase 12 self-hosted CUDA
  [run 31281013243](https://github.com/hartolit/milkdrift/actions/runs/31281013243)
  succeeded on closure commit `181a069ce81525e9c144fe8de051ced8e3c0b9d7` only.
- Phase 12 Quality
  [run 31281013257](https://github.com/hartolit/milkdrift/actions/runs/31281013257)
  passed its canonical native work and then exhausted disk under the superseded
  CI topology. It is infrastructure history, not evidence for this repair.
- The predecessor pristine-state tree has local native/portable evidence recorded
  in implementation status; that evidence also predates this repair.
- The redesigned verification plans, workflow policy tests, embedded shell
  syntax checks, six canonical native components, and both portable targets pass
  locally from fresh isolated targets. The scheduled nursery command starts and
  reports exploratory lints separately; its findings are not a canonical failure.
  No redesigned GitHub-hosted Quality or current self-hosted CUDA run exists yet.
- External mixed-checkpoint evidence remains absent. Historical reports retain
  their original schema and exact commit attribution.

## Handoff and acceptance order

1. Run the isolated canonical `cargo xtask verify` gate when complete repository
   acceptance is required; hosted CI must use the matching typed component plans.
2. Run both named portable target matrices in isolated targets.
3. On the supported NVIDIA host, run the exact CUDA check/Clippy graph and all
   dedicated adapter/E0/fault/E1 hardware suites.
4. Record only results from the exact repaired commit/tree. Update current
   evidence and history after those results exist, not before.
5. Refuse workflow/workspace activation while any correctness, ownership,
   capacity, portability, or documentation contradiction remains.

Canonical owners:

- [Project architecture](../../project/architecture.md)
- [Workspace roles and members](../../project/workspace.md)
- [Dependency policy](../../project/dependency-policy.md)
- [Validation procedures](../../project/validation.md)
- [Performance and measurement registry](../../project/performance.md)
- [Execution plan](execution-plan.md)
- [Execution history](history.md)
