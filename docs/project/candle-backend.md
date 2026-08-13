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

## Required-layout and execution policy

The exact accepted product layouts are listed once in
[implementation status](implementation-status.md#support-matrix), and
[ADR-0020](../agent/decisions/0020-transactional-prepared-model-loading.md#accept-only-the-reviewed-required-layouts)
preserves the decision matrix. Locally, Candle derives one required primary from
the consumed tensors: F32 and F16 execute as themselves; BF16 executes as F32 on
CPU and as BF16 only on a selected accelerator that reports support. A mixed
F16/F32 or BF16/F32 required set is accepted only with the matching producer
declaration. Final vocabulary logits cross to E0 as caller-owned host F32.

A genuine required F16+BF16 mixture, empty required set, required unsupported
dtype, quantized representation, malformed/duplicate tensor layout, missing
required tensor, shape mismatch, gap/overlap, bounds failure, or arithmetic
overflow is rejected before execution-device initialization. Complete observed
extras never change that result. Unsupported devices fail explicitly and there is
no CPU fallback.

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

`CandleWeightShard` keeps each path paired with one honest verification input:

- `with_expected_content(path, CandleExpectedContentIdentity::new(byte_length, sha256))` supplies the exact whole-file bytes Candle must observe without claiming provider provenance or path immutability;
- `unverified_local(path)` carries no reusable expectation, so Candle establishes a retained-file baseline itself.

Candle trusts no cache name, symlink target, Git object ID, ETag, inode, mtime, provider authority label, or cache convention. It opens and retains every shard. A supplied expectation skips the local baseline pass but is still verified during materialization. An unverified local shard establishes a fresh baseline sequentially from the retained file before device admission. Private diagnostics distinguish `SuppliedExpectation` from `LocallyEstablishedBaseline`; provider-specific provenance remains in the resolver/application evidence layer.

## Sequential selective materialization and bounded transfer batches

`prepare_load` retains exact config, device, plan, open shards, complete private inventory, prefix/header digests, and established whole-shard identities. `load_prepared` consumes that preparation without replanning. Each shard then receives exactly one sequential pass from byte zero:

1. hash and compare the retained prefix/header before payload processing;
2. hash unused ranges through a fixed checked 64 KiB buffer without tensor-sized allocation;
3. read each required range once into aligned staging;
4. construct its CPU source tensor and cast independently when required;
5. retain CPU execution tensors directly, or enqueue accelerator transfers into the current private transfer batch;
6. complete endpoint validation and one synchronization for each full intermediate accelerator batch, retaining every endpoint until the complete batch commits;
7. keep the shard's final planned batch unsynchronized while verifying exact EOF, length, and whole-shard SHA-256;
8. only after that identity succeeds, complete endpoint validation and one synchronization, then commit the shard-final batch under explicit per-entry commit state; and
9. after every shard verifies, construct `Llama` behind `catch_unwind` from the committed tensor handles.

Planning and materialization consume the same immutable `TransferPlan`, represented as flat `TransferEntryPlan` inventory plus `TransferBatchPlan` ranges. The partition is sequential and deterministic: `PREFERRED_BATCH_HOST_STAGING_BYTES` is 256 MiB, `MAXIMUM_BATCH_ENTRIES` is 64, and every shard end is a mandatory boundary. Adding an entry closes the current batch first when it would exceed either policy. The first entry of an empty batch is admitted even when it exceeds the preferred byte target, yielding a bounded oversized singleton. A batch therefore never crosses shards or grows without a byte/count bound. The shard-end boundary fixes membership, while synchronization/commit of that final batch waits for whole-shard identity verification.

There are no per-tensor seeks, payload digests, mmap, unsafe code, or whole-model host buffers. Unused tensors are never converted, transferred, inserted into Candle's map, or retained. Removing/replacing the path cannot redirect an open retained file; header mutation fails before payload processing, while payload mutation, truncation, and extension fail before model publication. A late whole-shard mismatch consumes the pre-attempt preparation and returns a distinct failed-materialization owner containing every already materialized resource and the complete current batch.

An unmaterialized `CandleLlamaPreparedLoad` rejected by E0 is ordinary-drop-safe. After materialization starts, every failure returns `FailedLoad<CandleLlamaFailedPreparation>` and requires explicit cleanup through `FailedLoadOwner`.

## Final and loading-peak formulas

`MemoryFootprint` contains deterministic concrete required-tensor bytes, aligned payload allocation bounds, the fixed 64 KiB verification buffer that is live during shard materialization, and the actual retained heap capacities of the accelerator plan/owner vectors. The exact sequence-cache bytes-per-token rate is stored separately in `ModelDescriptor`; the complete sequence reservation is described below. Parsed config/header/inspection metadata and required-name/load-map metadata are independently capped by the limits above. Required maps cannot exceed 16,384 entries or 8 MiB aggregate names, and entry names move from the bounded manifest through the batch into the final map rather than acquiring another string allocation. Map buckets, additional tensor handles outside the counted plan/owner vector allocations, allocator bookkeeping/fragmentation, driver/context allocation, process RSS, and whole-device observations remain outside the logical tensor footprint.

For each required tensor `i` in materialization order:

- `Sᵢ` is exact source bytes;
- `Eᵢ` is execution bytes;
- `Aᵢ = Sᵢ + alignmentᵢ - 1` is the aligned staging bound;
- `Pᵢ` is required execution bytes already retained;
- `R = Σ Eᵢ` is all final required weight bytes;
- `V = 64 KiB` is the shard-verification buffer;
- `M_plan = capacity(TransferPlan.batches) × size_of::<TransferBatchPlan>() + capacity(TransferPlan.entries) × size_of::<TransferEntryPlan>()`;
- `M_owner = capacity(TransferBatchOwner.entries) × size_of::<TransferBatchEntry>()`;
- `M = M_plan + M_owner`, using the actual checked capacities retained by the preparation;
- and `C = layers × 2 × key_value_heads × head_dimension × execution_width` is cache bytes per token.

Unused tensors enter no formula. All arithmetic is checked.

### CPU

CPU creates neither `TransferPlan` nor `TransferBatchOwner`, so its sequential path has no `M` term.

```text
Hcpu = max(
    R,
    V,
    max_i(V + Pᵢ + Aᵢ + Sᵢ),
    max_cast_i(V + Pᵢ + Sᵢ + Eᵢ)
)

final:   host weights R, host working 0, device weights/working 0
loading: host weights R, host working Hcpu - R, device weights/working 0
```

This matches real ordering: aligned bytes and a source tensor coexist; for a cast, source and execution tensors coexist; already completed required tensors remain retained.

### Accelerator transfer path

For entry `i` in batch `b`:

- `Qᵢ = Sᵢ` without a cast, otherwise `Qᵢ = Sᵢ + Eᵢ`, because both source and converted host tensors remain owned through synchronization;
- `C_b,ᵢ = Σ Qⱼ` over earlier entries `j` in that batch; and
- `W_b` is the implemented simultaneous host tensor-staging peak:

```text
W_b = max(
    Σ Qᵢ,
    max_i(C_b,ᵢ + Aᵢ + Sᵢ),
    max_cast_i(C_b,ᵢ + Sᵢ + Eᵢ)
)

Haccelerator = M + V + max_b(W_b)

final:   host weights/working 0, device weights R, device working 0
loading: host weights 0, host working Haccelerator, device weights R, device working 0
```

`M` and `V` are additive because the plan/owner capacities remain allocated for the preparation and the verifier remains allocated throughout staging and while the shard-final batch waits for identity verification and then synchronizes/commits. The byte target applies to candidate `W_b`; it is not an unrelated margin. Host source and optional cast tensors remain live through synchronization and the complete batch commit. While an entry commits, its batch owner and the final map temporarily hold shallow handles to the same device storage; handle aliasing does not duplicate payload bytes. Transferred-but-uncommitted storage, committed storage, and later Llama handles together never exceed `R` distinct logical execution bytes, so that ownership remains classified as device weights rather than device working memory. Only required device tensors are transferred, so ignored extras require no device headroom. `ModelDescriptor::estimated_footprint` is the device-independent CPU final estimate; `prepare_load` produces target-specific final and loading plans and checks the loading peak against host/device budgets and current device availability before materialization.

CUDA is the currently implemented accelerator path. The partition, batch owner, footprint simulation, and synchronization policy stay inside the Candle adapter and use no NVIDIA-specific portable contract. E0 sees only generic plan, execution-device, ownership, and cleanup facts; a later AMD accelerator path would remain an adapter implementation and could reuse this policy only where its transfer semantics support the same lifetime proof.

## Sequence reservation follows simultaneous lifetimes

`SequencePlan::reservation` is a `SequenceReservation` with three explicit facts:

- `persistent_footprint` is maximum sequence-owned logical payload retained between backend calls;
- `transient_footprint` is additional creation or one-call logical-payload/source-transfer headroom; and
- `total_footprint` is their checked component-wise sum and is the value E0 admits before native creation.

Caller-owned logits, sampling, token history, stop matching, output queues, and terminal records are accounted separately by E0. RSS/VRAM, allocator size classes and fragmentation, pools, CUDA contexts, kernel/library workspaces, stacks, and driver observations are not deterministic logical payload and are not included. The component arithmetic is exact for the documented upper-bound model; it is not an instantaneous allocator measurement.

The model is source-locked to batch one, non-flash Candle 0.11.0. Let `L` be layers, `T` sequence capacity, `M` maximum prefill, `P` configured positions, `H` hidden width, `I` intermediate width, `A` attention heads, `K` KV heads, `D = H/A`, `V` vocabulary, and `w` execution bytes. The persistent terms are:

```text
token staging = 4M host bytes
all-layer K/V = 2LTKDw
retained rope = PDw
retained masks <= T(T + M)/2 U8 bytes when M > 1, otherwise 0
```

The mask expression is a checked closed-form bound over every permitted repeated-prefill schedule and every retained `(seq_len, kv_len)` key. Decode is mask-free because `seq_len == 1`.

### Reviewed phase table

| Phase | Pinned Candle operation | Named reservation evidence |
|---|---|---|
| Sequence creation | `Cache::new` builds F32 inverse frequency, position, product, cosine, and sine tensors, then retains cosine/sine in execution dtype | retained rope plus creation device/CPU peak `2D + 6PD` bytes; CUDA host source is `max(4D, 4P)` |
| Input and mask | adapter `u32` staging, `Tensor::from_slice`, `Cache::mask`, and `utils::build_causal_mask` | fixed staging, input tensor, retained U8 masks, and at most `M×T` host mask source |
| Q/K/V layout | three linear projections, reshape/transpose, Q/K `contiguous`, and rotary output | separately named projection, layout-copy, and rotary-output payloads |
| Cache replacement | per-layer `Tensor::cat(..., 2)?.contiguous()` for K and V | all-layer final cache in persistent payload plus one current-layer full-pair duplicate bound; this replacement phase ends before attention compute |
| Grouped-query expansion | `utils::repeat_kv` uses `Tensor::cat` for K and V | two full expanded context tensors only when `A > K` |
| Non-flash attention | Q/K/V `to_dtype(F32)`, score matmul/divide, optional masked fill, softmax, contiguous V, attention-value matmul, and cast back | maximum of distinct first-prefill and cached-context phases, with named F32 conversions, score buffers, fill scalar, conditional V copy, output, cast, and projection |
| Transformer block | RMS norms, attention residual, gate/up/SILU/product/down MLP, final residual | maximum of named attention, residual, gate/up/product, down-projection, and final-add phases for one block; mutually exclusive MLP expression/down phases are not summed |
| Model forward | embedding, sequential block replacement, final RMS norm, last-token narrow/contiguous, LM head, final F32 conversion | input tensor plus maximum of embedding, one-block, and final-logits phases |
| CUDA logits copy | F32 logits `to_device(Device::Cpu)` before copying into the caller buffer | one host F32 vocabulary tensor, mutually exclusive with creation and mask-source phases |

Candle executes blocks sequentially, so transient block tensors do not scale with `L`; only persistent all-layer cache does. CPU transient host headroom is the maximum of creation extra and `mask source + model-forward peak`. CUDA transient host headroom is the maximum of cache-construction source, mask source, and host logits transfer; CUDA transient device headroom is the maximum of cache-creation extra and model-forward peak. Persistent and transient values are then added, while mutually exclusive transient phases are never summed.

This audit corrected seven production defects in the predecessor formula: CUDA mask-source and logits-transfer host phases had been summed despite being mutually exclusive, cache replacement had been summed with the later attention-compute phase, mutually exclusive MLP expression/down-projection peaks had also been summed, a first-prefill F32 V-contiguous copy had been charged against cached full-context compute (and against GQA output that is already contiguous), a cache-construction scalar source term had been multiplied by layer count, the masked-fill F32 device scalar was omitted, and anonymous tensor coefficients obscured which simultaneous lifetimes they represented. The reviewed adapter also rejects `K > P` before allocation because Candle 0.11.0's cache-trimming branch reads `dims()[1]` as sequence length and then narrows the last dimension.

### Version and conformance gate

Root dependencies are exact `=0.11.0`. The test gate also locks the reviewed crates.io artifacts:

| Package | SHA-256 checksum in `Cargo.lock` |
|---|---|
| `candle-core` | `5ecb245093b0f791b89d3420c3df9c6d49c60ab63ba54db896bf8a3baf486706` |
| `candle-nn` | `eaa10b6ccc365b33210ce404fbf45e60d3e0bdac1004463cf1052e6ee1c1739a` |
| `candle-transformers` | `3bcbbf7ff00ff6fe2af22b93600195917fe90e90ff48424a140d1a926c44b1c1` |

The reviewed source locations are `candle-transformers/src/models/llama.rs` (`Config`, `Cache`, attention, MLP, block, forward), `candle-transformers/src/utils.rs` (mask and grouped-query expansion), `candle-nn/src/{rotary_emb,ops,layer_norm,linear,embedding}.rs`, and `candle-core/src/tensor.rs` (creation, layout, concatenation, dtype, and device-transfer behavior).

Unit fixtures lock every named persistent/transient component for F32/F16/BF16 arithmetic, CPU/CUDA placement, GQA/non-GQA, mask-free and mask-producing paths, creation-dominated cases, overflow, unsupported paths, and a 22-layer TinyLlama full context. The executable oracle creates actual Candle caches, observes retained rotary dtype/shape, runs real non-GQA and GQA forwards, and checks per-layer K/V and mask shapes against the planned cache rate. Download-free CPU integration exercises homogeneous and mixed source policy, F32/F16 execution, BF16-source-to-F32 CPU policy, length-one and maximum prefill, first and near-capacity decode, stable plan/identity/capacity, and explicit destruction.

Transient private tensors cannot all be observed through Candle's public API. Their evidence is therefore the pinned source-derived named phase fixture, not RSS sampling or a patched registry. CUDA calculations and dedicated suites compile only with the CUDA toolchain; current hardware execution truth remains in [implementation status](implementation-status.md).

## Loader module boundaries

Loader responsibilities are separated by invariant: `config` owns bounded configuration decoding; `safetensors` owns allocation-bounded raw header decoding; `manifest` owns shard I/O, layout, and retained inventory; `configuration_policy` owns source-locked numeric Llama rejection; `scalar` owns declaration-to-execution compatibility; `schema` owns required tensor validation; `identity` owns expected-content and local-baseline establishment; `payload` owns aligned streaming and CPU tensor construction; `transfer_plan` owns `TransferPlan`, its flat `TransferEntryPlan` inventory, and `TransferBatchPlan` ranges; `transfer_batch` owns the live `TransferBatchOwner`; `prepared` coordinates sequential verification and materialization; `construction` borrows the final tensor map and establishes handle-only Llama ownership without allocating a second map; `footprint` consumes the same partition and ownership graph; and `cleanup` owns failed-materialization sole ownership and retryable release.

The large corpora are `config/tests.rs`, `safetensors/tests.rs`, and `scalar/tests.rs` under their production modules rather than inline bodies mixed with parsing or policy. This split changes neither the crate boundary nor hot-path dispatch.

## Failed materialization ownership

`CandleLlamaPreparedLoad` is the ordinary-drop-safe transaction before materialization. Consuming it either returns a complete model or creates the distinct `CandleLlamaFailedPreparation`, whose sole purpose is to retain and explicitly clean resources acquired by the failed attempt. The failed owner contains:

- retained device and parsed configuration;
- open inspected shards;
- completed final tensors;
- the complete current `TransferBatchOwner`, including coordinates, names, byte/count accounting, synchronization/commit state, and every source, optional converted-host, and transferred-device tensor; and
- a completed native model when a later ownership checkpoint fails.

`load_prepared` returns `FailedLoad<CandleLlamaFailedPreparation>` on any materialization, construction, or synchronization failure. It never converts away or aliases the only cleanup owner.

No transferred tensor is committed to the final map before its batch synchronization succeeds. Normal accelerator materialization performs exactly one synchronization per nonempty transfer batch. The locked Candle Llama construction path only installs shallow handles to those synchronized weights, so it enqueues no distinct device work and requires no unconditional final load synchronization.

`FailedLoadOwner::cleanup` is retryable and idempotent after success. It synchronizes the retained device first. A synchronization failure leaves the whole batch, every committed owner, and the failed typestate reachable and unchanged for another attempt. Only successful synchronization clears the model, batch, final tensors, shards, configuration, and device and marks cleanup complete. The failed owner must report the accepted plan for its entire lifetime; E0 checks that report on both sides of each cleanup attempt and reclassifies any contradiction as unverified ownership. E0 owns bounded retry, peak accounting, exhaustion, and terminal shutdown policy; see [Inference runtime](inference-runtime.md) and [ADR-0020](../agent/decisions/0020-transactional-prepared-model-loading.md).

## Generation lifecycle

The loaded model exclusively owns required execution weights and the selected device. Each sequence owns an independent Candle cache, position, fixed token capacity, and token staging allocation. Sequences do not retain or clone model ownership.

Sequence planning remains arithmetic-only. Creation allocates the cache on the loaded model's retained device. Prefill/decode use that same device, and sequence destruction, model synchronization, and unload preparation explicitly synchronize it. No device/context is created per token.

The compatibility path executes:

```text
prepare exact load
-> admit loading peak
-> sequentially verify shards and materialize required tensors in bounded accelerator batches
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
- retained-file mutation detection and late whole-shard verification.

The package's dedicated harness-free `cuda_hardware` target runs the complete reviewed adapter hardware suite: explicit CPU execution in a CUDA build, invalid ordinal rejection, F32 CUDA/CPU logits, homogeneous BF16, mixed F16/F32, and mixed BF16/F32. Its custom runner requires explicit opt-in, registers at least one case, counts every attempt, and cannot silently succeed with zero execution.

Exact current-tree commands and whether CUDA was compiled or executed are recorded in [validation](validation.md) and [implementation status](implementation-status.md). Temporary mixed derivatives and fixture identity are documented beside the committed fixture in [`PROVENANCE.md`](../../crates/runtime/inference-runtime/tests/fixtures/candle-llama/PROVENANCE.md). The 2026-08-08 RTX 5070 Ti execution remains historical evidence for the earlier Phase 12 closure tree unless the amended tree is separately run on hardware.

## Unsupported and deferred work

GGUF and other quantized formats, arbitrary required mixed layouts, required F16+BF16, required unsupported/unknown tensor dtypes, AMD/ROCm, Metal, cuDNN, flash attention, NCCL, multi-GPU execution, and GPU-side sampling remain unsupported. External mixed-checkpoint evidence is absent. Another model architecture or conversion policy requires a separate reviewed matrix rather than weakening this fail-closed path.
