# Current implementation status

**Status date:** 2026-08-10

```text
Phase 10 complete.
Phase 11 complete for its historically executed CPU + Linux CUDA matrix.
Post-Phase 11 quality closure complete.
Phase 12 complete for its 2026-08-08 deterministic CPU and exact local CUDA
closure matrix.
The 2026-08-10 pristine artifact-loading amendment passed targeted CPU quality,
the exact CUDA compile graph, and the complete local CUDA hardware matrix.
The amended tree has not run the complete canonical/portable/policy/link gate or
a GitHub self-hosted workflow, and external mixed-checkpoint evidence is absent.
No subsequent product phase is active.
```

This page is the sole product-level support matrix and validation-state owner. It separates implementation, focused working-tree validation, clean-tree acceptance, compile-only coverage, and hardware execution. It does not turn earlier evidence into evidence for later code. Repeatable procedures belong in [validation](validation.md), measurements in [performance evidence](performance.md), and historical chronology in [execution history](../agent/execution/history.md).

Milkdrift remains workflow-first: the local Candle endpoint is one lifecycle-safe execution target for future operator-defined workflows, not the project identity. The current `application-runtime` and Slint desktop are a reference application kit and thin host, not the future general workflow API or control plane.

## Phase 12 implementation baseline

Phase 12 production work is split across:

- commit `58490fe693fef7a2635956181088664cd90685e8`, which introduced exact prepared loading, per-tensor Safetensors inspection and conversion, conversion-aware final/loading-peak accounting, and retained partial-load cleanup through E0;
- commit `12510695aa29be6a2665dbf3777cccbb8172c2d1`, which integrated optional configuration metadata, E1 receipt semantics, persistence versioning, cleanup events, and thin Slint presentation;
- the 2026-08-08 closure segment, which broadened deterministic mixed-layout adapter/E0 coverage, updated the inward benchmark observer and guarded CUDA workflow, and reconciled canonical evidence owners;
- the 2026-08-10 pristine artifact-loading amendment, which adds strict declaration truth, complete-observed versus required scalar separation, selective materialization, whole-shard identity authorities, required-only footprints, structural metadata bounds, and the internal loader module split.

On 2026-08-08, the Phase 12 closure tree passed the focused download-free CPU suites, the canonical gate from a previously absent Cargo target directory, both portable-domain target matrices, locked dependency policy, offline Markdown links, the exact CUDA compile chain, and the exact local deterministic hardware matrix. On 2026-08-10, the artifact-loading amendment separately passed its targeted CPU quality commands, exact CUDA compile graph, and complete local hardware matrix; it did not rerun the full canonical/portable/policy/link gate. Both hardware runs were local rather than GitHub Actions. No external mixed-dtype checkpoint has been accepted.

## Supported product

| Capability | Current implementation and evidence boundary |
|---|---|
| Project identity | Operator-defined workflows, explicit context/authority/resource ownership, and replaceable execution targets; the current local inference application is the implemented foundation. |
| Local engine | Candle only. |
| Artifact source | Hugging Face Hub revision resolved to an immutable commit. Selected LFS shards carry exact provider SHA-256/length identity; non-LFS and arbitrary local paths use explicit mutable-source fallback semantics. |
| Model format and architecture | Unquantized Safetensors through the current Llama compatibility path. All selected structure is inspected; only required Llama tensors are materialized. |
| Required scalar layouts | Exactly `{F32}`, `{F16}`, `{F16,F32}`, `{BF16}`, and `{BF16,F32}` under the declaration rules below. Understood unused extras may add complete observed categories without changing execution. |
| CPU | Mandatory in every build and the fresh-install/default selection. The amended targeted suites passed locally; the complete canonical gate remains historical to the 2026-08-08 closure tree. |
| CUDA | Non-default explicit ordinal 0 remains the only product CUDA identity. The amended exact compile chain and complete deterministic hardware matrix passed locally on RTX 5070 Ti ordinal 0. No GitHub self-hosted, generic NVIDIA, or external mixed-checkpoint claim is made. |
| Device failure | Explicit failure with no automatic CPU fallback. |
| Resident models | One selected/resident model in E1. |
| Direct completion | Available for every successfully loaded compatible model. |
| Built-in chat | Exact `TinyLlama/TinyLlama-1.1B-Chat-v1.0` profile at commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`, with `</s>` token ID 2. Loading another compatible model does not imply chat support. |
| Frontend | Thin Slint desktop through the E1 façade. |
| Persistence | redb-backed preferences and model catalogue; conversation history remains in memory. |

The complete current composition remains:

```text
Slint or another native host
        -> application-runtime (E1 reference application kit)
             -> one bounded Hub worker
             -> Hugging Face tokenizer/decoder
             -> redb persistence
             -> one Candle hosted E0 worker/thread
                  -> inference-runtime (E0)
                       -> Candle + unquantized Llama Safetensors
                       -> mandatory/default CPU
                          or explicit feature-gated CUDA ordinal 0
```

## Scalar meanings and declaration truth

The loader uses four separate scalar facts:

1. **Configuration declaration** is optional recognized F32/F16/BF16 producer intent derived from bounded `config.json` bytes. `dtype` is modern and never silently falls back when present but unsupported. Equal recognized fields agree; conflicting, unsupported, duplicated, malformed, or wrong-typed fields fail explicitly.
2. **Complete observed set** is the fixed-size `ScalarTypeSet` built from every structurally valid tensor header across every selected shard, including unused extras.
3. **Required set and primary** are adapter-private compatibility facts derived only from tensors consumed by the supported Llama schema.
4. **Execution scalar** is selected during exact device-aware preparation and verified from the loaded backend model by E0.

These facts must not collapse into one “source scalar.” Detailed names, shapes, offsets, per-tensor dtypes, required classification, and shard identities remain private to `candle-backend`.

## Exact required-layout policy

| Required tensor scalar set | Required primary | Permitted declaration | CPU execution | Supported CUDA planner policy |
|---|---|---|---|---|
| `{F32}` | F32 | absent or F32 | F32 | F32 |
| `{F16}` | F16 | absent or F16 | F16 | F16 |
| `{F16,F32}` | F16 | absent or F16 | F16 | F16 |
| `{BF16}` | BF16 | absent or BF16 | F32 | BF16 when supported; otherwise planning fails |
| `{BF16,F32}` | BF16 | absent or BF16 | F32 | BF16 when supported; otherwise planning fails |

A genuine required F16+BF16 mixture, empty required set, required unsupported dtype, or quantized required representation is rejected. Structurally understood unused integer, boolean, FP8/bit-packed, wider floating/integer, complex, or other tensors remain complete observed evidence but are never materialized and do not affect declarations, execution, or footprints. Final vocabulary logits return to E0 as host F32 and sampling remains host-side.

## Prepared loading and memory truth

`ModelLoader::prepare_load` creates one opaque source/configuration/device-bound preparation and exact plan; `load_prepared` consumes it without replanning. Before device initialization, Candle validates every selected header, complete dtype evidence, duplicate names, contiguous offsets, bounds, ranks/dimensions, required Llama schema, and checked byte arithmetic under explicit shard/header/tensor/name/shape/metadata/inventory ceilings.

Every shard path remains paired with identity authority. Verified immutable identity currently means exact Hugging Face LFS SHA-256 and length at the resolved commit and skips a pre-admission payload pass. Project-established and unverified mutable sources are sequentially hashed from Candle's retained open file before admission. Materialization then processes each retained shard once from byte zero, verifies its header, hashes ignored ranges through a fixed 64 KiB buffer, stages only required ranges, and checks exact EOF/length and whole-shard SHA-256 before model publication. There are no per-tensor seeks/digests, mmap, unsafe code, or whole-model host buffers.

Two deterministic footprint phases are distinct:

- **Final footprint** is exact post-load required execution-tensor ownership plus cache bytes per token.
- **Loading peak** is the exact required-only peak for aligned source staging, source tensor, optional cast tensor, required CUDA transfer, and already retained required weights. Ignored extras contribute no host/device tensor headroom. The fixed verification buffer and parsed metadata are independently bounded rather than hidden inside `MemoryFootprint`.

The exact formulas are canonical in [Candle backend](candle-backend.md). E0 admits both the peak and final quantities against its aggregate budget, reserves the peak before materialization, and commits only the final quantity after complete model verification.

## Load failure and cleanup ownership

A failed materialization returns `FailedLoad<PreparedLoad>`: the primary `LoadError` and the sole owner of every completed or pending tensor/device resource. E0 immediately attempts `PreparedLoad::cleanup`.

- Cleanup success restores the pre-load reservation and returns the original load failure.
- Cleanup failure moves the preparation into E0 pending-model cleanup as `CleanupResource::FailedLoad`, retains the complete loading-peak footprint, and reports primary model-load failure separately from failed-load cleanup failure.
- The initial failed cleanup is attempt one. `poll_cleanup` performs at most one additional retained operation, uses the configured finite total-attempt budget (three by default), releases ownership/accounting exactly once on success, and leaves exhausted ownership quarantined and accounted.
- A complete loaded model that fails E0 post-load verification follows the same no-publication rule and retains the conservative loading-peak quantity if explicit unload preparation fails.

E1 publishes no resident `LoadedModel` for any such failure. `ApplicationEvent::ModelCleanupPending { exhausted, failure }` distinguishes retained cleanup from an ordinary owner-free `ModelLoadFailed`; application activity remains unloading and device selection remains locked until a private E0 snapshot proves zero aggregate ownership, disconnection/confirmed worker stop resolves ownership, or exhaustion remains explicit.

These semantics extend rather than replace [ADR-0006](../agent/decisions/0006-explicit-bounded-shutdown.md): terminal shutdown exhaustion still uses `RetainUntilProcessExit`, and process termination remains the final reclamation boundary for deliberately retained native ownership.

## E1, persistence, and Slint boundary

`ResolvedModel` remains device-independent and exposes resolved identity, tokenizer/compatibility facts, and the optional configuration declaration. It does not expose a complete observed set, required primary, execution scalar, or actual device.

Public E1 `LoadedModel` exposes the receipt-verified execution scalar and execution device, but no configuration/source scalar or observed tensor inventory. E1 validates declaration agreement across resolution/admission/descriptor evidence and requires a nonempty complete observed set; it does not reject integer/`Other` evidence from ignored tensors, compare declarations to individual tensors, infer required primary, choose conversions, or reproduce Candle's device policy.

`LAM1` catalogue writes are version 2 and store the optional configuration declaration only. Exact version 1 records remain readable, with their mandatory scalar decoded in memory as `Some(...)`. Observed layouts, required primary, execution scalar/device, cache paths, shard identities, and per-tensor details are not persisted. `LAS1` settings remain version 2 writes/version 1 reads under the existing device and accelerator-memory policy.

Slint remains a thin reference host. Resolved summaries may show the optional configuration declaration; loaded summaries show only verified execution scalar and execution device. Labels are presentation, never parsed identity, and the frontend owns no model-loading or fallback policy.

## Validation and evidence state

| Evidence class | Current state on 2026-08-10 |
|---|---|
| Production implementation | Original Phase 12 in commits `58490fe` and `1251069`; pristine artifact-loading amendment present in the current coherent change. |
| Amended focused/download-free CPU quality | Passed: formatting; all-target checks; 24 Candle unit + 1 fixture-consistency + 25 Candle CPU integration tests; 23 Hub tests; 3 hosted native-E0 tests; 79 application tests; 78 benchmark tests; strict all-target Clippy; warning-denied rustdoc. |
| Amended CUDA compile graph | Metadata, architecture, hygiene, six-package all-target check/Clippy as applicable, and four-package test compilation passed with `CUDA_COMPUTE_CAP=120`. |
| Amended local CUDA hardware execution | Passed explicit CPU-in-CUDA, exactly four adapter tests, mixed hosted-E0 lifecycle, 32 E0 faults, E1 no-fallback, and E1 CUDA lifecycle on NVIDIA GeForce RTX 5070 Ti ordinal 0, driver/KMD 610.43.03, CUDA UMD/toolkit 13.3, `nvcc` 13.3.73, compute capability 12.0, build cap 120. |
| Amended complete canonical/portable/policy/link gate | Not run. The 2026-08-08 Phase 12 closure result remains historical and is not reused as amended-tree evidence. |
| GitHub self-hosted workflow | Not run for Phase 12 or the amendment; no remote workflow provenance is claimed. |
| External mixed-dtype checkpoint | Absent. No immutable, license-reviewed external mixed checkpoint has been accepted or executed. |
| Benchmark/evidence additions | Inward observer compilation/tests passed; no new performance measurement or external product report is claimed. |

### Historical Phase 11 evidence

The accepted pre-Phase 12 hardware-executed source baseline remains commit `1a62d2ed6623500e9052b4b8386ebd058984bd89`, tree `79864da274aed94471c2fbcfedaa97c2f32f3e7a`.

- shared-CPU [quality run 30942153370](https://github.com/hartolit/milkdrift/actions/runs/30942153370) passed on that baseline;
- self-hosted [CUDA hardware run 30942148369](https://github.com/hartolit/milkdrift/actions/runs/30942148369) passed on that baseline for Linux x86_64, NVIDIA GeForce RTX 5070 Ti, driver 610.43.03, CUDA Toolkit 13.3, compute capability 12.0, ordinal 0, and `CUDA_COMPUTE_CAP=120`.

That evidence remains historical proof of the Phase 11 device path only. It is not evidence that the Phase 12 per-tensor implementation, mixed layouts, new peak accounting, or retained prepared-load cleanup executed on CUDA hardware.

Historical external TinyLlama CPU/CUDA evidence remains attributed to clean Commit E `411945e0fd53363f98609db21a43d757c4d9b506`, tree `7099dcb5c9879190543d3afa5fde399a84d799df`. TinyLlama at the pinned revision is homogeneous BF16, not an external mixed-dtype checkpoint, and cannot close the Phase 12 external evidence gap.

## Evidence infrastructure

`runtime-benchmarks` remains the sole non-production cross-crate observer. No production, tooling, test, or application package depends on it. The closure tree updates its synthetic and external schemas to distinguish optional configuration declaration, observed scalar set, planned execution scalar/device, exact final footprint, loading peak, actual receipt facts where public, process RSS, and whole-device CUDA observations. It uses an unmaterialized observer `prepare_load`, copies the public plan, and drops the preparation without turning benchmark needs into a new production API.

Historical reports retain their original schema meanings and measurements. No Phase 12 performance result or external checkpoint/product report is inferred merely because observer or workflow definitions changed. The separately executed local CUDA compile and hardware-correctness results are recorded above.

## Unsupported behavior

- Arbitrary required mixed-dtype layouts, required F16+BF16, required FP8/integer/boolean/unknown tensors, quantized weights, GGUF, and non-Llama architectures are unsupported. Structurally understood unused extras are permitted but never executed.
- CUDA outside the exact locally executed RTX 5070 Ti amendment matrix, generic NVIDIA compatibility, generic `gpu`, automatic CPU fallback, cuDNN, flash attention, multi-GPU, NCCL, and GPU-side sampling are unsupported.
- Metal is not implemented.
- Another local engine, hosted-provider execution, peer execution, general workflow execution, plugin execution, and remote/browser transport are not current product paths.
- Multi-model E1 residency is unsupported.
- Chat compatibility is not generalized beyond the exact reviewed TinyLlama profile.
- Conversation persistence, arbitrary branch trees, and a Slint generation-settings panel are not implemented.
- Strict allocation freedom is not claimed for Candle or Hugging Face tokenization/decoding.
- Synthetic fixtures establish deterministic compatibility and lifecycle behavior, not language quality, representative scale, production throughput, or external-checkpoint compatibility.

## Current closure boundary

The pristine artifact-loading amendment is complete for targeted deterministic CPU quality and the exact locally executed CUDA matrix. A clean committed-tree canonical/portable/policy/link run and GitHub self-hosted run remain unexecuted evidence classes, and external mixed-checkpoint evidence remains explicitly absent rather than being replaced with homogeneous TinyLlama or a fabricated claim.

The next major architectural direction returns to workflow/workspace/authority foundations. Phase 12 strengthens one local endpoint and does not authorize indefinite expansion of the Candle loader or promotion of the current application kit into the workflow core.
