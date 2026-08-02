# LLM App execution plan

**Plan status:** No implementation phase is active
**Status date:** 2026-08-02

```text
Phase 10 repository infrastructure and synthetic acceptance: complete.
External real-product baseline: outstanding.
Phase 11: not active.
```

This document owns active and future objectives, work packages, acceptance criteria, and ordering. Current product truth lives in [implementation status](../../project/implementation-status.md), repeatable commands in [validation](../../project/validation.md), performance methodology and results in [performance evidence](../../project/performance.md), and closed-tree chronology in [execution history](history.md).

## Program objective

Build a local-first model application without weakening explicit ownership, bounded resource use, portable domain logic, or truthful support claims. The established product path is:

```text
user input
  -> prompt or exact compatible chat rendering
  -> tokenization and context admission
  -> E0 prefill, sampling, and incremental decode
  -> bounded output
  -> cancellation or completion
  -> explicit release, unload, and shutdown
```

Candle with immutable Hugging Face Hub Safetensors on CPU remains the sole current local composition. New formats, engines, devices, deployment targets, or public abstraction layers require separate evidence and review.

## Governing decisions

- [ADR-0006](../decisions/0006-explicit-bounded-shutdown.md) requires bounded, observable shutdown and retained ownership when cleanup cannot be proved.
- [ADR-0013](../decisions/0013-candle-only-local-execution.md) keeps Candle as the sole local engine.
- [ADR-0015](../decisions/0015-exact-reviewed-domain-dependency-dag.md) owns the reviewed domain dependency graph.
- [ADR-0016](../decisions/0016-virtual-workspace-focused-xtask.md) owns the virtual workspace and Rust-native verification boundary.
- [ADR-0017](../decisions/0017-stable-clippy-gate-exploratory-nursery.md) separates mandatory and exploratory lint policy.
- [ADR-0018](../decisions/0018-benchmark-and-model-fixture-policy.md) separates benchmark roles and governs fixtures and generated evidence.

## Completed program context

| Phase | Durable outcome | Current authority |
|---|---|---|
| 0–8 | Documentation, quality gates, transactional runtime safety, generation, E1, Slint, conversation behavior, and the historical GGUF experiment were delivered in sequence. | [Execution history](history.md); current support is not inferred from superseded phase text. |
| 9 | Candle-only composition, lifecycle hardening, exact domain DAG, virtual workspace tooling, module reconciliation, and stable lint policy were closed. | Current [architecture](../../project/architecture.md), [workspace](../../project/workspace.md), and accepted ADRs. |
| Pre-10 | Terminal cleanup semantics, benchmark placement, artifact hygiene, and fixture provenance were established. | ADR-0006 and ADR-0018. |
| 10 repository infrastructure | Sampling and runtime measurement surfaces plus deterministic acceptance gates were implemented and accepted on a later clean exact tree. | [Phase 10 history](history.md#phase-10--repository-infrastructure-and-synthetic-acceptance) and [performance evidence](../../project/performance.md). |

Detailed completion inventories, command results, and timing intervals do not belong in this plan.

## Phase 10 closure boundary

### Repository infrastructure and synthetic acceptance — complete

The completed work packages are retained only as future context:

| Work package | Durable result |
|---|---|
| 10.1 Sampling | The crate-owned benchmark has explicit `sample_only` and `restore_and_sample` boundaries, an ordinary one-shot matrix test, and deterministic allocation coverage. |
| 10.2 Runtime package | `benchmarks/runtime` is the sole non-production cross-crate benchmark observer and uses reviewed public APIs, the root lockfile, and the root target. |
| 10.3 Lifecycle and memory | The bounded synthetic runner exercises hosted E0 lifecycle/accounting/RSS observations plus fresh download-free E1 start/shutdown cycles. |
| 10.4 Candidate review | Unsupported checklist benchmarks remain deferred until a named decision and evidence gap justify them. |
| 10.5 Evidence format | The runner emits one versioned JSON report with allowlisted environment, fixture, lifecycle, accounting, and process-memory metadata. |
| 10.6 Optimization discipline | No production optimization or product-axis expansion was accepted from synthetic measurements alone. |

Repository acceptance requires all of the following on one clean code-under-test commit:

- deterministic domain allocation validation;
- one-shot sampling-matrix smoke coverage;
- focused runtime package tests;
- workspace benchmark-target compilation;
- clean bounded synthetic lifecycle execution;
- the canonical repository gate, architecture and hygiene policy, dependency policy, and documentation links;
- both named portable-domain target checks;
- no package-local targets, benchmark lockfiles, or generated source-tree artifacts.

The exact procedure is canonical in [validation](../../project/validation.md). The accepted Commit A identity and summarized outcome are recorded in [history](history.md#phase-10--repository-infrastructure-and-synthetic-acceptance); exact measurements exist only in [performance evidence](../../project/performance.md).

### External real-product baseline — outstanding

This is a separate evidence class. Its current implementation and evidence status is canonical in [performance evidence](../../project/performance.md#external-product-evidence); repository synthetic acceptance does not satisfy it.

A future external baseline work package must:

1. select and document one exact external model identifier and immutable revision;
2. use a narrow opt-in path through public E1 behavior without duplicating product orchestration;
3. require explicit network authorization and an allowed cache or artifact location;
4. actually resolve, load, generate decoded output, release, unload, and shut down successfully;
5. record controlled environment, timing, memory, identity, lifecycle, and limitations without exposing text, token IDs, credentials, or caches;
6. place raw output under root `target` or outside the repository and add only curated results to `docs/project/performance.md`;
7. pass the canonical repository gate on the resulting clean tree.

Acceptance requires an observed run against the exact model/revision. Source presence, compilation, cached artifacts, successful resolution alone, or historical smoke evidence is insufficient.

No network access is authorized for the current closure, so this work package is not active.

## Phase 11 — GPU execution

**Status:** Not active.

**Activation prerequisite:** record the outstanding external real-product baseline, reconcile any product-path issue it exposes, and begin Phase 11 from a clean accepted CPU tree. Do not activate Phase 11 merely because repository benchmark infrastructure exists.

### Objective

Add explicit GPU device support without redesigning E1, weakening CPU behavior, or presenting feature compilation as execution evidence.

### Work package 11.1 — Supported device matrix

- Select exact supported Candle/device combinations.
- Keep engine, artifact source, model format, scalar type, and device as distinct facts.
- Define unsupported combinations and explicit CPU fallback policy.

### Work package 11.2 — Build and CI matrix

- Add deliberate device features rather than assuming `--all-features` is valid.
- Keep CPU compilation and behavior mandatory.
- Separate hardware-executed jobs from compile-only feature checks.

### Work package 11.3 — Discovery, admission, and lifecycle

- Add stable device identity and capability reporting.
- Admit model, sequence, and workspace memory against device limits.
- Preserve cancellation, synchronization, unload, and shutdown ownership.
- Fail unsupported selections before partial residency where possible.

### Work package 11.4 — E1 and frontend exposure

- Expose frontend-neutral device summaries and selection through E1.
- Keep Slint presentation-only and free of vendor/runtime types.

### Work package 11.5 — Executed device evidence

- Measure load, first output, decode throughput, host/device memory, cancellation, unload/synchronization, transfer behavior, fallback, and CPU comparison.
- Distinguish compile-only targets from hardware-executed results.

### Phase 11 acceptance criteria

- Inference executes on every claimed GPU target.
- CPU behavior remains covered and available.
- Unsupported combinations fail explicitly.
- Device resources are released on completion, cancellation, unload, shutdown, and contract failure.
- UI labels and evidence identify the device actually used.
- No shared-CI wall-clock threshold is introduced without a controlled runner and reviewed policy.

## Future execution tracks

These tracks are intentionally unnumbered and inactive:

- **Peer and hosted execution:** define a coarse application request/stream contract only when a real second deployment target proves the seam; do not represent remote services as E0 backends.
- **Composable workflows:** evolve beyond `corrective-workflow` only when independent ownership and lifecycle are demonstrated.
- **Long-term memory and context repair:** preserve raw provenance while testing concrete retrieval and active-context behavior before selecting storage architecture.
- **Tools, authority, and trust:** make capabilities explicit, narrow, inspectable, and revocable; credentials never enter model context.
- **Long-lived node and multiple frontends:** separate node/service lifetime from any one terminal or window.

Project vision motivates these tracks but does not activate them.

## Critical ordering

```text
accepted CPU product
  -> Phase 10 repository infrastructure and synthetic acceptance
  -> executed external real-product baseline
  -> resolve baseline findings
  -> activate Phase 11 GPU work
```

Peer/hosted execution and research tracks start only from their own reviewed evidence triggers; they are not implicit successors to GPU work.

## Operating rules

1. Start each work package from a clean committed tree and record commit, tree, and dirty state.
2. Use the pinned toolchain, committed lockfile, root target, and canonical verification procedure.
3. Preserve public APIs unless a work package explicitly authorizes change.
4. Add deterministic tests for new invariants and reproduced failures.
5. Keep synthetic, component, product, and compile-only evidence distinct.
6. Do not claim allocation freedom, portability, product compatibility, or device execution without a named gate or observed run.
7. Update the canonical owner for changed status, methodology, procedure, or chronology; link instead of copying.
8. Record architectural changes in an ADR.
9. Leave Phase 11 and inactive future tracks untouched until their activation criteria are met.
