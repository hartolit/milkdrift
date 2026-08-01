# Synthetic Candle integration fixture provenance

## Purpose and ownership

This fixture is project-authored synthetic test data for download-free Candle execution and E0 lifecycle integration. It is not trained, does not measure language quality, and is not a performance model.

No external base-model weights, tokenizer assets, training data, model download, or externally derived tensor values were used. The fixture and its generator are licensed under the repository's Apache-2.0 license.

The fixture remains here because `native_backend_generation.rs` is its only committed consumer. It must not be copied or promoted to a shared fixture location until a second real consumer exists and sharing avoids duplication.

## Deterministic generation

The Cargo-native maintenance generator is:

`crates/adapters/candle-backend/tests/generate_synthetic_fixture.rs`

Run it from the repository root only when intentionally regenerating the fixture:

```text
cargo test --locked -p candle-backend --test generate_synthetic_fixture -- --ignored --exact regenerate_committed_candle_fixture
```

The generator performs no network access. It writes one Llama configuration and one Safetensors file using Candle on CPU.

Architecture and scalar facts:

- Llama architecture;
- F32 tensors;
- vocabulary size 16;
- hidden size 8;
- intermediate size 16;
- one hidden layer;
- two attention heads and two key/value heads;
- maximum position count 16;
- 12 tensors total.

Tensor construction is fully deterministic:

- `model.embed_tokens.weight` and `lm_head.weight` are 16 × 8 matrices. Each value is `-0.5` or `0.5`, selected from the corresponding token identifier bit; the low four bits repeat across the eight dimensions.
- `model.norm.weight`, `model.layers.0.input_layernorm.weight`, and `model.layers.0.post_attention_layernorm.weight` are unit vectors.
- all attention projection and MLP projection matrices are zero matrices.

Two consecutive generator runs on the recorded tree produced identical bytes.

## Committed files

| File | Size | SHA-256 |
|---|---:|---|
| `config.json` | 360 bytes | `052b5c325859dc723ed0825f711950cbff112a140239953273cebacdb36afdd0` |
| `model.safetensors` | 4,800 bytes | `cc4798af93488b4fb2ae0548c2b28ace600521732b52023a7786c3227d72d672` |

## Replacement audit

The prior fixture first appeared in commit `8de2ebf2811d5158e3439efe2114379de59322d0` on 2026-07-24 under `crates/engines/inference-runtime`, then moved without content changes in commit `f8b3396cc23085696123b95c9dcb4b17c3d9c214` on 2026-07-29. Its files were:

| Prior file | Size | SHA-256 |
|---|---:|---|
| `config.json` | 335 bytes | `6c27e4687ddb94eea5e180e7d2e679826c4ccb1b7224945aab9f013607704b7a` |
| `model.safetensors` | 4,800 bytes | `a4407aa5c225725d3ea9036e41734533af33b95a0c778309858feed003c2a64c` |

The prior tensor bytes exactly matched the deterministic shape and values of a generator added in the same squash, and they did not match the separately downloaded random model found in that historical commit. However, the squash did not record a fixture-specific authorship attestation, source invocation, license statement, or authorized chain of title. Redistribution permission therefore could not be established from repository evidence alone.

The prior bytes were not accepted on size or apparent synthetic structure. They were replaced by the newly generated project fixture documented above.
