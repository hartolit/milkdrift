# ADR-0019: Add explicit feature-gated CUDA execution with application-owned selection

- **Status:** Accepted
- **Date:** 2026-08-03
- **Phase 11 follow-up:** E1/Slint exposure accepted 2026-08-03

## Context

Milkdrift's accepted local product uses Candle and immutable Hugging Face Safetensors. Phase 11 first added CUDA below E1 and then required application/frontend exposure without making GPU availability an implicit policy, weakening mandatory CPU behavior, coupling model resolution to a machine, adding another engine, or allowing any layer to ignore the requested device.

Candle can compile CUDA support behind Cargo features, but compilation alone does not prove execution. Its convenience `cuda_if_available` operation may return CPU when CUDA is unavailable, which conflicts with an explicit device request. CUDA model weights, sequence caches, transfers, synchronization, cleanup, discovery, persistence, UI presentation, and memory admission also have different ownership and failure boundaries from CPU execution.

## Decision

The accepted Phase 11 execution and exposure matrix is:

- CPU remains compiled by default, mandatory, and represented as `Cpu` with device ID 0;
- CUDA is a non-default `candle-backend/cuda` feature for Linux x86_64 execution;
- the product feature chain is exactly `desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`;
- `inference-runtime/cuda -> candle-backend/cuda` remains a development-only compatibility-test edge;
- no default feature graph reaches CUDA, no generic `gpu` alias exists, and `cudnn`, `flash-attn`, and `nccl` remain disabled;
- a CUDA `DeviceId` is interpreted as a backend-visible ordinal, not a globally permanent hardware identity;
- the first executed target is CUDA ordinal 0 on an NVIDIA GeForce RTX 5070 Ti with compute capability 12.0 and CUDA Toolkit 12.8 or newer;
- enabling CUDA adds capability and does not change an explicit CPU request;
- an explicitly requested CUDA device never falls back to CPU.

`domain-contracts::ExecutionDevice` owns the compact `{ id, kind }` identity. A loaded backend model must report its actual execution device and accepted resident footprint. E0 compares both values with the admitted request and plan before publishing a receipt or resident slot. A mismatch uses the existing transactional cleanup/quarantine path, including retained accounting when explicit cleanup fails.

Candle device construction is centralized and maps CPU ID 0 to `Device::Cpu` and CUDA ordinal `n` to `Device::new_cuda(n)`. Nonzero CPU IDs, unsupported device kinds, CUDA in a CPU-only build, driver initialization failures, host/device budget failures, device execution failures, and synchronization failures remain distinguishable stable adapter outcomes. The optional direct `cudarc` dependency uses Candle's exact selected version and safe APIs only for device name, compute capability, memory discovery, and native out-of-memory classification; no `cudarc` type crosses the adapter boundary.

Source scalar metadata remains independent from execution dtype. BF16-sourced CPU models continue to execute in F32 where Candle requires it. BF16 remains BF16 on a CUDA device that reports support. Model planning charges execution weights to host memory on CPU and device memory on CUDA. Sequence cache and rope working bytes are host working memory on CPU and device working memory on CUDA.

Sampling remains in E0 over caller-owned host `f32` logits. CUDA logits use Candle's safe device-to-host transfer before sampling. That upstream transfer may allocate a temporary CPU tensor, so Milkdrift does not claim an allocation-free CUDA hot path. Sequence destruction and model unload use explicit synchronization; dropping CUDA tensors alone is not treated as successful synchronization.

E1 owns `ApplicationDevice::{Cpu, Cuda { ordinal: u32 }}`, `ApplicationDeviceSummary`, and application-owned compute-capability, unavailability, and discovery diagnostics. No Candle or `cudarc` type crosses the public E1 boundary. CPU always exists and is the fresh-install default. Initial bounded discovery probes CUDA 0 and, when different, a persisted selected CUDA ordinal. Structured probe failure leaves persisted CUDA selected and visible. Selection changes only under E1's `can_select_device` lifecycle policy; load re-probes the selected device, blocks unavailable selection with a structured error, preserves selection, and never falls back.

`ModelSelection` remains repository plus revision. `ResolvedModel` is device-independent and reports only artifacts, source, format, scalar, tokenizer, immutable identity, and compatibility. Selected device is separate application state. `LoadedModel` reports only the actual device from an E0 receipt after E1 validates the ticket, logical model ID and handle, immutable resolution/artifacts, scalar, Llama/Candle/Safetensors evidence, tokenizer vocabulary, selected versus actual device, and bounded footprint. Mismatch publishes no `LoadedModel` and uses the existing private incompatible-model unload/retention path. Unload clears actual loaded-device state and preserves selection.

Accelerator memory configuration is `AcceleratorMemoryPolicy::{Automatic, Limit { bytes: NonZeroU64 }}`. Because E0's aggregate budget is fixed at startup, `Automatic` uses the least reported physical total across every CUDA row in the bounded startup catalogue; an unavailable row or missing total contributes zero and fails closed. `Limit` uses the lower of that safe capacity and the user cap. Before load, E1 re-probes the selected CUDA device and requires the fixed budget to be nonzero and no greater than the latest physical total; changed or newly discovered capacity that cannot bound it requires restart and yields a structured no-fallback error. Existing CPU host budgeting is unchanged. Candle still checks current available VRAM for the selected device before partial residency. Host RAM is not accelerator-capacity evidence, and there is no undocumented `u64::MAX` device shortcut. One resident model remains.

`LAS1` application settings version 2 tags selected CPU/CUDA identity and memory policy. Exact version 1 remains readable, selects CPU, maps zero legacy device bytes to `Automatic`, and maps nonzero bytes to `Limit`; new saves use version 2. A fresh empty default repository is valid. `LAM1` model records remain version 1. Unavailable persisted CUDA is not migrated to CPU.

Slint presents a compact device `ComboBox` backed by stable Rust identity/index mapping. It gives unavailable devices a distinct label, derives selection/load enabled state from E1, and keeps selected-device, artifact-only resolved-model, and actual-device loaded-model summaries distinct. Labels are never parsed for semantics, and the frontend never falls back.

Metal remains domain vocabulary only. Metal execution, `cudnn`, `flash-attn`, `nccl`, multi-GPU distribution, GPU-side sampling, GGUF/quantization, and automatic CPU fallback are deferred.

## Rejected alternatives

- **Use `cuda_if_available`:** rejected because an explicit CUDA request could silently execute on CPU.
- **Enable CUDA by default:** rejected because ordinary CPU builds and CI must not require a CUDA toolkit or driver.
- **Treat a compiled feature as support evidence:** rejected because the supported target requires an ignored, explicitly opted-in hardware execution test.
- **Move sampling to CUDA now:** rejected because Phase 11 first preserves E0's existing checked host-logit sampling contract.
- **Add a universal accelerator abstraction or generic `gpu` feature:** rejected because only CPU and one concrete CUDA path have implementation evidence.
- **Put the device in `ModelSelection` or `ResolvedModel`:** rejected because artifact resolution and execution-device selection have different identity, persistence, and availability lifecycles.
- **Migrate unavailable persisted CUDA to CPU or infer CPU fallback:** rejected because it changes explicit user state and can hide an execution failure.
- **Parse Slint labels as device identity:** rejected because display text is not a stable semantic contract.
- **Infer accelerator budget from host RAM or use `u64::MAX`:** rejected because neither is discovered physical CUDA capacity.
- **Use project-owned unsafe copies:** rejected because Candle provides a safe transfer boundary, even though it may allocate upstream temporary storage.

## Consequences

- CPU remains the mandatory default and remains usable in CUDA-enabled binaries.
- CUDA builds are deliberate and separate from the canonical CPU gate.
- E0 receipts and snapshots identify the actual verified execution device.
- Host and device reservations remain exact to the accepted plans and survive cleanup failure.
- Hardware evidence must state the driver, toolkit, GPU, compute capability, selected ordinal, fixture result, synchronization result, and post-unload accounting.
- E1 and Slint expose explicit CPU/CUDA selection while preserving CPU as the fresh-install default and preserving explicit unavailable CUDA state without fallback.
- External/product-model CUDA execution evidence and measurements are still required before Phase 11 can complete; this decision does not claim a new hardware/manual run.

## Review trigger

Review when adding another CUDA hardware target, changing public device identity or discovery bounds, coupling resolution to execution device, changing Candle/cudarc versions, moving sampling or transfers onto the GPU, implementing Metal, enabling cuDNN/flash attention/NCCL, or pursuing multi-GPU execution.
