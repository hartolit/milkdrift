# ADR-0019: Add explicit feature-gated CUDA execution below E1

- **Status:** Accepted
- **Date:** 2026-08-03

## Context

Milkdrift's accepted local product uses Candle, Safetensors, and CPU execution. Phase 11 needs a first GPU path without making GPU availability an implicit runtime policy, weakening the mandatory CPU path, adding another engine, or allowing a backend to ignore the device requested by E0.

Candle can compile CUDA support behind Cargo features, but compilation alone does not prove execution. Its convenience `cuda_if_available` operation may return CPU when CUDA is unavailable, which conflicts with an explicit device request. CUDA model weights, sequence caches, transfers, synchronization, and cleanup also have different accounting and failure boundaries from CPU execution.

## Decision

The initial Phase 11 lower-layer matrix is:

- CPU remains compiled by default, mandatory, and represented as `Cpu` with device ID 0;
- CUDA is a non-default `candle-backend/cuda` feature for Linux x86_64 execution;
- a CUDA `DeviceId` is interpreted as a backend-visible ordinal, not a globally permanent hardware identity;
- the first executed target is CUDA ordinal 0 on an NVIDIA GeForce RTX 5070 Ti with compute capability 12.0 and CUDA Toolkit 12.8 or newer;
- enabling CUDA adds capability and does not change an explicit CPU request;
- an explicitly requested CUDA device never falls back to CPU.

`domain-contracts::ExecutionDevice` owns the compact `{ id, kind }` identity. A loaded backend model must report its actual execution device and accepted resident footprint. E0 compares both values with the admitted request and plan before publishing a receipt or resident slot. A mismatch uses the existing transactional cleanup/quarantine path, including retained accounting when explicit cleanup fails.

Candle device construction is centralized and maps CPU ID 0 to `Device::Cpu` and CUDA ordinal `n` to `Device::new_cuda(n)`. Nonzero CPU IDs, unsupported device kinds, CUDA in a CPU-only build, driver initialization failures, host/device budget failures, device execution failures, and synchronization failures remain distinguishable stable adapter outcomes. The optional direct `cudarc` dependency uses Candle's exact selected version and safe APIs only for device name, compute capability, memory discovery, and native out-of-memory classification; no `cudarc` type crosses the adapter boundary.

Source scalar metadata remains independent from execution dtype. BF16-sourced CPU models continue to execute in F32 where Candle requires it. BF16 remains BF16 on a CUDA device that reports support. Model planning charges execution weights to host memory on CPU and device memory on CUDA. Sequence cache and rope working bytes are host working memory on CPU and device working memory on CUDA.

Sampling remains in E0 over caller-owned host `f32` logits. CUDA logits use Candle's safe device-to-host transfer before sampling. That upstream transfer may allocate a temporary CPU tensor, so Milkdrift does not claim an allocation-free CUDA hot path. Sequence destruction and model unload use explicit synchronization; dropping CUDA tensors alone is not treated as successful synchronization.

Metal remains domain vocabulary only. Metal execution, cuDNN, flash attention, NCCL, multi-GPU distribution, GPU-side sampling, GGUF/quantization, and automatic CPU fallback are deferred.

## Rejected alternatives

- **Use `cuda_if_available`:** rejected because an explicit CUDA request could silently execute on CPU.
- **Enable CUDA by default:** rejected because ordinary CPU builds and CI must not require a CUDA toolkit or driver.
- **Treat a compiled feature as support evidence:** rejected because the supported target requires an ignored, explicitly opted-in hardware execution test.
- **Move sampling to CUDA now:** rejected because Phase 11 first preserves E0's existing checked host-logit sampling contract.
- **Add a universal accelerator abstraction:** rejected because only CPU and one concrete CUDA path have implementation evidence.
- **Use project-owned unsafe copies:** rejected because Candle provides a safe transfer boundary, even though it may allocate upstream temporary storage.

## Consequences

- CPU remains the mandatory default and remains usable in CUDA-enabled binaries.
- CUDA builds are deliberate and separate from the canonical CPU gate.
- E0 receipts and snapshots identify the actual verified execution device.
- Host and device reservations remain exact to the accepted plans and survive cleanup failure.
- Hardware evidence must state the driver, toolkit, GPU, compute capability, selected ordinal, fixture result, synchronization result, and post-unload accounting.
- E1 and frontends still select CPU until a separate Phase 11 session adds application-owned discovery, selection, persistence, and presentation.

## Review trigger

Review when adding another CUDA hardware target, exposing device selection through E1, changing Candle/cudarc versions, moving sampling or transfers onto the GPU, implementing Metal, enabling cuDNN/flash attention/NCCL, or pursuing multi-GPU execution.
