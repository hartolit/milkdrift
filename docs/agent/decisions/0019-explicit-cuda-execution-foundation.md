# ADR-0019: Add explicit feature-gated CUDA execution with application-owned selection

- **Status:** Accepted
- **Date:** 2026-08-03
- **Phase 11 follow-up:** E1/Slint exposure accepted 2026-08-03
- **Amends:** [ADR-0013](0013-candle-only-local-execution.md) for the execution-device dimension
- **Contract amendment:** source/execution scalar separation and accounted-footprint terminology accepted 2026-08-04
- **Evidence acceptance:** product-model CUDA evidence and self-hosted hardware CI accepted 2026-08-04

## Context

Milkdrift's accepted local product uses Candle and immutable Hugging Face Safetensors. Phase 11 first added CUDA below E1 and then required application/frontend exposure without making GPU availability an implicit policy, weakening mandatory CPU behavior, coupling model resolution to a machine, adding another engine, or allowing any layer to ignore the requested device.

Candle can compile CUDA support behind Cargo features, but compilation alone does not prove execution. Its convenience `cuda_if_available` operation may return CPU when CUDA is unavailable, which conflicts with an explicit device request. CUDA model weights, sequence caches, transfers, synchronization, cleanup, discovery, persistence, UI presentation, and memory admission also have different ownership and failure boundaries from CPU execution.

## Decision

The accepted Phase 11 execution and exposure matrix is:

- CPU remains compiled by default, mandatory, and represented as `Cpu` with device ID 0;
- CUDA is a non-default `candle-backend/cuda` implementation feature; accepted product support is narrower than feature compilation;
- the product feature chain is exactly `desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`;
- `inference-runtime/cuda -> candle-backend/cuda` remains a development-only compatibility-test edge;
- no default feature graph reaches CUDA, no generic `gpu` alias exists, and `cudnn`, `flash-attn`, and `nccl` remain disabled;
- a CUDA `DeviceId` is interpreted as a backend-visible ordinal, not a globally permanent hardware identity;
- accepted product support is CUDA ordinal 0 only on the executed Linux x86_64 NVIDIA GeForce RTX 5070 Ti row with driver 610.43.03, CUDA Toolkit 13.3, compute capability 12.0, and build target 120; the fixture workflow enforces Toolkit 12.8 or newer but does not broaden product support beyond the observed 13.3 row;
- enabling CUDA adds capability and does not change an explicit CPU request;
- an explicitly requested CUDA device never falls back to CPU.

`domain-contracts::ExecutionDevice` owns the compact `{ id, kind }` identity. `ModelDescriptor::metadata.scalar_type` remains immutable source scalar metadata. `LoadPlan::execution_scalar_type` records the scalar selected by device-aware planning, and a loaded backend model must report its actual execution device, actual execution scalar, and accounted footprint. E0 compares the handle, complete descriptor, requested versus actual device, planned versus actual execution scalar, and planned versus actual accounted footprint before completing the lifecycle transition or publishing a receipt or resident slot. A mismatch uses the existing transactional cleanup/quarantine path, including complete reserved footprint retention when explicit cleanup fails. The accounted footprint is the quantity E0 admits, reserves, and restores transactionally; physical memory observation remains separate OS/driver evidence.

Candle device construction is centralized and maps CPU ID 0 to `Device::Cpu` and CUDA ordinal `n` to `Device::new_cuda(n)`. Discovery, planning, and loading are bounded cold paths that each initialize and probe independently; only loading retains the Candle device. Sequence creation, prefill, decode, destruction, synchronization, and unload reuse that retained device, so no CUDA device or direct context is created per token or decode step. This bounded behavior does not justify a cache, singleton, unsafe sharing, or a wider public API. Nonzero CPU IDs, unsupported device kinds, an explicit CUDA request in a build without the CUDA feature, driver initialization failures, host/device budget failures, device execution failures, and synchronization failures remain distinguishable stable adapter outcomes. The optional direct `cudarc` dependency uses Candle's exact selected version and safe APIs only for device name, compute capability, physical memory observation, and native out-of-memory classification; no `cudarc` type crosses the adapter boundary.

Source scalar metadata remains independent from execution scalar. F32 sources execute as F32 on CPU and CUDA. F16 sources execute as F16 where the current Candle path supports them. BF16-sourced CPU models execute in F32 where Candle requires it, while BF16 remains BF16 on a CUDA device that reports support. Unsupported combinations fail during planning before partial residency. Model planning derives weight and cache accounting from the execution scalar, charges execution weights to host memory on CPU and device memory on CUDA, and charges sequence cache and rope working bytes to the corresponding execution-memory domain. No `candle_core::DType` crosses the adapter boundary.

Sampling remains in E0 over caller-owned host `f32` logits. CUDA logits use Candle's safe device-to-host transfer before sampling. That upstream transfer may allocate a temporary CPU tensor, so Milkdrift does not claim an allocation-free CUDA hot path. Sequence destruction and model unload use explicit synchronization; dropping CUDA tensors alone is not treated as successful synchronization.

E1 owns `ApplicationDevice::{Cpu, Cuda { ordinal: u32 }}`, `ApplicationDeviceSummary`, and application-owned compute-capability, unavailability, and discovery diagnostics. No Candle or `cudarc` type crosses the public E1 boundary. CPU always exists and is the fresh-install default. Initial bounded discovery probes CUDA 0 and, when different, a persisted selected CUDA ordinal. Structured probe failure leaves persisted CUDA selected and visible. Selection changes only under E1's `can_select_device` lifecycle policy; load re-probes the selected device, blocks unavailable selection with a structured error, preserves selection, and never falls back.

`ModelSelection` remains repository plus revision. `ResolvedModel` is device-independent and reports only artifacts, source, format, source scalar, tokenizer, immutable identity, and compatibility. Selected device is separate application state. `LoadedModel` reports source scalar, receipt-verified execution scalar, and the actual loaded device after E1 validates the ticket, logical model ID and handle, immutable resolution/artifacts, coherent supported scalar evidence, Llama/Candle/Safetensors evidence, tokenizer vocabulary, selected versus requested and actual device, and bounded reserved footprint. E1 does not infer scalar from device or reproduce Candle's device-aware planner. Mismatch publishes no `LoadedModel` and uses the existing private incompatible-model unload/retention path. Unload clears loaded scalar and actual-device facts while preserving resolved source evidence and selected device.

Accelerator memory configuration is `AcceleratorMemoryPolicy::{Automatic, Limit { bytes: NonZeroU64 }}`. Because E0's aggregate budget is fixed at startup, `Automatic` uses the least reported physical total across every CUDA row in the bounded startup catalogue; an unavailable row or missing total contributes zero and fails closed. `Limit` uses the lower of that safe capacity and the user cap. Before load, E1 re-probes the selected CUDA device and requires the fixed budget to be nonzero and no greater than the latest physical total; changed or newly discovered capacity that cannot bound it requires restart and yields a structured no-fallback error. Existing CPU host budgeting is unchanged. Candle still checks current available VRAM for the selected device before partial residency. Host RAM is not accelerator-capacity evidence, and there is no undocumented `u64::MAX` device shortcut. One resident model remains.

`LAS1` application settings version 2 tags selected CPU/CUDA identity and memory policy. Exact version 1 remains readable, selects CPU, maps zero legacy device bytes to `Automatic`, and maps nonzero bytes to `Limit`; new saves use version 2. A fresh empty default repository is valid. `LAM1` model records remain version 1 and persist source scalar only. Execution scalar is runtime evidence and is not persisted. Unavailable persisted CUDA is not migrated to CPU.

Slint presents a compact device `ComboBox` backed by stable Rust identity/index mapping. It gives unavailable devices a distinct label, derives selection/load enabled state from E1, and keeps selected device, artifact-only resolved source scalar, loaded source/execution scalars, and actual loaded device distinct. Resolved summaries show source scalar only; loaded summaries show source scalar, execution scalar, and actual device. Labels are never parsed for semantics, scalar is never inferred from device, and the frontend never falls back.

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
- E0 receipts and snapshots identify the actual verified execution device and execution scalar.
- Backend models report an accounted footprint rather than claiming a physical measurement; E0 reserved footprints remain exact to accepted plans and survive cleanup failure.
- Hardware evidence must state the driver, toolkit, GPU, compute capability, selected ordinal, fixture result, synchronization result, and post-unload accounting.
- E1 and Slint expose explicit CPU/CUDA selection plus separate source scalar, execution scalar, and actual loaded-device facts while preserving CPU as the fresh-install default and preserving explicit unavailable CUDA state without fallback.
- The accepted real-model CPU/CUDA evidence remains attributed to clean Commit E and its original schema-2 facts. The amended executable/workflow tree subsequently passed observed normal CPU quality run `30942153370` and self-hosted CUDA hardware run `30942148369`; schema-3 refactoring did not rerun or replace the historical timing tables.

## Review trigger

Review when changing the source/execution scalar contract, accounted footprint or reserved footprint semantics, CUDA cold-path initialization ownership, adding another CUDA hardware target, changing public device identity or discovery bounds, coupling resolution to execution device, changing Candle/cudarc versions, moving sampling or transfers onto the GPU, implementing Metal, enabling cuDNN/flash attention/NCCL, or pursuing multi-GPU execution.
