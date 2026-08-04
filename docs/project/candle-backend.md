# Candle CPU-default and CUDA-capable reference backend

## Scope

`crates/adapters/candle-backend` is the sole current local execution adapter. It implements the `domain-contracts` backend boundary for unquantized Hugging Face Llama configuration files and one or more Safetensors weight shards. CPU is compiled by default. Explicit CUDA ordinals are supported behind the non-default `candle-backend/cuda` feature and never fall back to CPU.

The adapter owns every Candle and `cudarc` type. No native tensor, device, model, cache, context, dtype, or error crosses into `domain-contracts`. Artifact download and tokenizer integration remain separate adapters.

## Scalar contract

Scalar identity has two distinct stages:

- `ModelDescriptor::metadata.scalar_type` is immutable source metadata and reports the scalar stored in the weight tensors;
- `LoadPlan::execution_scalar_type` is selected during device-aware planning;
- `LoadedModel::execution_scalar_type()` is the actual domain `ScalarType` retained as loaded execution evidence.

Candle's `DType` remains private to the adapter. The exact policy is:

| Source scalar | CPU execution | CUDA execution |
| --- | --- | --- |
| F32 | F32 | F32 |
| F16 | F16 | F16 |
| BF16 | F32 | BF16 only when the selected device reports support |

Unsupported BF16 CUDA execution fails during the cold planning/loading preparation step, before any model weight shard is loaded. Vocabulary logits are normalized to caller-owned host F32 storage for every supported source and execution scalar.

## Accounting and physical memory observation

`MemoryFootprint`, `ModelDescriptor::estimated_footprint`, `LoadPlan::expected_footprint`, and `LoadedModel::accounted_footprint()` are planning and accounting quantities. They are not measurements of physical memory currently allocated by Candle or available on a device.

Inspection remains device-independent and uses the mandatory CPU execution policy for `ModelDescriptor::estimated_footprint`. Device-aware planning computes execution weight bytes and cache bytes per token from the selected execution scalar:

- CPU execution weights are host weight accounting;
- CUDA execution weights are device weight accounting;
- the largest source shard is host loading headroom;
- sequence cache and rope bytes are host working accounting on CPU and device working accounting on CUDA.

After load, `accounted_footprint()` reports the complete quantity accepted from the load plan, including retained accounting for loading headroom. Physical CUDA capacity and moment-in-time availability are separate driver observations in `CandleDeviceSummary::{total_memory_bytes, available_memory_bytes}`. Current availability is checked before partial model residency but is not substituted for the configured accounting budget.

## Device initialization lifecycle

The loader has independent cold-path initialization boundaries:

1. `inspect` reads configuration and weight-file metadata without constructing an execution device;
2. `discover_device` independently constructs the explicitly requested device and returns observed device facts;
3. `plan_load` inspects the source, independently initializes the requested device, selects the execution scalar, checks host/device accounting budgets and current CUDA availability, returns a portable `LoadPlan`, and drops the prepared device;
4. `load` repeats source validation and device initialization, selects the execution scalar again, validates accounting again, then retains the newly prepared Candle device while loading weights and constructing the Llama model.

For CUDA, each discovery, planning, or loading call invokes the centralized `Device::new_cuda(ordinal)` path and a direct safe `CudaContext::new(ordinal)` probe for name, compute capability, and physical memory observations. There is deliberately no cross-call device or context cache.

Sequence planning performs arithmetic only. Sequence creation allocates a Candle cache on the loaded model's retained device. Prefill and decode create their required tensors and execute through that same device; they do not construct a CUDA device or direct CUDA context per sequence or token. Sequence destruction, model synchronization, and unload preparation also reuse and synchronize the retained device. Successful unload preparation does not create another context; resources are released when the owner drops the model and finished sequences.

## Generation lifecycle

The loaded model exclusively owns weights and the execution device. Each sequence owns an independent Candle cache, position, fixed token capacity, and token staging allocation. Sequences do not retain or clone the loaded model. Successful explicit destruction synchronizes the loaded device and marks the sequence `SequenceState::Finished` before drop.

The compatibility path executes:

```text
inspect
→ plan and load
→ verify source scalar, execution scalar, device, and accounted footprint
→ create independent sequences on the loaded device
→ prefill and interleave decode
→ cancel at a checked boundary
→ reject unsupported in-place sequence reset
→ expire a bounded drain window
→ explicitly destroy sequences and assert Finished
→ synchronize and prepare unload
→ drop sequence and model resources
```

## Deterministic CPU and opt-in CUDA tests

The deterministic CPU fixtures assign distinguishable token embeddings and LM-head rows while keeping the transformer residual path stable. CPU tests cover all advertised scalar mappings:

- F32 source, F32 execution;
- F16 source, F16 execution;
- BF16 source metadata retained as BF16 with actual F32 execution;
- weight and cache accounting derived from the execution scalar;
- host F32 final-position logits for each mapping;
- exact decode position progression, independent sequence state, cancellation boundaries, synchronized destruction, and unload preparation.

With the `cuda` feature enabled, a non-ignored test explicitly executes CPU to prove that enabling CUDA does not change device selection or initialize the CUDA branch for a CPU request. Oversized CUDA identities are rejected before driver initialization.

Actual CUDA hardware tests remain ignored and additionally require `MILKDRIFT_CUDA_TEST=1`. The accepted target test probes CUDA ordinal 0, validates its driver-reported identity, compute capability, BF16 support, physical total memory, and current available memory, compares deterministic F32 CPU/CUDA logits, proves BF16 source metadata remains BF16 with actual BF16 CUDA execution, synchronizes sequence/model work, and prepares unload. Feature compilation alone is not execution evidence.

## Allocation and reset capabilities

The adapter intentionally does not advertise `CapabilitySet::ALLOCATION_FREE_HOT_PATH`. Candle 0.11's upstream Llama implementation concatenates KV-cache tensors and constructs intermediate tensors during forward passes. CUDA host-logit transfer also uses a temporary upstream CPU tensor.

The adapter does not advertise `CapabilitySet::SEQUENCE_RESET`. Candle's upstream Llama cache cannot clear private KV and mask state in place without replacement allocation. Callers destroy and recreate a sequence at a cold lifecycle boundary.

## Failure containment and deferred work

Candle failures are translated into allocation-neutral `BackendFailure` values with stable categories and numeric codes. Model construction remains isolated behind `catch_unwind` because the upstream loader can panic for malformed layer structure.

GGUF and other quantized formats, Metal execution, cuDNN, flash attention, NCCL, multi-GPU execution, and GPU-side sampling remain unsupported.
