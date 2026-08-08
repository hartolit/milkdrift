# Milkdrift execution plan

**Plan status:** Phase 12 complete; workflow/workspace/authority is the planned next major program
**Status date:** 2026-08-08

```text
Phase 10 complete.
Phase 11 complete for the executed CPU + Linux CUDA matrix.
Post-Phase 11 quality closure complete.
Phase 12 complete for deterministic CPU compatibility and the exact local CUDA matrix.
No subsequent product phase is active.
```

This document owns current ordering, completed program context, the Phase 12 closure boundary, and the planned successor. Current product truth lives in [implementation status](../../project/implementation-status.md); repeatable procedures and recorded gates live in [validation](../../project/validation.md), schema semantics and preserved measurements in [performance evidence](../../project/performance.md), and chronology in [execution history](history.md).

## Current program objective

Phase 12 closed the reviewed compatibility tranche without weakening explicit ownership or allowing local-loader work to redefine Milkdrift. Its boundary is:

```text
configuration-declared metadata
  -> observed per-tensor Safetensors layout
  -> exact prepared final/loading-peak plan
  -> E0 admission and materialization
  -> verified actual execution facts
  -> explicit release, retained cleanup, unload, and shutdown
```

Candle with immutable Hugging Face Hub Safetensors remains the sole local composition. CPU is mandatory and default; Phase 12 deterministic CUDA support is limited to the exact locally executed RTX 5070 Ti row. The Phase 12 GitHub self-hosted workflow remains unrun, and no external mixed-checkpoint claim exists. The next major program returns to workflow, workspace, artifact, and authority foundations rather than extending the loader indefinitely.

## Governing decisions

- [ADR-0006](../decisions/0006-explicit-bounded-shutdown.md) requires bounded, observable shutdown and retained ownership when cleanup cannot be proved.
- [ADR-0013](../decisions/0013-candle-only-local-execution.md) keeps Candle as the sole local engine.
- [ADR-0015](../decisions/0015-exact-reviewed-domain-dependency-dag.md) owns the reviewed domain dependency graph.
- [ADR-0016](../decisions/0016-virtual-workspace-focused-xtask.md) owns the virtual workspace and Rust-native verification boundary.
- [ADR-0017](../decisions/0017-stable-clippy-gate-exploratory-nursery.md) separates mandatory and exploratory lint policy.
- [ADR-0018](../decisions/0018-benchmark-and-model-fixture-policy.md) separates benchmark roles and governs fixtures and generated evidence.
- [ADR-0019](../decisions/0019-explicit-cuda-execution-foundation.md) owns explicit CUDA execution, application selection, and no-fallback policy.

## Completed program context

| Phase | Durable outcome | Current authority |
|---|---|---|
| 0–8 | Documentation, quality gates, transactional runtime safety, generation, E1, Slint, conversation behavior, and the historical GGUF experiment were delivered in sequence. | [Execution history](history.md); current support is not inferred from superseded phase text. |
| 9 | Candle-only composition, lifecycle hardening, exact domain DAG, virtual workspace tooling, module reconciliation, and stable lint policy were closed. | Current [architecture](../../project/architecture.md), [workspace](../../project/workspace.md), and accepted ADRs. |
| Pre-10 | Terminal cleanup semantics, benchmark placement, artifact hygiene, and fixture provenance were established. | ADR-0006 and ADR-0018. |
| 10 | Sampling/runtime measurement surfaces, deterministic acceptance gates, and the exact external CPU product baseline were implemented and accepted on clean exact trees. | [Phase 10 history](history.md#phase-10--repository-infrastructure-and-synthetic-acceptance), [external closure](history.md#phase-10--external-cpu-baseline-closure), and [performance evidence](../../project/performance.md). |
| 11 | Mandatory CPU plus explicit CUDA ordinal 0 execution, E1/Slint selection, lifecycle evidence, and product benchmarking were accepted for the executed Linux x86_64 RTX 5070 Ti matrix only. | [Phase 11 history](history.md#phase-11--executed-cpu--linux-cuda-closure), [implementation status](../../project/implementation-status.md#historical-phase-11-evidence), and [performance evidence](../../project/performance.md#external-product-evidence). |
| 12.1 | Per-tensor inspection, exact preparation/admission, mixed conversion, and transactional partial-load ownership were committed. | Segment 1 commit `58490fe`; [segmented guide](milkdrift-phase12-execution-guide.md). |
| 12.2 | Immutable artifact metadata, E1 execution truth, persistence compatibility, retained cleanup events, and thin-host adaptation were committed. | Segment 2 commit `1251069`; [artifact/application specification](milkdrift-phase12-application-artifact-integration.md). |

Phases 10–12 plus the post-Phase 11 quality maintenance closure are completed program context. No subsequent product phase is active; workflow/workspace/authority is the planned successor and requires a reviewed activation decision. Detailed historical timing intervals remain outside this plan.

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

### External real-product baseline — complete

This separate evidence class completed on clean Commit C `771c0de4d72565a6302ca60f3b6bafd8c807962b`, tree `3d5b6ccc5ecc959de7cb370c1147f76e4cd32e3f`. Its authoritative implementation, exact model/revision, observed lifecycle, controlled timing, process-memory evidence, and limitations are canonical in [performance evidence](../../project/performance.md#external-product-evidence).

The completed work package:

1. fixes one exact external model identifier and immutable revision;
2. uses one narrow opt-in `runtime-benchmarks` path through public E1 behavior and removes the superseded application-runtime smoke orchestration;
3. requires explicit network authorization and an allowed canonical cache location;
4. resolves, loads, proves compatible chat, runs controlled direct completion, observes release, unloads, and shuts down successfully;
5. records controlled environment, timing, memory, identity, lifecycle, and limitations without exposing text, token IDs, credentials, or caches;
6. keeps raw output beneath ignored root `target` and places curated results only in `docs/project/performance.md`;
7. restores a clean CPU tree and passes the canonical local repository gate.

The observed run used `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at `fe8a4ea1ffedaf415f4da2f062534de366a451e6`. Source presence, compilation, cache presence, or resolution alone did not satisfy acceptance.

## Phase 11 — GPU execution

**Status:** Complete for the executed CPU + Linux CUDA matrix only.

**Activation prerequisite:** satisfied by the observed external real-product baseline and clean accepted CPU tree.

**Closure provenance:** clean Commit E `411945e0fd53363f98609db21a43d757c4d9b506`, tree `7099dcb5c9879190543d3afa5fde399a84d799df`.

**Closure handoff:** `domain-contracts` owns `ExecutionDevice`; Candle/E0 execute mandatory CPU or explicit opt-in CUDA; E1 owns discovery, selection, persistence, memory policy, admission, and receipt validation; Slint exposes stable presentation-only selection. Support extends only to CUDA ordinal 0 on the executed Linux x86_64 NVIDIA GeForce RTX 5070 Ti matrix with driver 610.43.03, toolkit 13.3, compute capability 12.0, and build target 120. This does not establish generic NVIDIA compatibility. Normal CPU quality run `30942153370` and self-hosted CUDA hardware run `30942148369` were observed successful on the final executable/workflow baseline.

### Objective

The completed objective was to add explicit GPU device support without redesigning E1, weakening CPU behavior, or presenting feature compilation as execution evidence.

### Work package 11.1 — Supported device matrix

**Status:** Complete.

- CPU remains mandatory, default, shared-CI, and covered by executed tests.
- Explicit CUDA ordinal 0 is supported only for the exact executed Linux matrix named above.
- Engine, artifact source, model format, source scalar, execution scalar, and device remain distinct facts.
- Unavailable CUDA fails explicitly without fallback; other NVIDIA/GPU targets are unsupported and unclaimed.

### Work package 11.2 — Build and CI matrix

**Status:** Complete.

- The product feature graph is exactly `desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`.
- The benchmark feature graph is exactly `runtime-benchmarks/cuda -> application-runtime/cuda`.
- The direct E0 test edge `inference-runtime/cuda -> candle-backend/cuda` remains development-only, and no default graph reaches CUDA.
- Final CPU and CUDA compile, test, and Clippy gates passed. The normal shared-CPU quality workflow and separate self-hosted CUDA hardware workflow were both observed successful; feature compilation remains distinct from hardware execution.

### Work package 11.3 — Discovery, admission, and lifecycle

**Status:** Complete.

- Stable device identity, bounded discovery, capability reporting, persisted explicit selection, and accelerator-memory admission are implemented.
- Persisted unavailable CUDA remains visible and fails on load without CPU fallback.
- E0 verifies the actual loaded device, and its receipt carries that identity through E1 to Slint.
- Completion, cancellation, release, unload, synchronization, and bounded shutdown retain explicit ownership.
- Sampling remains host-side over F32 logits after CUDA transfer; GPU-side sampling is deferred.

### Work package 11.4 — E1 and frontend exposure

**Status:** Complete.

- E1 exposes frontend-neutral device summaries and explicit persisted selection.
- Slint remains presentation-only and free of vendor/runtime types.
- The user accepted manual CPU and CUDA Slint operation: both worked, CUDA output was visibly near instant, and no interaction issue was observed. No screenshots were recorded or claimed.

### Work package 11.5 — Executed device evidence

**Status:** Complete.

- The exact supported TinyLlama primary workload executed on CPU and CUDA, including compatible chat, controlled completion, cancellation, release, unload, and shutdown.
- Three complete CUDA lifecycle cycles were stable.
- A direct E0 CUDA snapshot test proved zero model, request, workspace, and cleanup accounting after lifecycle cleanup.
- CUDA adapter and E1 CUDA tests passed, followed by final CPU and CUDA compile/test/Clippy gates.
- Schema-2 chat timing is now recorded in the external evidence reports.
- Raw reports remained beneath ignored root `target/`; exact result tables and limitations are canonical in [performance evidence](../../project/performance.md#external-product-evidence).
- Compile-only, shared-CI, and hardware-executed evidence remain distinct.

### Phase 11 acceptance criteria

- Inference executes on every claimed GPU target.
- CPU behavior remains covered and available.
- Unsupported combinations fail explicitly.
- Device resources are released on completion, cancellation, unload, shutdown, and contract failure.
- UI labels and evidence identify the device actually used.
- No shared-CI wall-clock threshold is introduced without a controlled runner and reviewed policy.

These criteria are accepted only for mandatory CPU and the exact executed Linux CUDA row. Metal, `cudnn`, flash attention, GGUF/quantized formats, GPU-side sampling, multi-GPU, `nccl`, another engine, hosted execution, and peer execution remain unsupported or deferred. One selected/resident model remains the product limit.

## Post-Phase 11 quality maintenance closure

**Status:** Complete. This is an unnumbered maintenance closure, not Phase 12.

The closure reconciled final source/API terminology and E1 module structure, refactored the external evidence implementation and schema without replacing historical measurements, established the trusted download-free CUDA hardware workflow, observed both normal CPU and CUDA Actions acceptance on the final executable/workflow tree, and reconciled canonical documentation. It changed no product behavior and activated no future track.

Commit chronology and run links are canonical in [post-Phase 11 history](history.md#post-phase-11-quality-closure). Current support remains solely in [implementation status](../../project/implementation-status.md), procedures in [validation](../../project/validation.md), and exact measurements in [performance evidence](../../project/performance.md).

## Phase 12 — per-tensor Safetensors compatibility

**Status:** Complete.

Execution authority is the [segmented guide](milkdrift-phase12-execution-guide.md) and its three ownership specifications. The older [monolithic Phase 12 plan](milkdrift-phase12-per-tensor-safetensors-compatibility.md) is retained only as superseded historical planning input.

| Segment | State | Durable/current result |
|---|---|---|
| 1 — core loader/runtime | Complete at `58490fe` | Per-tensor header inspection; supported homogeneous and mixed floating layouts; consumable prepared loads; exact final and loading-peak footprints; E0 plan/receipt validation; retained failed-load ownership and accounting. |
| 2 — artifact/application | Complete at `1251069` | Optional configuration-declared metadata remains distinct from execution facts; immutable artifact resolution and persistence remain compatible; E1 does not reproduce Candle conversion policy; Slint remains thin. |
| 3 — validation/project truth | Complete in this closure commit | Download-free CPU/CUDA fixtures, benchmark observers, synthetic schema 3, external schema 4, CUDA workflow coverage, canonical validation, and project truth reconciled. |

Focused CPU suites passed for the 20-test adapter matrix, 3-test native hosted-E0 suite, 32-test E0 fault suite, and 78-test benchmark package. Fresh-target canonical verification, both portable targets, dependency policy, offline links, and the exact CUDA compile chain also passed on the closure tree.

The implemented CPU matrix is homogeneous `{F32}`, `{F16}`, `{BF16}` plus mixed `{F16, F32}` and `{BF16, F32}`. The exact local CUDA fixture matrix passed on 2026-08-08 on the accepted RTX 5070 Ti row, including adapter, hosted-E0, fault, no-fallback, and E1 lifecycle targets. The Phase 12 GitHub self-hosted workflow remains unrun. F16/BF16 mixtures, integer/unknown tensor layouts, quantized formats, and non-Llama paths remain unsupported.

No suitable immutable, license-reviewed external mixed checkpoint has been established. Pinned TinyLlama is homogeneous BF16 and cannot close that gap. Missing network or credentials would be acquisition failures rather than incompatibility evidence. Under the segmented specification, deterministic compatibility may be documented honestly while this external evidence class remains absent.

## Planned successor — workflow, workspace, and authority

This is the next major program. It remains planned and requires its own reviewed activation boundary:

1. ratify versioned workflow, node, port, artifact, workspace, authority, capability, budget, and endpoint schemas;
2. implement a minimal general workflow runtime and headless host;
3. expose direct completion and correction as configurable public templates;
4. add durable run/workspace provenance and explicit commit authority;
5. prove a second execution-target category through a real implementation rather than treating it as a local E0 backend.

Later inactive tracks include capability-scoped plugins/connectors, provider and peer targets, durable context search and repair, long-lived nodes, multiple replaceable frontends, and a control center over stable public schemas.

## Critical ordering

```text
accepted CPU product
  -> Phase 10 repository infrastructure and synthetic acceptance
  -> executed external real-product baseline
  -> Phase 11 executed CPU + Linux CUDA matrix closure
  -> post-Phase 11 quality maintenance closure
  -> Phase 12 Segment 1 (`58490fe`)
  -> Phase 12 Segment 2 (`1251069`)
  -> Phase 12 Segment 3 and canonical closure (complete)
  -> workflow / workspace / authority foundation (planned, not active)
```

Peer/hosted execution and other later tracks still require their own reviewed evidence triggers; they are not implicit consequences of local CUDA or mixed-layout work.

## Operating rules

1. Start each committed segment from a clean reviewed tree; record commit/tree for accepted boundaries and label current working-tree evidence as dirty until closure.
2. Use the pinned toolchain, committed lockfile, root target, and canonical verification procedure.
3. Preserve public APIs unless a work package explicitly authorizes change.
4. Add deterministic tests for new invariants and reproduced failures.
5. Keep synthetic, component, product, and compile-only evidence distinct.
6. Do not claim allocation freedom, portability, product compatibility, or device execution without a named gate or observed run.
7. Update the canonical owner for changed status, methodology, procedure, or chronology; link instead of copying.
8. Record architectural changes in an ADR.
9. Phase 12 is complete. Keep the workflow/workspace/authority successor and all later tracks inactive until a separate reviewed activation decision.
