# Candle backend

## Scope

`crates/adapters/candle-backend` is the sole current local execution adapter. It implements the portable `domain-contracts` backend boundary for the reviewed unquantized Hugging Face Llama configuration and Safetensors path. CPU is mandatory/default. The non-default `candle-backend/cuda` feature constructs only an explicitly requested CUDA ordinal and never falls back to CPU.

The adapter owns every Candle, Safetensors, filesystem, and `cudarc` detail. Tensor names, shard paths, offsets, payload digests, native tensors, devices, model/cache types, dtypes, and vendor errors do not cross into portable domain code, E1, or Slint. Project-authored source continues to forbid unsafe code.

Phase 12 broadens the exact floating tensor layouts accepted by this path; it does not add quantization, GGUF, another architecture, another engine, or generic GPU support. Hardware evidence remains limited to the exact locally executed RTX 5070 Ti row.

## Four distinct scalar facts

Phase 12 deliberately separates four meanings:

- **Configuration-declared optional scalar** is recognized immutable configuration metadata supplied to `CandleLlamaSource` as `Option<ScalarType>`. It represents producer intent only. It does not prove tensor homogeneity and may be absent.
- **Observed tensor scalar set** is `ModelMetadata::observed_tensor_scalar_types`, a fixed-size `ScalarTypeSet` populated from every tensor header in every selected Safetensors shard. It records categories without exporting per-tensor format details.
- **Inferred primary scalar** is adapter-private. It is derived only from the exact observed set and is used to validate a present declaration and select execution policy.
- **Execution scalar** is selected during device-aware `prepare_load`, stored in `LoadPlan::execution_scalar_type`, materialized for every final execution tensor, and reported by `LoadedModel::execution_scalar_type()` for E0 verification.

Candle's `DType` remains private to the adapter. No layer may use a configuration declaration as a substitute for observed headers or treat the execution scalar as a description of serialized bytes.

## Exact accepted layouts

The adapter accepts exactly these observed sets:

| Observed set | Inferred primary | Permitted declaration |
|---|---|---|
| `{F32}` | F32 | `None` or F32 |
| `{F16}` | F16 | `None` or F16 |
| `{F16,F32}` | F16 | `None` or F16 |
| `{BF16}` | BF16 | `None` or BF16 |
| `{BF16,F32}` | BF16 | `None` or BF16 |

When a recognized declaration is present it must equal the inferred primary. Absence is accepted. A contradictory or unsupported present declaration returns `UnsupportedFormat`.

Every other observed set is rejected, including:

- any set containing both F16 and BF16, with or without F32;
- an empty set;
- FP8, integer, boolean, unknown, or another unsupported Safetensors dtype;
- quantized tensor representations;
- malformed, duplicate, missing-required, shape-incompatible, overflowing, or out-of-bounds tensor layouts.

Unsupported tensor dtypes and disallowed scalar combinations are rejected during complete header inspection, before execution-device initialization or tensor materialization. This is the exact current compatibility subset, not general mixed-dtype Safetensors support.

## Execution-scalar policy

| Inferred primary | CPU execution | Supported CUDA planner policy |
|---|---|---|
| F32 | F32 | F32 |
| F16 | F16 | F16 |
| BF16 | F32 | BF16 when the selected device reports BF16 support |

A BF16 CUDA request on a device that does not report BF16 support fails during preparation. Unsupported device kinds fail explicitly. Enabling the CUDA feature does not alter an explicit CPU request.

Every selected tensor is materialized from its own observed source dtype, converted on CPU to the one selected execution dtype when needed, and then retained on CPU or transferred to the explicitly selected CUDA device. Final vocabulary logits are normalized to caller-owned host F32 storage for every accepted mapping.

This table describes implemented planner policy. The exact Phase 12 CUDA compile chain and local deterministic hardware matrix passed on 2026-08-08 with `CUDA_COMPUTE_CAP=120` on the exact RTX 5070 Ti row. This does not establish generic NVIDIA or external mixed-checkpoint compatibility, and the Phase 12 GitHub self-hosted workflow remains unrun.

## Exact preparation and artifact binding

The model-loader lifecycle is now:

```text
inspect(source)
    -> device-independent descriptor and CPU final estimate

prepare_load(source, exact configuration)
    -> complete pre-device header inspection
    -> inferred primary and declaration validation
    -> explicit device initialization
    -> execution-scalar selection
    -> exact final and loading-peak calculation
    -> budget/current-availability validation
    -> opaque PreparedLoad exposing one exact LoadPlan

load_prepared(preparation)
    -> consume that exact preparation without replanning
    -> materialize, convert, transfer, construct, synchronize
    -> complete model or FailedLoad<PreparedLoad>
```

Preparation sorts selected weight paths deterministically, caps the selected shard count and aggregate header allocation, opens every shard, validates length prefixes before header-sized allocation, and retains the open `File` handles. Header inspection validates parser metadata, duplicate keys/names, offsets, bounds, shapes, checked element counts, supported dtypes, and the complete required Llama tensor schema.

For each tensor the preparation retains:

- the open shard handle and inspected file length;
- source dtype, shape, range, element count, and required/extra classification;
- a SHA-256 digest of the exact payload range.

`load_prepared` reads from those retained handles rather than reopening source paths. Before reading a shard it rechecks the retained handle's length; after each payload read it recomputes and compares the per-tensor digest before constructing the source tensor. Therefore:

- removing the path after preparation does not invalidate or redirect the retained open file;
- replacing the path cannot switch the accepted preparation to another file;
- same-inode payload mutation is detected by the digest check;
- materialization cannot silently inspect one set of weight bytes and load another.

The parsed Llama configuration and exact `LoadConfiguration` are likewise retained in the opaque preparation. An unmaterialized preparation rejected by E0 is ordinary-drop-safe. Once materialization has failed, explicit `PreparedLoad::cleanup` is required.

## Per-tensor materialization

For each inspected tensor, in deterministic shard/tensor order, Candle:

1. reads one aligned host payload from the retained shard handle;
2. verifies its prepared digest;
3. constructs a CPU source tensor with the observed dtype and inspected shape;
4. casts it on CPU when source and execution dtypes differ;
5. either transfers ownership into the CPU final map or transfers to the selected CUDA device;
6. synchronizes CUDA transfer before moving the device tensor into the final map;
7. retains one final tensor by name and releases staging endpoints as soon as the transaction permits.

After all selected tensors are present, `Llama::load` constructs the model behind `catch_unwind`. The completed native model is stored in the transaction before final synchronization so even a synchronization failure returns one complete cleanup owner. On success, required tensor handles move into `CandleLlamaModel`; extra tensors and duplicate shallow map handles are released before the model is returned.

## Final and loading-peak formulas

`MemoryFootprint` contains deterministic tensor ownership/headroom for the phase named by its carrier. It excludes serialized headers/configuration, paths/digests, allocator bookkeeping and fragmentation, driver/context allocation, process RSS, and whole-device observations.

For each selected tensor `i`, define:

- `Nᵢ`: checked element count;
- `Sᵢ = Nᵢ × source_widthᵢ`: source payload bytes;
- `Eᵢ = Nᵢ × execution_width`: execution tensor bytes;
- `Aᵢ = Sᵢ + alignmentᵢ - 1`: aligned staging allocation bound;
- `Pᵢ = Σ Eⱼ` for tensors ordered before `i` in the final map;
- `R = Σ Eᵢ` for required Llama tensors only;
- `M = Σ Eᵢ` for every selected tensor, including supported extras;
- `C = layers × 2 × key_value_heads × head_dimension × execution_width`: cache bytes per token.

All products and sums use checked arithmetic.

### CPU

The final CPU footprint is:

```text
host_weight_bytes      = R
host_working_bytes     = 0
device_weight_bytes    = 0
device_working_bytes   = 0
cache_bytes_per_token  = C
```

The exact CPU host loading peak is:

```text
Hcpu = max(
    M,
    max_i(Pᵢ + Aᵢ + Sᵢ),
    max_i(Pᵢ + Sᵢ + Eᵢ) for tensors requiring a cast)
)
```

The CPU loading-peak footprint is:

```text
host_weight_bytes      = R
host_working_bytes     = Hcpu - R
device_weight_bytes    = 0
device_working_bytes   = 0
cache_bytes_per_token  = C
```

This covers already retained execution tensors, aligned raw staging, the CPU source tensor, cast duplication where needed, and supported extra tensors that are materialized but not part of final required model ownership.

### CUDA

The final CUDA footprint is:

```text
host_weight_bytes      = 0
host_working_bytes     = 0
device_weight_bytes    = R
device_working_bytes   = 0
cache_bytes_per_token  = C
```

The exact CUDA host staging peak is:

```text
Hcuda = max_i(
    Aᵢ + Sᵢ,
    Eᵢ,
    Sᵢ + Eᵢ when tensor i requires a cast
)
```

The CUDA loading-peak footprint is:

```text
host_weight_bytes      = 0
host_working_bytes     = Hcuda
device_weight_bytes    = R
device_working_bytes   = M - R
cache_bytes_per_token  = C
```

`M - R` accounts for supported extra execution tensors temporarily transferred into the complete map before native model construction releases them. Host-to-device work is synchronized per tensor. No physical-residency claim follows from these deterministic quantities.

`ModelDescriptor::estimated_footprint` is the device-independent CPU final footprint produced by inspection. The exact target-specific `LoadPlan::expected_footprint` and `loading_peak_footprint` are produced by `prepare_load`. Preparation checks the complete loading peak against host/device budgets and, for CUDA, current driver-reported availability before materialization.

## Failed materialization ownership

`CandleLlamaPreparedLoad` is the sole transaction owner of:

- retained device and parsed configuration;
- open inspected shards;
- completed final tensors;
- pending source, cast-host, or transferred-device tensor;
- a completed native model when final synchronization fails.

`load_prepared` returns `FailedLoad<CandleLlamaPreparedLoad>` on any materialization, construction, or synchronization failure. It never converts away the only owner.

`PreparedLoad::cleanup` is retryable and idempotent after success. It synchronizes the retained device first. A synchronization failure leaves every owner reachable and the preparation valid for another attempt. Only successful synchronization clears the model, final/pending tensors, shards, configuration, and device and marks cleanup complete. E0 owns bounded retry, peak accounting, exhaustion, and terminal shutdown policy; see [Inference runtime](inference-runtime.md) and [ADR-0020](../agent/decisions/0020-transactional-prepared-model-loading.md).

## Generation lifecycle

The loaded model exclusively owns required execution weights and the selected device. Each sequence owns an independent Candle cache, position, fixed token capacity, and token staging allocation. Sequences do not retain or clone model ownership.

Sequence planning remains arithmetic-only. Creation allocates the cache on the loaded model's retained device. Prefill/decode use that same device, and sequence destruction, model synchronization, and unload preparation explicitly synchronize it. No device/context is created per token.

The compatibility path executes:

```text
prepare exact load
-> admit loading peak
-> materialize each tensor
-> verify final descriptor/device/execution scalar/accounted footprint in E0
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

Current download-free CPU coverage includes:

- all five exact accepted layouts and absent/matching declarations;
- F32→F32, F16→F16, and BF16→F32 CPU execution;
- F16+BF16 and unsupported U8 rejection before device initialization;
- duplicate, malformed, bounds, shape, overflow, shard-order, and required-schema rejection;
- exact final/loading-peak formulas, extra-tensor headroom, and pre-materialization budget rejection;
- retained-open-file loading and same-inode digest-mutation rejection;
- per-tensor conversion, prefill/decode logits, synchronization, destruction, and unload;
- partial cleanup failure retaining every owner and succeeding exactly once on retry.

These focused, download-free CPU tests passed on the Phase 12 closure tree. Temporary mixed derivatives and their provenance are documented beside the committed fixture in [`PROVENANCE.md`](../../crates/runtime/inference-runtime/tests/fixtures/candle-llama/PROVENANCE.md).

CUDA-enabled tests preserve explicit CPU execution and reject oversized ordinals before driver initialization. On 2026-08-08, the explicit CPU-in-CUDA test and all four guarded ignored CUDA adapter tests passed locally on the exact RTX 5070 Ti row. They cover homogeneous F32/BF16 and mixed F16/F32 and BF16/F32 CUDA policy, exact peak/final accounting, generation, synchronization, and unload under `MILKDRIFT_CUDA_TEST=1`. These are local closure-tree results; the Phase 12 GitHub self-hosted workflow has not run.

## Unsupported and deferred work

GGUF and other quantized formats, arbitrary mixed layouts, F16+BF16, unsupported/unknown tensor dtypes, Metal, cuDNN, flash attention, NCCL, multi-GPU execution, and GPU-side sampling remain unsupported. External mixed-checkpoint evidence is absent. Another model architecture or conversion policy requires a separate reviewed matrix rather than weakening this fail-closed path.
