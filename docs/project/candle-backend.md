# Candle backend

## Scope

`crates/adapters/candle-backend` is the sole current local execution adapter. It implements the portable `domain-contracts` backend boundary for the reviewed unquantized Hugging Face Llama configuration and Safetensors path. CPU is mandatory/default. The non-default `candle-backend/cuda` feature constructs only an explicitly requested CUDA ordinal and never falls back to CPU.

The adapter owns every Candle, Safetensors, filesystem, source-identity, and `cudarc` detail. Tensor names, shard paths, offsets, whole-shard digests, native tensors, devices, model/cache types, dtypes, and vendor errors do not cross into portable domain code, E1, or Slint. Project-authored source continues to forbid unsafe code.

The artifact-loading amendment hardens the reviewed floating Llama path; it does not add quantization, GGUF, another architecture, another engine, or generic GPU support. Current-tree compile and hardware evidence is stated separately in [implementation status](implementation-status.md).

## Four distinct scalar facts

The loader deliberately separates four meanings:

1. **Configuration declaration** is optional recognized F32/F16/BF16 producer intent derived from the same bounded `config.json` bytes Candle decodes. Callers cannot inject it.
2. **Complete observed scalar set** is `ModelMetadata::observed_tensor_scalar_types`, populated from every structurally valid tensor header in every selected shard, including unused extras.
3. **Required scalar set and primary** are adapter-private and derive only from tensors consumed by the supported Llama schema.
4. **Execution scalar** is selected during device-aware preparation, stored in `LoadPlan`, materialized only for required tensors, and reported by the loaded model for E0 verification.

Safetensors 0.8 categories are classified completely. F32, F16, BF16, I8, and U8 map directly into portable categories; BOOL, bit-packed/FP8, wider integer, F64, complex, and remaining understood forms map to stable adapter-owned `Other` codes that collapse into the portable `Other` bit. An unused understood tensor may use any such category: it remains complete observed evidence but does not select precision or require device memory. A required tensor must be F32, F16, or BF16.

Modern `dtype` does not silently fall back to legacy `torch_dtype`. Both absent/null means no declaration; one recognized field or two equal recognized fields selects that declaration; any present unsupported value, conflicting recognized pair, duplicate field, wrong type, or malformed JSON fails explicitly. A recognized declaration must be absent or equal the required primary.

## Exact required-layout and execution policy

| Required set | Required primary | Permitted declaration | CPU execution | Supported CUDA policy |
|---|---|---|---|---|
| `{F32}` | F32 | absent or F32 | F32 | F32 |
| `{F16}` | F16 | absent or F16 | F16 | F16 |
| `{F16,F32}` | F16 | **F16 required** | F16 | F16 |
| `{BF16}` | BF16 | absent or BF16 | F32 | BF16 when supported |
| `{BF16,F32}` | BF16 | **BF16 required** | F32 | BF16 when supported |

A genuine required F16+BF16 mixture, empty required set, required unsupported dtype, quantized representation, malformed/duplicate tensor layout, missing required tensor, shape mismatch, gap/overlap, bounds failure, or arithmetic overflow is rejected before execution-device initialization. Complete observed extras never alter this matrix. A BF16 CUDA request on a device without reported BF16 support fails during preparation; unsupported devices fail explicitly and there is no CPU fallback. Final vocabulary logits cross to E0 as caller-owned host F32.

## Bounded inspection and source identity

`inspect` reads bounded configuration bytes plus each shard's 8-byte prefix and bounded JSON header. It scans no payload and initializes no execution device. Custom deserialization and checked allocation enforce these production ceilings:

| Structure | Ceiling |
|---|---:|
| selected shards | 256 |
| one / aggregate shard headers | 8 MiB / 64 MiB |
| tensors | 16,384 |
| one / aggregate tensor-name bytes | 512 / 8 MiB |
| tensor rank / one dimension extent | 8 / 1,048,576 |
| aggregate shape dimensions | 131,072 |
| metadata entries | 1,024 |
| metadata key / value bytes | 256 / 4 KiB |
| aggregate metadata strings | 4 MiB |
| final owned inspection inventory | 64 MiB |
| configuration bytes | 1 MiB |

Shapes use fixed rank-8 storage. Inspection validates duplicate JSON/tensor names, cross-shard duplicates, deterministic contiguous offsets with no gaps or overlaps, exact payload/file bounds, checked element/bit counts, and the complete required Llama schema before device initialization.

`CandleWeightShard` keeps each path paired with one authority:

- `VerifiedImmutable { length, sha256 }` is accepted only from a source with proven cryptographic identity and immutability semantics. The current production authority is exact Hugging Face LFS SHA-256 plus file size at the resolved commit.
- `ProjectEstablished { length, sha256 }` says project code hashed the whole file but the path is not proven immutable.
- `Unverified` carries no reusable identity.

Candle trusts no cache name, symlink target, Git object ID, ETag, inode, mtime, or provider convention. It opens and retains every shard. Only verified immutable identity skips a pre-admission payload pass. Project-established shards are rehashed against their supplied digest and unverified shards establish a fresh baseline, both sequentially from the retained file before device admission.

## Sequential selective materialization

`prepare_load` retains exact config, device, plan, open shards, complete private inventory, prefix/header digests, and established whole-shard identities. `load_prepared` consumes that preparation without replanning. Each shard then receives exactly one sequential pass from byte zero:

1. hash and compare the retained prefix/header before payload processing;
2. hash unused ranges through a fixed checked 64 KiB buffer without tensor-sized allocation;
3. read each required range once into aligned staging;
4. construct its CPU source tensor, cast independently when required, and retain on CPU or transfer/synchronize on CUDA;
5. verify exact EOF, length, and whole-shard SHA-256;
6. after every shard verifies, construct `Llama` behind `catch_unwind` and synchronize before publication.

There are no per-tensor seeks, payload digests, mmap, unsafe code, or whole-model host buffers. Unused tensors are never converted, transferred, inserted into Candle's map, or retained. Removing/replacing the path cannot redirect an open retained file; header mutation fails before payload processing, while payload mutation, truncation, and extension fail before model publication. A late whole-shard mismatch consumes the pre-attempt preparation and returns a distinct failed-materialization owner containing every already materialized resource.

An unmaterialized `CandleLlamaPreparedLoad` rejected by E0 is ordinary-drop-safe. After materialization starts, every failure returns `FailedLoad<CandleLlamaFailedPreparation>` and requires explicit cleanup through `FailedLoadOwner`.

## Final and loading-peak formulas

`MemoryFootprint` contains deterministic concrete required-tensor bytes only. The exact sequence-cache bytes-per-token rate is stored separately in `ModelDescriptor` and multiplied by accepted sequence capacity to produce a concrete `SequencePlan` footprint. The fixed 64 KiB verification buffer, parsed config/header/inspection metadata, and required-name/load-map metadata are independently capped by the limits above. Required maps cannot exceed 16,384 entries or 8 MiB aggregate names; one name clone is transient per materialized tensor, and failure-safe model construction temporarily duplicates one map's names/shallow tensor handles under the same bounds. Platform-dependent map bucket overhead, allocator bookkeeping/fragmentation, driver/context allocation, process RSS, and whole-device observations remain outside the tensor footprint.

For each required tensor `i` in materialization order:

- `Sᵢ` is exact source bytes;
- `Eᵢ` is execution bytes;
- `Aᵢ = Sᵢ + alignmentᵢ - 1` is the aligned staging bound;
- `Pᵢ` is required execution bytes already retained;
- `R = Σ Eᵢ` is all final required weight bytes;
- `C = layers × 2 × key_value_heads × head_dimension × execution_width` is cache bytes per token.

Unused tensors enter no formula. All arithmetic is checked.

### CPU

```text
Hcpu = max(
    R,
    max_i(Pᵢ + Aᵢ + Sᵢ),
    max_cast_i(Pᵢ + Sᵢ + Eᵢ)
)

final:   host weights R, host working 0, device weights/working 0
loading: host weights R, host working Hcpu - R, device weights/working 0
```

This matches real ordering: aligned bytes and a source tensor coexist; for a cast, source and execution tensors coexist; already completed required tensors remain retained.

### CUDA

```text
Hcuda = max(
    max_i(Aᵢ + Sᵢ),
    max_i(Eᵢ),
    max_cast_i(Sᵢ + Eᵢ)
)

final:   host weights/working 0, device weights R, device working 0
loading: host weights 0, host working Hcuda, device weights R, device working 0
```

The host execution tensor remains live until its transferred device tensor has synchronized and entered the final map. Only required device tensors are ever transferred, so ignored extras require no device headroom. `ModelDescriptor::estimated_footprint` is the device-independent CPU final estimate; `prepare_load` produces exact target-specific final and loading plans and checks the loading peak against host/device budgets and current CUDA availability before materialization.

## Sequence reservation follows simultaneous lifetimes

The adapter's `SequencePlan::expected_footprint` is a checked conservative upper bound for reviewed live logical tensor payload and source-transfer bytes, not a physical allocator measurement and not a sum of mutually exclusive phases. The locked batch-one, non-flash Candle Llama 0.11.0 model separates persistent all-layer KV/cache ownership from one transformer block's transient forward peak and the outer embedding/norm/logit peak.

Candle's model loop replaces the current hidden state while executing blocks sequentially. Consequently:

```text
persistent = KV for every layer + rotary cache + retained mask cache
execution peak = persistent + one block transient peak + outer model peak
reservation = component-wise max(sequence creation, execution)
```

Only the persistent KV term scales with `num_hidden_layers`; multiplying the complete block transient set by layer count would reserve tensors that are never simultaneously live and can reject valid requests by tens of GiB. CPU and CUDA retain separate host/device source-transfer phases, and E0 separately reserves caller-owned logits, sampling, history, stop, and output state. Exact synthetic cases plus a 22-layer TinyLlama full-context regression lock the reviewed formula.

## Failed materialization ownership

`CandleLlamaPreparedLoad` is the ordinary-drop-safe transaction before materialization. Consuming it either returns a complete model or creates the distinct `CandleLlamaFailedPreparation`, whose sole purpose is to retain and explicitly clean resources acquired by the failed attempt. The failed owner contains:

- retained device and parsed configuration;
- open inspected shards;
- completed final tensors;
- pending source, cast-host, or transferred-device tensor;
- a completed native model when final synchronization fails.

`load_prepared` returns `FailedLoad<CandleLlamaFailedPreparation>` on any materialization, construction, or synchronization failure. It never converts away or aliases the only cleanup owner.

`FailedLoadOwner::cleanup` is retryable and idempotent after success. It synchronizes the retained device first. A synchronization failure leaves every owner reachable and the failed typestate valid for another attempt. Only successful synchronization clears the model, final/pending tensors, shards, configuration, and device and marks cleanup complete. The failed owner must report the accepted plan for its entire lifetime; E0 checks that report on both sides of each cleanup attempt and reclassifies any contradiction as unverified ownership. E0 owns bounded retry, peak accounting, exhaustion, and terminal shutdown policy; see [Inference runtime](inference-runtime.md) and [ADR-0020](../agent/decisions/0020-transactional-prepared-model-loading.md).

## Generation lifecycle

The loaded model exclusively owns required execution weights and the selected device. Each sequence owns an independent Candle cache, position, fixed token capacity, and token staging allocation. Sequences do not retain or clone model ownership.

Sequence planning remains arithmetic-only. Creation allocates the cache on the loaded model's retained device. Prefill/decode use that same device, and sequence destruction, model synchronization, and unload preparation explicitly synchronize it. No device/context is created per token.

The compatibility path executes:

```text
prepare exact load
-> admit loading peak
-> sequentially verify shards and materialize required tensors only
-> verify final descriptor/device/execution scalar/reported footprint in E0
-> commit final reservation and publish receipt
-> create independent sequences
-> prefill and interleave decode
-> sample host F32 logits in E0
-> explicitly destroy sequences
-> synchronize and prepare unload
-> release model ownership
```

The adapter still does not advertise `ALLOCATION_FREE_HOT_PATH`: upstream Candle Llama creates intermediate/cache tensors, and CUDA host-logit transfer may allocate a temporary CPU tensor. It does not advertise `SEQUENCE_RESET` because upstream private cache state cannot be reset in place under the required contract.

## Deterministic validation state

Current deterministic coverage includes:

- required F32 with unused F16, BF16, both, integer, boolean, F64/`Other`, and large extras under absent/matching declarations;
- mixed required F16/F32 and BF16/F32 with unrelated extras, plus genuine required F16/BF16 rejection;
- strict modern/legacy declaration precedence, unsupported values, conflicts, malformed values, and explicit Llama identity;
- required unsupported-dtype rejection before device initialization while unused understood categories load;
- duplicate tensors, shard reordering, offset gaps/overlaps, bounds, truncation/extension, shape/schema mismatch, and checked overflow;
- every production metadata ceiling, individual dimension extent, and injected allocation failures;
- verified-immutable fast-path behavior, mutable project/unverified baselines, retained-file mutation detection, and late whole-shard verification;
- deterministic instrumentation proving ignored ranges are hashed but never materialized or transferred, including a CUDA-branch simulation without hardware;
- exact required-only CPU/CUDA final and loading formulas plus pre-materialization budget rejection;
- every source/host/cast/transfer/map/model/final-sync ownership checkpoint and idempotent retryable cleanup.

The package's dedicated harness-free `cuda_hardware` target runs the complete reviewed adapter hardware suite: explicit CPU execution in a CUDA build, invalid ordinal rejection, F32 CUDA/CPU logits, homogeneous BF16, mixed F16/F32, and mixed BF16/F32. Its custom runner requires explicit opt-in, registers at least one case, counts every attempt, and cannot silently succeed with zero execution.

Exact current-tree commands and whether CUDA was compiled or executed are recorded in [validation](validation.md) and [implementation status](implementation-status.md). Temporary mixed derivatives and fixture identity are documented beside the committed fixture in [`PROVENANCE.md`](../../crates/runtime/inference-runtime/tests/fixtures/candle-llama/PROVENANCE.md). The 2026-08-08 RTX 5070 Ti execution remains historical evidence for the earlier Phase 12 closure tree unless the amended tree is separately run on hardware.

## Unsupported and deferred work

GGUF and other quantized formats, arbitrary required mixed layouts, required F16+BF16, required unsupported/unknown tensor dtypes, Metal, cuDNN, flash attention, NCCL, multi-GPU execution, and GPU-side sampling remain unsupported. External mixed-checkpoint evidence is absent. Another model architecture or conversion policy requires a separate reviewed matrix rather than weakening this fail-closed path.
