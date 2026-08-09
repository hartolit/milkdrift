# ADR-0019: Add explicit feature-gated CUDA execution with application-owned selection

- **Status:** Accepted
- **Date:** 2026-08-03
- **Phase 11 follow-up:** E1/Slint exposure accepted 2026-08-03
- **Amends:** [ADR-0013](0013-candle-only-local-execution.md) for the execution-device dimension
- **Contract amendment:** source/execution scalar separation and accounted-footprint terminology accepted 2026-08-04
- **Evidence acceptance:** product-model CUDA evidence and self-hosted hardware CI accepted 2026-08-04
- **Phase 12 amendment:** [ADR-0020](0020-transactional-prepared-model-loading.md) supersedes this ADR's former homogeneous-source, independent plan/load, E1 loaded-source-scalar, `LAM1` v1-write, and Slint loaded-source-display clauses

## Context

Milkdrift's accepted local product uses Candle and immutable Hugging Face Safetensors. Phase 11 added CUDA below E1 and then required application/frontend exposure without making GPU availability implicit policy, weakening mandatory CPU behavior, coupling model resolution to a machine, adding another engine, or allowing any layer to ignore the requested device.

Candle can compile CUDA support behind Cargo features, but compilation alone does not prove execution. Its convenience `cuda_if_available` operation may return CPU when CUDA is unavailable, which conflicts with an explicit device request. CUDA weights, caches, transfers, synchronization, cleanup, discovery, persistence, UI presentation, and memory admission have different ownership and failure boundaries from CPU execution.

Phase 12 later replaced the homogeneous source-scalar assumption and independent planning/loading calls with exact per-tensor prepared transactions. That amendment changes how scalar/layout and loading ownership are described, but it does not change this ADR's explicit device selection, feature, fallback, or historical hardware-support decision.

## Decision

### Device and feature boundary

The accepted execution and exposure matrix is:

- CPU remains compiled by default, mandatory, and represented as `Cpu` with device ID 0;
- CUDA is a non-default `candle-backend/cuda` implementation feature; product support is narrower than feature compilation;
- the product feature chain is exactly `desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`;
- `inference-runtime/cuda -> candle-backend/cuda` remains a development-only compatibility-test edge;
- no default graph reaches CUDA, no generic `gpu` alias exists, and `cudnn`, `flash-attn`, and `nccl` remain disabled;
- a CUDA `DeviceId` is a backend-visible ordinal, not globally permanent hardware identity;
- enabling CUDA adds capability and does not change an explicit CPU request;
- an explicitly requested CUDA device never falls back to CPU.

The accepted Phase 11 hardware row is CUDA ordinal 0 on Linux x86_64, NVIDIA GeForce RTX 5070 Ti, driver 610.43.03, CUDA Toolkit 13.3, compute capability 12.0, and build target 120. The fixture workflow's toolkit minimum does not broaden product support beyond that observed row.

### Portable and adapter device ownership

`domain-contracts::ExecutionDevice` owns compact `{ id, kind }` identity. No Candle or `cudarc` type crosses the adapter boundary.

Candle device construction is centralized and maps CPU ID 0 to `Device::Cpu` and CUDA ordinal `n` to `Device::new_cuda(n)`. Discovery and prepared loading are bounded cold paths. A complete loaded model retains its device; sequence creation, prefill, decode, destruction, synchronization, and unload reuse that device. No CUDA device or context is created per token. This does not justify unsafe sharing, a singleton, or a wider public API.

Nonzero CPU IDs, unsupported kinds, CUDA in a build without the feature, driver initialization, host/device admission, device execution, and synchronization failures remain distinguishable stable outcomes. The optional direct `cudarc` dependency uses Candle's selected version and safe APIs only for device identity/capability/memory observation and native out-of-memory classification.

### Scalar/layout and prepared-load amendment

[ADR-0020](0020-transactional-prepared-model-loading.md) now owns the durable load contract:

- configuration declaration, complete observed scalar set, required execution-tensor scalar set/primary, and execution scalar are separate facts;
- the exact accepted required sets are `{F32}`, `{F16}`, `{F16,F32}`, `{BF16}`, and `{BF16,F32}`; complete observed extras never select execution;
- required F16+BF16 and required unsupported/quantized tensor types are rejected, while structurally understood unused extras remain observed but are not materialized;
- CPU maps F32→F32, F16→F16, and BF16→F32;
- CUDA policy maps F32→F32, F16→F16, and BF16→BF16 only when the selected device reports support;
- `prepare_load` binds an exact plan to retained source/device state, `load_prepared` consumes it without replanning, and `FailedLoad<PreparedLoad>` preserves partial-load cleanup ownership;
- final ownership and loading-peak headroom are separate deterministic footprints.

E0 reserves the exact loading peak before materialization. On success it verifies complete descriptor, requested/actual device, planned/actual execution scalar, and final planned/actual accounted footprint, then commits the final reservation. On failed materialization or post-load validation, explicit cleanup/quarantine retains the loading peak when cleanup fails. This preserves [ADR-0010](0010-verify-backend-contracts-at-e0.md) and [ADR-0006](0006-explicit-bounded-shutdown.md).

### Sampling and synchronization

Sampling remains in E0 over caller-owned host F32 logits. CUDA logits use Candle's safe device-to-host transfer before sampling. Upstream transfer may allocate a temporary CPU tensor, so Milkdrift does not claim an allocation-free CUDA hot path. Sequence destruction and model unload explicitly synchronize the selected device; dropping CUDA tensors alone is not treated as proof of synchronization.

### E1 selection and loaded facts

E1 owns `ApplicationDevice::{Cpu, Cuda { ordinal: u32 }}`, summaries, compute capability, unavailability, and discovery diagnostics. CPU always exists and is the fresh-install default. Initial bounded discovery probes CUDA 0 and, when different, a persisted selected CUDA ordinal. Structured probe failure leaves persisted CUDA selected and visible. Selection changes only under `can_select_device`; load re-probes, blocks unavailable selection, preserves it, and never falls back.

`ModelSelection` remains repository plus revision. `ResolvedModel` is device-independent and may expose optional configuration-declared scalar metadata. Public E1 `LoadedModel` now exposes the receipt-verified execution scalar and actual execution device but no source/declaration scalar or observed tensor inventory. E1 does not infer execution from device/declaration or reproduce Candle's per-tensor policy. A mismatch publishes no resident model and enters explicit incompatible-receipt cleanup; retained lower ownership emits `ModelCleanupPending`.

This preserves [ADR-0013](0013-candle-only-local-execution.md): E1 remains non-generic, concrete composition remains private, and token-sensitive work remains statically dispatched through the sole Candle local engine.

### Accelerator memory policy

`AcceleratorMemoryPolicy` remains `Automatic` or `Limit { bytes: NonZeroU64 }`. Because E0's aggregate budget is fixed at startup, `Automatic` uses the least reported physical total across every CUDA row in the bounded catalogue; unavailable rows or missing totals contribute zero and fail closed. `Limit` uses the lower safe capacity and user cap.

Before load, E1 re-probes the selected CUDA device and requires the fixed budget to be nonzero and no greater than the latest physical total. Changed capacity that cannot bound it requires restart and returns a structured no-fallback failure. CPU host budgeting is unchanged. Candle preparation separately checks its exact Phase 12 loading peak against remaining budget and current CUDA availability. Host RAM is not accelerator-capacity evidence, and there is no undocumented `u64::MAX` device shortcut.

### Persistence and Slint amendment

`LAS1` application settings continue to write version 2 and read exact version 1. Selected device and memory policy remain persisted; unavailable CUDA is not migrated to CPU.

`LAM1` model catalogue records now write version 2 with optional configuration-declared scalar metadata. Exact version 1 remains readable as a present declaration. Complete observed layout, required primary, execution scalar/device, shard identity, and per-tensor details are not persisted.

Slint remains a thin presentation adapter with stable Rust-owned identity/index mapping. Resolved summaries may show the optional declaration. Loaded summaries show execution scalar and execution device only. Labels are never parsed; Slint does not infer scalar, choose conversion, or fall back.

### Deferred targets

Metal remains domain vocabulary only. Metal execution, cuDNN, flash attention, NCCL, multi-GPU, GPU-side sampling, GGUF/quantization, another engine, and automatic CPU fallback remain unsupported.

## Evidence boundary

The accepted Phase 11 hardware evidence remains attributed to the exact pre-Phase 12 baseline and matrix. It proves the then-current explicit CUDA path, not Phase 12 prepared loading or mixed layouts.

A separate Phase 12 local closure-tree run passed the exact CUDA compile chain and deterministic hardware matrix on 2026-08-08 on the narrowly identified RTX 5070 Ti row. This does not rewrite the historical Phase 11 Actions evidence or establish generic NVIDIA or external mixed-checkpoint compatibility. The Phase 12 GitHub self-hosted workflow has not run; presence of workflow steps is not remote execution evidence. Current evidence truth is canonical in [implementation status](../../project/implementation-status.md).

## Rejected alternatives

- **Use `cuda_if_available`:** rejected because an explicit CUDA request could silently execute on CPU.
- **Enable CUDA by default:** rejected because ordinary CPU builds and CI must not require a CUDA toolkit or driver.
- **Treat a compiled feature as hardware support:** rejected because device execution requires an observed guarded run.
- **Move sampling to CUDA:** rejected because E0's checked host-logit sampling contract remains current.
- **Add a universal accelerator abstraction or generic `gpu` feature:** rejected because only explicit CPU and one concrete CUDA identity are implemented.
- **Put device in `ModelSelection` or `ResolvedModel`:** rejected because artifact resolution and device selection have different identity/persistence/availability lifecycles.
- **Migrate unavailable CUDA to CPU:** rejected because it changes explicit user state and hides failure.
- **Parse Slint labels as identity:** rejected because display text is not semantic state.
- **Infer accelerator budget from host RAM or use `u64::MAX`:** rejected because neither is discovered physical CUDA capacity.
- **Use project-authored unsafe copies:** rejected because safe transfer exists and no Phase 12 requirement justifies widening the unsafe boundary.
- **Retain the old homogeneous-source and independent plan/load clauses:** rejected and superseded by ADR-0020 because they cannot truthfully own mixed per-tensor loading.

## Consequences

- CPU remains mandatory/default and usable in CUDA-enabled binaries.
- CUDA builds remain deliberate and separate from the canonical CPU gate.
- E0 receipts/snapshots identify the actual verified execution device and execution scalar.
- Phase 12 final and loading-peak accounting strengthen admission without changing explicit device policy.
- E1 and Slint preserve explicit selection while exposing only application-relevant resolved and loaded facts.
- Hardware evidence must identify exact commit/tree, driver, toolkit, GPU, compute capability, ordinal, fixture/result, synchronization, and post-unload accounting.
- Historical Phase 11 evidence remains historical; Phase 12 support cannot inherit a hardware run that predates its implementation.

## Review trigger

Review this ADR when changing default/feature graphs, explicit selection/fallback, public device identity, discovery bounds, accelerator budget policy, Candle/cudarc versions, host/GPU sampling placement, synchronization ownership, or claimed CUDA hardware rows.

Review scalar layouts, prepared loading, final/peak formulas, partial-load ownership, E1 scalar facts, or `LAM1` semantics under [ADR-0020](0020-transactional-prepared-model-loading.md). Review terminal retention under ADR-0006, backend verification under ADR-0010, and another local engine under ADR-0013.
