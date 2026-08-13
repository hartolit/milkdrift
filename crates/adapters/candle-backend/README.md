# candle-backend

CPU-default and feature-gated CUDA adapter for unquantized Hugging Face Llama models stored as Safetensors. The crate owns Candle, Safetensors, filesystem, and `cudarc` details and implements the portable contracts from `domain-contracts`.

## Source API and artifact identity

`CandleLlamaSource` accepts a configuration path and one or more identity-bearing shards:

```rust,ignore
CandleLlamaSource::new(config_path, Vec<CandleWeightShard>)
```

Each `CandleWeightShard` keeps its path paired with one `CandleShardIdentity` while the source sorts complete pairs deterministically:

- `VerifiedImmutable { byte_length, sha256 }`: identity came from a source whose content-addressing and immutability semantics were independently verified;
- `ProjectEstablished { byte_length, sha256 }`: trusted project code computed the complete-file identity, without claiming that the path itself is immutable;
- `Unverified`: no reusable identity is available.

`CandleLlamaSource::from_local_files(config_path, paths)` is the explicit convenience for unverified mutable local files. At least one weight shard is required.

Candle does not trust filenames, symlinks, inode numbers, mtimes, ETags, or cache conventions. Inspection opens and retains each file. A supplied identity length must match retained file metadata. Only `VerifiedImmutable` skips a baseline payload pass. Its expected cryptographic identity is verified during the one sequential materialization pass.

`ProjectEstablished` and `Unverified` are mutable-source fallbacks. Before device initialization or admission, `ProjectEstablished` is rehashed from Candle's retained file and compared with the supplied digest, while `Unverified` establishes a fresh baseline. Both use one sequential whole-file SHA-256 pass from byte zero that revalidates the exact inspected prefix/header digest, exact length, and EOF. Materialization later verifies the retained file against that admitted identity before publication.

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

## One-pass selective materialization

After identity establishment and admission, each retained shard is processed by exactly one sequential pass from byte zero:

1. read and hash the prefix/header through one checked fixed 64 KiB verification buffer;
2. compare the exact retained prefix/header digest before processing payload bytes;
3. read and hash unused tensor ranges through the same 64 KiB buffer without allocating or constructing tensors;
4. read each required payload once into one checked aligned staging allocation;
5. construct its CPU source tensor, independently cast if required, and independently retain on CPU or transfer/synchronize on CUDA;
6. at EOF, verify exact length and the accepted whole-file SHA-256;
7. only after every shard verifies, construct and synchronize the Llama model for publication.

There are no per-tensor seeks, no mmap, and no unsafe code. Unused tensors are never staged into tensor-sized storage, converted, transferred, inserted into Candle's load map, or retained by the model. Header mutation fails before payload processing. Payload mutation, extension, and truncation fail before model publication. A late digest mismatch consumes the ordinary-drop-safe preparation and returns a distinct `CandleLlamaFailedPreparation` as the sole cleanup owner of all tensors already materialized.

## Exact required-only footprints

`MemoryFootprint` accounts for deterministic tensor ownership and cache bytes. It excludes the separately bounded 64 KiB verification buffer, parsed config/header/inventory metadata, required-name/map metadata, allocator bookkeeping/fragmentation, process RSS, and CUDA driver/context allocations. Required map growth is independently capped by 16,384 tensor entries, 512 bytes per name, 8 MiB aggregate names, and the 64 MiB retained inventory ceiling. At most one 512-byte required-name clone is in flight during materialization; model construction temporarily owns one additional map with at most the same entry/name bounds so the prepared value retains all original tensor handles if construction fails. Hash-map bucket overhead is platform-dependent but bounded by the entry ceiling.

For required tensors in deterministic materialization order, define:

- `S_i`: exact serialized source bytes, including checked bit-packed calculation;
- `E_i`: execution bytes;
- `align_i`: executable source alignment (2 for F16/BF16, 4 for F32);
- `A_i = S_i + align_i - 1`: aligned staging allocation bound;
- `P_i`: required execution bytes retained before tensor `i`;
- `R = sum(E_i)`: all required final execution weights;
- `C`: exact KV-cache bytes per token at execution width.

Unused tensors do not enter any formula.

### CPU

```text
Hcpu = max(
    R,
    max_i(P_i + A_i + S_i),
    max_cast_i(P_i + S_i + E_i)
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

### CUDA

```text
Hcuda = max(
    max_i(A_i + S_i),
    max_i(E_i),
    max_cast_i(S_i + E_i)
)
```

Final CUDA footprint:

```text
host weights = 0
host working = 0
device weights = R
device working = 0
cache bytes/token = C
```

Loading CUDA footprint:

```text
host weights = 0
host working = Hcuda
device weights = R
device working = 0
cache bytes/token = C
```

All arithmetic is checked. There is no extra headroom for ignored tensors.

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
- pending source, cast-host, and transferred-device tensor endpoints;
- a constructed model if a later checkpoint or final synchronization fails.

Each endpoint is stored before subsequent fallible validation, synchronization, insertion, or publication work. CPU/CUDA synchronization occurs before explicit release. `FailedLoadOwner::cleanup` is retryable and idempotent: synchronization failure leaves every handle and the lifetime-stable accepted plan intact for another attempt; only successful synchronization clears the model, tensors, shards, config, and device. The raw Candle owner performs no hidden abandonment. The project-owned `FailedLoad` guard encapsulates it and deliberately retains an unresolved owner if a direct caller abandons the failure before cleanup succeeds; E0 normally remains the reachable owner and performs bounded retries. E0 verifies the failed owner's plan before and after every cleanup attempt; report substitution or mutation becomes unverified retained ownership and blocks admission until release.

Private deterministic instrumentation covers hashed ignored ranges versus required materialization events, the immutable fast path versus project-established/unverified mutable baselines, source/cast/transfer/map/model/final-sync ownership checkpoints, mutation and truncation, and cleanup failure/retry. No fault-injection hook is public.

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

Model downloading, tokenizer integration, GGUF/quantized formats, Metal, cuDNN, flash attention, NCCL, multi-GPU execution, and GPU-side sampling remain outside this adapter.
