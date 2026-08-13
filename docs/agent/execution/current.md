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
Exact-tree local CPU, component, composite, portable, policy, and link acceptance passes on commit b1f7e90b1ba67f1cf968d773052b5062ef8cbbb9, tree fcb3ee6fa00243734abd74b64218aa0db2e340c1.
No repair was required during closure validation; remote Quality, current CUDA hardware, and external-model evidence remain separate and pending.
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

## Foundation closure 05 local evidence

The exact clean executable candidate was commit
`b1f7e90b1ba67f1cf968d773052b5062ef8cbbb9`, tree
`fcb3ee6fa00243734abd74b64218aa0db2e340c1`. A tracked document cannot name the
commit/tree created by committing itself; the resulting documentation-only
closure commit and its post-commit checks belong in the completion report.

The host was Linux `7.1.6-arch1-1` x86_64 with Rust 1.96.1, Cargo 1.96.1, an
AMD Ryzen 9 7940HS (8 physical/16 logical cores), 60 GiB reported memory, 4 GiB
zram swap, and a 953 GiB Btrfs filesystem with 782 GiB initially free. The
observed display adapter was AMD Radeon 780M. `nvidia-smi` and `nvcc` were absent;
CUDA was therefore not attempted and is not a local failure. Clean evidence
targets used `CARGO_INCREMENTAL=0` and were built sequentially.

Thirty-one validation commands passed with no failing command: six structural,
fourteen focused, six hosted-parity components, one composite, two portable, and
two policy/link commands. The focused commands reported 564 passing libtest or
doctest cases, plus successful harness-free allocation targets; the canonical
workspace test plan reported 525 passing cases. The sole ignored test was the
explicit source-tree fixture-regeneration maintenance operation.

The focused matrix ran `application-runtime` three times, `redb-storage`,
`domain-contracts`, Candle CPU and full package tests, all four maintained E0
runtime/generation/fault/native-backend targets, `task-graph`,
`corrective-workflow`, and the complete `xtask` suite. It proves the deterministic
CPU fixtures and scalar facts, sequence plan/create/prefill/decode/destroy and
reservation checks, backpressure/cancellation, ordinary unload, failed-load
cleanup/retry/exhaustion, mismatch and unverified retention, the unified E1
coordinator, and bounded shutdown/disconnection truth. No RSS leak claim is made.

Fresh hosted-parity target observations were 145,840 KiB (structure), 2,027,432
KiB (check), 7,681,172 KiB (tests), 2,027,432 KiB (Clippy), 2,041,140 KiB
(rustdoc), and 1,253,884 KiB (exact benches). The clean local composite retained
10,405,360 KiB. Each portable target retained 152,232 KiB and compiled exactly
`context-planner`, `domain-contracts`, `sampling`, `task-graph`, and
`tokenization` plus `libm`. Pinned cargo-deny 0.20.2 passed advisories, bans,
licenses, and sources; pinned Lychee 0.24.2 checked 256 links with 0 errors.

No source, test, workflow, or documentation defect was exposed before evidence
recording. GitHub-hosted Quality, current self-hosted RTX CUDA acceptance, and a
reviewed external mixed checkpoint remain pending. This local result does not
authorize AMD support or the workflow/workspace product program.

## Handoff and acceptance order

1. Push only when requested and observe the redesigned hosted Quality run on the
   exact pushed closure commit.
2. On the supported NVIDIA host, run the exact CUDA check/Clippy graph and all
   dedicated adapter/E0/fault/E1 hardware suites.
3. Record only results from the exact repaired commit/tree. Update current
   evidence and history after those results exist, not before.
4. Refuse AMD or workflow/workspace activation while any correctness, ownership,
   capacity, portability, or documentation contradiction remains.

Canonical owners:

- [Project architecture](../../project/architecture.md)
- [Workspace roles and members](../../project/workspace.md)
- [Dependency policy](../../project/dependency-policy.md)
- [Validation procedures](../../project/validation.md)
- [Performance and measurement registry](../../project/performance.md)
- [Execution plan](execution-plan.md)
- [Execution history](history.md)
