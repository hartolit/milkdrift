# candle-backend

CPU-default and feature-gated CUDA adapter for unquantized Hugging Face Llama models stored as Safetensors. The crate owns Candle, Safetensors, filesystem, and `cudarc` details and implements the portable contracts from `domain-contracts`.

## Source API and artifact identity

`CandleLlamaSource` accepts a configuration path and one or more identity-bearing shards:

```rust,ignore
CandleLlamaSource::new(config_path, Vec<CandleWeightShard>)
```

Each `CandleWeightShard` keeps its path paired with either an exact content expectation or an explicit unverified-local state while the source sorts complete pairs deterministically:

- `CandleWeightShard::with_expected_content(path, CandleExpectedContentIdentity::new(byte_length, sha256))` supplies only the exact bytes Candle must observe. It does not claim who established that expectation or that the path is immutable;
- `CandleWeightShard::unverified_local(path)` supplies no reusable identity, so Candle establishes its own retained-file baseline.

`CandleLlamaSource::from_local_files(config_path, paths)` is the explicit convenience for unverified mutable local files. At least one weight shard is required.

Candle does not trust filenames, symlinks, inode numbers, mtimes, ETags, provider vocabulary, or cache conventions. Inspection opens and retains each file. A supplied expected length must match retained file metadata. A supplied expectation skips Candle's local baseline pass, but never skips exact whole-file verification during sequential materialization.

An unverified local shard uses one bounded-buffer whole-file SHA-256 pass from byte zero before device admission. That pass revalidates the exact inspected prefix/header digest, length, and EOF and records a locally established baseline. Materialization later verifies either the supplied expectation or the local baseline before publication. Diagnostics keep these two establishment states distinct without treating either as provider provenance.

## Strict bounded configuration

The adapter reads `config.json` through a checked allocation with an exact 1 MiB ceiling. The retained bytes are parsed twice: once by a custom duplicate-aware top-level visitor and once as Candle's `LlamaConfig`. The scalar declaration is therefore derived from the same exact bytes as the executable configuration; callers cannot inject it.

The declaration policy for modern `dtype` and legacy `torch_dtype` is fail-closed:

- both absent or null: no declaration;
- one recognized and the other absent/null: use the recognized declaration;
- both recognized and equal: use that declaration;
- either present with an unsupported string: fail explicitly, with no fallback;
- both recognized but different: fail explicitly;
- duplicate fields, wrong JSON types, or malformed JSON: fail explicitly.

Recognized declarations are F32 (`float32`/`f32`), F16 (`float16`/`half`/`f16`), and BF16 (`bfloat16`/`bf16`) under ASCII case normalization. Raw producer strings never cross the adapter boundary.

Configuration must explicitly identify Llama: `model_type` must be `llama` (ASCII case normalization is accepted). If `architectures` is present and non-null, it must be a nonempty array containing only `LlamaForCausalLM` and/or `LlamaModel`. Missing, contradictory, non-Llama, duplicated, or malformed architecture identity is rejected. Existing nonzero/divisibility validation remains, and hidden layers are capped at 256 before required tensor names are generated.

## Four scalar facts

The adapter keeps four meanings separate:

1. **Configuration declaration** — optional F32/F16/BF16 producer intent derived from bounded config bytes.
2. **Complete observed set** — `ModelMetadata::observed_tensor_scalar_types`, populated from every structurally valid tensor in every shard.
3. **Required scalar set** — adapter-private categories from only tensors consumed by the supported Candle Llama schema.
4. **Execution scalar** — selected during device-aware preparation and reported in the load plan and loaded model.

Every Safetensors 0.8 dtype is represented structurally. F32, F16, BF16, I8, and U8 map directly to portable categories. BOOL, F4/F6, all FP8 variants, wider integers, F64, C64, and the remaining understood formats map to stable adapter-owned `ScalarType::Other(code)` categories; `ScalarTypeSet` intentionally collapses those codes into its single `Other` bit.

Unused understood tensors may use any of those dtypes. They remain complete observed evidence but do not select precision and do not fail merely because Candle cannot execute them. A required tensor must be F32, F16, or BF16; any required other dtype fails before device initialization.

The required-primary matrix is exact:

| Required scalar set | Required primary | Permitted declaration |
|---|---:|---:|
| `{F32}` | F32 | absent or F32 |
| `{F16}` | F16 | absent or F16 |
| `{F16,F32}` | F16 | **F16 required** |
| `{BF16}` | BF16 | absent or BF16 |
| `{BF16,F32}` | BF16 | **BF16 required** |

Empty, F16+BF16, and any required set containing another category are rejected. Complete observed extras never affect this matrix. A mixed required set is deliberately not self-describing: `{F16,F32}` and `{BF16,F32}` do not reveal which dtype is producer-intended primary. Milkdrift therefore requires the matching recognized declaration before performing a lossy conversion; an absent declaration is accepted only for homogeneous required sets.

Execution selection remains:

| Required primary | CPU | CUDA |
|---|---:|---:|
| F32 | F32 | F32 |
| F16 | F16 | F16 |
| BF16 | F32 | BF16 only when the selected device reports support |

CPU BF16 sources execute as F32 because the supported Candle CPU matmul path does not accept BF16 operands. CUDA is available only behind the non-default `cuda` feature and is always explicitly selected; it never falls back to CPU.

## Bounded full manifest inspection

`inspect()` reads only bounded configuration bytes plus each shard's 8-byte prefix and bounded JSON header. It never scans weight payloads and never initializes an execution device. Nevertheless, every shard and every tensor header is parsed and validated before preparation can initialize a device.

One private injectable `InspectionLimits` has these production values:

| Structure | Production ceiling |
|---|---:|
| selected shards | 256 |
| one shard header | 8 MiB |
| aggregate headers | 64 MiB |
| tensors | 16,384 |
| one tensor name | 512 bytes |
| aggregate tensor-name bytes | 8 MiB |
| tensor rank | 8 |
| one shape dimension extent | 1,048,576 |
| aggregate shape dimensions | 131,072 |
| metadata entries | 1,024 |
| one metadata key | 256 bytes |
| one metadata value | 4 KiB |
| aggregate metadata string bytes | 4 MiB |
| final owned inspection inventory | 64 MiB |

Custom Serde visitors/seeds reject tensor, name, rank, individual dimension extent, aggregate shape, metadata, and retained-inventory growth while traversing the JSON. Tensor shapes use fixed rank-8 storage. Checked reservations map allocation failure deterministically.

Inspection preserves duplicate JSON/tensor detection, cross-shard duplicate detection, deterministic offset order, exact contiguous Safetensors offsets (no overlap or gap), exact payload/file bounds, checked element counts, and checked bit-packed byte calculations. Bit-packed tensors must occupy an integral number of bytes, matching Safetensors validation.

Parsed metadata strings and header/config buffers are non-tensor resources governed by the ceilings above. Metadata is discarded after validation. They are deliberately excluded from `MemoryFootprint` because they are independently and strictly bounded.

## One-pass selective materialization and bounded transfer batches

After identity establishment and admission, each retained shard is processed by exactly one sequential pass from byte zero:

1. read and hash the prefix/header through one checked fixed 64 KiB verification buffer;
2. compare the exact retained prefix/header digest before processing payload bytes;
3. read and hash unused tensor ranges through the same 64 KiB buffer without allocating or constructing tensors;
4. read each required payload once into one checked aligned staging allocation;
5. construct its CPU source tensor and independently cast when required;
6. retain CPU execution tensors directly, or enqueue accelerator transfers into the current private transfer batch;
7. close full intermediate accelerator batches only after endpoint validation and one synchronization, move entries into final ownership while tracking per-entry commit state, and only after the complete batch commits release its host staging;
8. retain the shard's final planned batch unsynchronized while verifying exact EOF, length, and accepted whole-file SHA-256;
9. only after that identity succeeds, complete endpoint validation and one synchronization, then commit the shard-final batch; and
10. after every shard verifies, construct the Llama model from the committed tensor handles.

Accelerator batches follow one deterministic `TransferPlan` shared by planning and materialization. `PREFERRED_BATCH_HOST_STAGING_BYTES` is 256 MiB and `MAXIMUM_BATCH_ENTRIES` is 64. A batch ends before adding an entry that would exceed either policy and at every shard boundary. The first tensor in an empty batch is always admitted, so a tensor larger than the preferred byte target becomes a bounded singleton rather than being rejected solely because of that target. No batch crosses shards. A shard boundary fixes the final batch membership; synchronization/commit of that batch is deliberately deferred until the whole-shard identity succeeds.

There are no per-tensor seeks, no mmap, and no unsafe code. Unused tensors are never staged into tensor-sized storage, converted, transferred, inserted into Candle's load map, or retained by the model. Header mutation fails before payload processing. Payload mutation, extension, and truncation fail before model publication. A late digest mismatch consumes the ordinary-drop-safe preparation and returns a distinct `CandleLlamaFailedPreparation` as the sole cleanup owner of all tensors already materialized and the complete current batch.

## Exact load footprints

`MemoryFootprint` accounts for deterministic logical tensor ownership, aligned payload allocation bounds, the fixed 64 KiB verification buffer that is actually live during shard materialization, and the actual retained heap capacities of the accelerator plan/owner vectors. Parsed config/header/inventory metadata, required-name/map metadata, allocator bookkeeping/fragmentation, process RSS, and accelerator driver/context allocations remain outside it. Required maps are independently capped by 16,384 tensor entries, 512 bytes per name, 8 MiB aggregate names, and the 64 MiB retained inventory ceiling. Tensor names move from the bounded manifest into entries and then the final map instead of gaining another heap allocation.

For required tensors in deterministic materialization order, define:

- `S_i`: exact serialized source bytes, including checked bit-packed calculation;
- `E_i`: execution bytes;
- `align_i`: executable source alignment (2 for F16/BF16, 4 for F32);
- `A_i = S_i + align_i - 1`: aligned staging allocation bound;
- `P_i`: required execution bytes retained before tensor `i`;
- `R = sum(E_i)`: all required final execution weights;
- `V = 64 KiB`: the shard-verification buffer;
- `M_plan = capacity(TransferPlan.batches) * size_of::<TransferBatchPlan>() + capacity(TransferPlan.entries) * size_of::<TransferEntryPlan>()` for the immutable flat plan plus batch ranges;
- `M_owner = capacity(TransferBatchOwner.entries) * size_of::<TransferBatchEntry>()` for the reusable owner;
- `M = M_plan + M_owner`, using the actual checked capacities retained by this preparation;
- `C`: exact KV-cache bytes per token at execution width.

Unused tensors do not enter any formula.

### CPU

CPU allocates neither `TransferPlan` nor `TransferBatchOwner`; its sequential path therefore has no `M` term.

```text
Hcpu = max(
    R,
    V,
    max_i(V + P_i + A_i + S_i),
    max_cast_i(V + P_i + S_i + E_i)
)
```

Final CPU footprint:

```text
host weights = R
host working = 0
device weights = 0
device working = 0
cache bytes/token = C
```

Loading CPU footprint:

```text
host weights = R
host working = Hcpu - R
device weights = 0
device working = 0
cache bytes/token = C
```

### Accelerator transfer path

For tensor `i` in accelerator batch `b`, define:

- `Q_i = S_i` when no cast is required, otherwise `Q_i = S_i + E_i`; this is the host tensor payload retained after its transfer is enqueued;
- `C_b,i = sum(Q_j)` for entries already retained in that batch before `i`; and
- `W_b` as the maximum simultaneous host staging within the batch:

```text
W_b = max(
    sum_i(Q_i),
    max_i(C_b,i + A_i + S_i),
    max_cast_i(C_b,i + S_i + E_i)
)

Haccelerator = M + V + max_b(W_b)
```

`M` and `V` are additive because the plan/owner capacities remain allocated for the preparation and the verifier remains allocated while every batch is staged and while a shard-final batch waits for identity verification and then synchronizes/commits. The 256 MiB preference is applied to projected `W_b`; it is not an unrelated margin and does not cap the admitted oversized singleton described above. Host source and cast tensors remain live through synchronization and the complete batch commit. During commit the batch entry and final map temporarily hold shallow handles to the same device storage, so the payload is counted once. Transferred-but-uncommitted storage, committed storage, and later Llama handles together never exceed `R` distinct logical execution bytes.

Final accelerator footprint:

```text
host weights = 0
host working = 0
device weights = R
device working = 0
cache bytes/token = C
```

Loading accelerator footprint:

```text
host weights = 0
host working = Haccelerator
device weights = R
device working = 0
cache bytes/token = C
```

All arithmetic is checked. There is no extra headroom for ignored tensors. CUDA is the currently implemented accelerator path, but batching, ownership, and accounting remain Candle-adapter policy; E0 receives only portable plan, device, ownership, and cleanup facts. No NVIDIA-specific batching contract is added, and another accelerator implementation may reuse the adapter-internal policy only where its transfer and synchronization semantics permit.

## Sequence reservation follows simultaneous lifetimes

`SequencePlan::reservation` separates persistent, additional transient, and checked-total host/device footprints. The total is a checked conservative upper bound over reviewed live logical tensor payload and source-transfer bytes. It is not physical RSS/VRAM and does not sum mutually exclusive transient phases. The locked Candle Llama 0.11.0 path separates:

- **persistent per-layer ownership** — KV cache for every transformer block, plus retained rotary and mask caches;
- **one-block transient peak** — attention, normalization, residual, MLP, conversion, and matmul tensors that can coexist inside one `Block::forward`;
- **outer model peak** — embedding/current hidden state, one block result, final normalization, selected final-token state, logits, and F32 logit conversion; and
- **creation/source-transfer phases** — token/mask host staging, device cache construction, and CUDA host-logit transfer.

Candle advances blocks sequentially (`x = block.forward(...)`), so persistent KV ownership scales with `num_hidden_layers`, while the complete block transient peak is admitted once rather than multiplied by layer count. CPU and CUDA take the component-wise maximum of creation and execution phases in their own memory domains, then add that transient headroom to persistent ownership. Realistic 22-layer TinyLlama regression arithmetic locks this distinction so transformer depth cannot silently multiply transient activation headroom again. Caller-owned E0 logits/sampling workspaces remain separately admitted. Exact package/version/checksum and actual Cache/KV/mask behavior tests gate any Candle upgrade; the full source-derived phase table is in the project Candle-backend documentation.

## Transactional ownership and cleanup

`CandleLlamaPreparedLoad` is the ordinary-drop-safe, pre-materialization transaction. `load_prepared` consumes it exactly once. If materialization acquires resources and then fails, the adapter returns the distinct `CandleLlamaFailedPreparation` typestate; only that failed typestate implements `FailedLoadOwner` and exposes retryable cleanup. This prevents the public API from assigning incompatible drop semantics to one type.

The failed owner retains:

- parsed config and selected device;
- every retained open shard and accepted whole-file identity;
- every completed required tensor in the final map;
- the complete current `TransferBatchOwner`, including entry coordinates/names/accounting/commit state and every source, optional converted-host, and transferred-device tensor; and
- a constructed model if a later ownership checkpoint fails.

Each endpoint enters the batch owner before subsequent fallible validation, synchronization, insertion, or publication work. No transferred tensor enters the final map before the batch synchronization succeeds. Normal accelerator loading synchronizes exactly once per nonempty transfer batch. The locked Candle Llama construction path only creates shallow handles to already synchronized weights, so it enqueues no distinct device work and has no redundant final load synchronization. Failure cleanup has its own synchronization boundary before explicit release.

`FailedLoadOwner::cleanup` is retryable and idempotent: synchronization failure leaves the complete batch, every committed tensor, every other handle, and the lifetime-stable accepted plan intact for another attempt; only successful synchronization clears the model, batch, tensors, shards, config, and device. The raw Candle owner performs no hidden abandonment. The project-owned `FailedLoad` guard encapsulates it and deliberately retains an unresolved owner if a direct caller abandons the failure before cleanup succeeds; E0 normally remains the reachable owner and performs bounded retries. E0 verifies the failed owner's plan before and after every cleanup attempt; report substitution or mutation becomes unverified retained ownership and blocks admission until release.

## Loader organization

Production responsibilities stay adapter-private and cohesive: `config` and `safetensors` decode bounded input; `manifest` retains inspected layout; `identity` establishes exact content; `configuration_policy`, `scalar`, and `schema` decide supported execution; `payload` owns aligned reading and CPU tensor creation; `transfer_plan` owns `TransferPlan`, flat `TransferEntryPlan` inventory, and `TransferBatchPlan` ranges; `transfer_batch` owns the live `TransferBatchOwner`; `prepared` coordinates the sequential transaction; `construction` borrows the final map and creates handle-only Llama ownership without allocating a second tensor map; `footprint` consumes the same partition/lifetimes; and `cleanup` owns failed-load release.

The large corpora live in `config/tests.rs`, `safetensors/tests.rs`, and `scalar/tests.rs` instead of dominating their production modules. This organization adds no hot-path dynamic dispatch and moves no Candle policy into E0 or E1.

## Stable failure details

Backend failures use project-owned numeric details rather than retaining vendor strings. Existing codes remain stable; artifact-loading additions include:

| Code | Meaning |
|---:|---|
| 32 | accepted whole-shard SHA-256 mismatch |
| 33 | configuration byte/layer ceiling |
| 34 | bounded configuration allocation |
| 35 | malformed/duplicate scalar declaration |
| 36 | unsupported present scalar declaration |
| 37 | conflicting recognized declarations |
| 38 | missing/malformed/contradictory architecture identity |
| 39 | per-shard or aggregate header ceiling |
| 40 | tensor/name/rank/shape structural ceiling |
| 41 | metadata structural ceiling |
| 42 | final inspection inventory ceiling |
| 43 | bounded inspection allocation |
| 44 | retained prefix/header identity mismatch |
| 45 | retained/supplied shard length mismatch |
| 46 | required tensor-map allocation |

## Runtime scope

The adapter supports CPU device ID 0 by default and explicit CUDA ordinals behind `candle-backend/cuda`. It supports unquantized Hugging Face Llama configuration plus one or more Safetensors shards, independent sequence caches, prompt prefill, incremental decode, synchronization, and unload preparation.

It does not advertise `CapabilitySet::ALLOCATION_FREE_HOT_PATH`: Candle's upstream Llama implementation allocates forward intermediates and grows KV-cache tensors. It does not advertise `CapabilitySet::SEQUENCE_RESET`; destroy and recreate a sequence at a cold lifecycle boundary.

Model downloading, tokenizer integration, GGUF/quantized formats, AMD/ROCm, Metal, cuDNN, flash attention, NCCL, multi-GPU execution, and GPU-side sampling remain outside this adapter.
