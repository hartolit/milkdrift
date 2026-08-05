# Candle backend

## Scope

`crates/adapters/candle-backend` is the sole current local execution adapter. It implements the `domain-contracts` backend boundary for the current compatibility subset of unquantized Hugging Face Llama configuration files and one or more Safetensors weight shards. CPU is compiled by default. The non-default `candle-backend/cuda` feature can construct an explicitly requested CUDA ordinal and never falls back to CPU; current product support is limited to ordinal 0 on the exact executed Linux x86_64 RTX 5070 Ti matrix in [implementation status](implementation-status.md). Feature compilation or ordinal representation alone is not hardware-execution evidence.

The adapter owns every Candle and `cudarc` type. No native tensor, device, model, cache, context, dtype, or error crosses into `domain-contracts`. Artifact download and tokenizer integration remain separate adapters.

## Source and execution scalar contract

Scalar terminology has three distinct meanings:

- **Configuration-declared source scalar:** immutable scalar metadata declared by model configuration and exposed as `ModelDescriptor::metadata.scalar_type`;
- **Observed tensor dtype:** the dtype encoded independently in one Safetensors tensor entry and inspected during loading;
- **Execution scalar:** the scalar selected for backend execution tensors during device-aware planning, exposed by `LoadPlan::execution_scalar_type` and retained by `LoadedModel::execution_scalar_type()` as loaded evidence.

Candle's `DType` remains private to the adapter. The current Llama loader requires every observed tensor dtype in every shard to equal the configuration-declared source scalar before any conversion to the execution scalar. This strict equality is an intentional current compatibility boundary, not a Safetensors format rule: a mixed-dtype repository is not necessarily malformed, but it may fail with `UnsupportedFormat`.

The execution policy remains:

| Configuration-declared source scalar | Execution scalar on CPU | Execution scalar on supported CUDA |
|---|---|---|
| F32 | F32 | F32 |
| F16 | F16 | F16 |
| BF16 | F32 | BF16 when the selected device reports support |

Unsupported BF16 CUDA execution fails during the cold planning/loading preparation step, before any model weight shard is loaded. Vocabulary logits are normalized to caller-owned host F32 storage for every supported configuration-declared source scalar and execution scalar.

## Accounted footprint and physical-memory observation

`MemoryFootprint`, `ModelDescriptor::estimated_footprint`, `LoadPlan::expected_footprint`, and `LoadedModel::accounted_footprint()` are planning and accounting quantities. `accounted_footprint()` is the adapter quantity E0 verifies against its accepted plan; after admission, E0 exposes that committed ownership quantity as a reserved footprint in receipts and snapshots. Neither term measures physical memory currently allocated by Candle or available on a device.

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

The loaded model reports its actual `ExecutionDevice`, execution scalar, complete descriptor, and accounted footprint through the domain `LoadedModel` contract. E0 compares those facts with the explicit request and accepted `LoadPlan` before committing a model slot or load receipt. A mismatch enters E0’s existing explicit unload/quarantine path; it is never repaired by selecting CPU.

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

- F32 configuration-declared source scalar, homogeneous F32 tensors, and F32 execution;
- F16 configuration-declared source scalar, homogeneous F16 tensors, and F16 execution;
- BF16 configuration-declared source metadata retained as BF16 with homogeneous BF16 tensors and actual F32 execution;
- rejection with `UnsupportedFormat` when an observed tensor dtype differs from the declared source scalar, using the existing fixture;
- weight and cache accounting derived from the execution scalar;
- host F32 final-position logits for each mapping;
- exact decode position progression, independent sequence state, cancellation boundaries, synchronized destruction, and unload preparation.

With the `cuda` feature enabled, a non-ignored test explicitly executes CPU to prove that enabling CUDA does not change device selection or initialize the CUDA branch for a CPU request. Oversized CUDA identities are rejected before driver initialization.

Actual CUDA hardware tests remain ignored and additionally require `MILKDRIFT_CUDA_TEST=1`. The accepted target test probes CUDA ordinal 0 on the exact supported matrix, validates its driver-reported identity, compute capability, BF16 support, physical total memory, and current available memory, compares deterministic F32 CPU/CUDA logits, proves BF16 source metadata remains BF16 with BF16 execution, synchronizes sequence/model work, and prepares unload. The accepted self-hosted run is recorded in [implementation status](implementation-status.md); feature compilation alone is not execution evidence.

## Allocation and reset capabilities

The adapter intentionally does not advertise `CapabilitySet::ALLOCATION_FREE_HOT_PATH`. Candle 0.11's upstream Llama implementation concatenates KV-cache tensors and constructs intermediate tensors during forward passes. CUDA host-logit transfer also uses a temporary upstream CPU tensor.

The adapter does not advertise `CapabilitySet::SEQUENCE_RESET`. Candle's upstream Llama cache cannot clear private KV and mask state in place without replacement allocation. Callers destroy and recreate a sequence at a cold lifecycle boundary.

## Failure containment and deferred work

Candle failures are translated into allocation-neutral `BackendFailure` values with stable categories and numeric codes. Model construction remains isolated behind `catch_unwind` because the upstream loader can panic for malformed layer structure.

GGUF and other quantized formats, Metal execution, cuDNN, flash attention, NCCL, multi-GPU execution, and GPU-side sampling remain unsupported.
