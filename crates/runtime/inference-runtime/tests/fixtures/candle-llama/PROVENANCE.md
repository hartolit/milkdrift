# Synthetic Candle integration fixture provenance

## Purpose and ownership

This directory contains project-authored synthetic test data for download-free Candle execution and E0 lifecycle integration. It is not trained, does not measure language quality, and is not a product-performance model.

No external base-model weights, tokenizer assets, training data, model download, or externally derived tensor values were used. The fixture and its generator are licensed under the repository's Apache-2.0 license.

The E0 integration test `native_backend_generation.rs` remains the fixture's primary owner. `application-runtime` tests canonicalize and load the same committed files for E1 completion, chat, output, cancellation, model cleanup, and unload coverage. `runtime-benchmarks` is an inward non-production consumer: it references this directory in place through `CARGO_MANIFEST_DIR`, verifies the committed sizes/hashes before use, and uses the fixture only for synthetic integration/lifecycle evidence. No production package depends on the benchmark observer.

The fixture remains beside its E0 integration owner because that test defines its compatibility and lifecycle assertions and the reviewed Cargo-native generator defines the committed bytes. Additional consumers read the owned fixture in place rather than copying it.

## Committed deterministic generation

The Cargo-native maintenance generator is:

`crates/adapters/candle-backend/tests/generate_synthetic_fixture.rs`

Run it from the repository root only when intentionally regenerating the committed F32 fixture:

```text
cargo test --locked -p candle-backend --test generate_synthetic_fixture -- --ignored --exact regenerate_committed_candle_fixture
```

The generator performs no network access. It writes one Llama configuration and one Safetensors file using Candle on CPU.

Committed architecture and scalar facts:

- Llama architecture;
- homogeneous observed tensor set `{F32}`;
- configuration declaration F32;
- vocabulary size 16;
- hidden size 8;
- intermediate size 16;
- one hidden layer;
- two attention heads and two key/value heads;
- maximum position count 16;
- 12 tensors total.

Tensor construction is fully deterministic:

- `model.embed_tokens.weight` and `lm_head.weight` are 16 × 8 matrices. Each value is `-0.5` or `0.5`, selected from the corresponding token-identifier bit; the low four bits repeat across the eight dimensions.
- `model.norm.weight`, `model.layers.0.input_layernorm.weight`, and `model.layers.0.post_attention_layernorm.weight` are unit vectors.
- all attention projection and MLP projection matrices are zero matrices.

The ordinary generator test verifies that committed `config.json` is byte-for-byte identical to the generator constant. The deterministic weight generator was executed twice when the replacement fixture was established; the weight blob remains unchanged by this declaration-only amendment.

## Temporary scalar derivatives

Phase 12 does **not** commit additional weight blobs. Download-free tests derive temporary F16/F32 and BF16/F32 variants from project-authored tensor values and delete their temporary directories after use.

The real-adapter E0/CUDA derivative path:

1. loads the committed `model.safetensors` through Candle on CPU;
2. converts each tensor with Candle's safe `to_dtype` operation;
3. writes a temporary Safetensors file beneath a process/nonce-specific operating-system temporary directory;
4. keeps `model.norm.weight` as F32 for a mixed layout while converting every other tensor to F16 or BF16;
5. writes a temporary configuration with the matching F16 or BF16 declaration and constructs an unverified local `CandleLlamaSource`;
6. executes inspection/preparation/load tests against required and observed `{F16,F32}` or `{BF16,F32}`;
7. removes the complete temporary directory from the fixture owner's `Drop` path.

Adapter-local CPU tests also construct equivalent tiny homogeneous and mixed values in temporary directories. Temporary auxiliary tensors may be added only to prove that structurally valid unused tensors remain complete observed artifact evidence while contributing neither materialization/transfers nor final or loading-peak tensor ownership. They are never written into this committed fixture directory.

These derivatives are deterministic in tensor names, shapes, values, selected dtypes, and expected accounting. Their process-specific paths and ephemeral serialized files are test intermediates, not durable evidence artifacts, so no additional committed size/hash table is created for them.

The derivative process uses:

- no network access;
- no external checkpoint or tokenizer;
- no new training/model data;
- no committed F16, BF16, or mixed weight file;
- no claim of representative model quality, scale, throughput, or external-checkpoint compatibility.

The temporary derivatives prove the reviewed scalar-layout, conversion, accounting, generation, cleanup, and unload contracts only. Guarded CUDA tests still require an actual separately recorded hardware run before they become hardware evidence.

## Committed files

| File | Size | SHA-256 |
|---|---:|---|
| `config.json` | 382 bytes | `e30225f7b8cbeb18c6fe2e9f623e87bd5d7cec3e28dd7e23a3f36ee107c69c4d` |
| `model.safetensors` | 4,800 bytes | `cc4798af93488b4fb2ae0548c2b28ace600521732b52023a7786c3227d72d672` |

No other configuration, tokenizer, index, shard, or weight file is committed for this fixture.

## Replacement audit

The prior fixture first appeared in commit `8de2ebf2811d5158e3439efe2114379de59322d0` on 2026-07-24 under `crates/engines/inference-runtime`, then moved without content changes in commit `f8b3396cc23085696123b95c9dcb4b17c3d9c214` on 2026-07-29. Its files were:

| Prior file | Size | SHA-256 |
|---|---:|---|
| `config.json` | 335 bytes | `6c27e4687ddb94eea5e180e7d2e679826c4ccb1b7224945aab9f013607704b7a` |
| `model.safetensors` | 4,800 bytes | `a4407aa5c225725d3ea9036e41734533af33b95a0c778309858feed003c2a64c` |

The prior tensor bytes exactly matched the deterministic shape and values of a generator added in the same squash, and they did not match the separately downloaded random model found in that historical commit. However, the squash did not record a fixture-specific authorship attestation, source invocation, license statement, or authorized chain of title. Redistribution permission therefore could not be established from repository evidence alone.

The prior bytes were not accepted on size or apparent synthetic structure. They were replaced by the newly generated project fixture documented above. Phase 12 derives temporary variants only from this replacement fixture or the same project-authored deterministic tensor specification; it does not reuse the provenance-uncertain bytes.
