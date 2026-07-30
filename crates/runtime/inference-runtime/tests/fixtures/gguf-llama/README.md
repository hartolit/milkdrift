# Tiny F32 Llama GGUF fixture

`tiny-llama-f32.gguf` is a real GGUF v3 model committed for the shared E0
native-backend integration suite. It is intentionally tiny (6,144 bytes): one
Llama block, 8-wide embeddings, 16-wide feed-forward state, a 16-token context,
and a 16-token vocabulary. Every tensor is F32.

## Provenance

The model is generated from the adjacent committed Candle fixture, not from an
external model download:

| Input/output | SHA-256 |
| --- | --- |
| `../candle-llama/config.json` | `6c27e4687ddb94eea5e180e7d2e679826c4ccb1b7224945aab9f013607704b7a` |
| `../candle-llama/model.safetensors` | `a4407aa5c225725d3ea9036e41734533af33b95a0c778309858feed003c2a64c` |
| `tiny-llama-f32.gguf` | `c3e55952008029142e0db9cf18674657c5827b67c4c221d6beced60d7d144ac7` |

The workspace pins `llama-cpp-2`/`llama-cpp-sys-2` `0.1.152`; the latter has
Cargo checksum
`72cd06c8ec4fb02291dbdeac96fb9ecdbf339aa4c9929799aaa56e0362ea3eda`
and package VCS revision
`58b2d048310cfb5fbd2d9c9a0a938f12d61f088a`. Its bundled
`convert_hf_to_gguf.py` was used as the format/name reference, but cannot run
from the packaged source alone here: it imports `torch`, `gguf`, and a
`conversion` module, while the package does not include all of those conversion
modules.

`generate_gguf.py` is therefore the project-owned conversion path. It uses only
Python's standard library, verifies both source hashes and the complete expected
tensor schema, applies the Hugging Face-to-llama.cpp Q/K RoPE row permutation,
renames all tensors to canonical GGUF Llama names, and writes deterministic GGUF
v3 bytes with 32-byte alignment. The zero transformer projections and matching
input/output embeddings preserve the Candle fixture's self-predicting behavior.

The GGUF contains complete tokenizer data for its deliberately small
SentencePiece-style test vocabulary: model/pre-tokenizer identifiers, all token
strings, scores and types, BOS/EOS/unknown/padding IDs, whitespace behavior, and
BOS/EOS insertion flags. Token IDs 1 and 2 remain the Candle config's BOS and
EOS IDs. This vocabulary is sufficient for fixture validation and tiny text
smoke tests; it is not intended as a general-purpose tokenizer.

## Rebuild and verify

From the project root:

```sh
python3 crates/runtime/inference-runtime/tests/fixtures/gguf-llama/generate_gguf.py
python3 crates/runtime/inference-runtime/tests/fixtures/gguf-llama/generate_gguf.py --check
```

The second command performs a byte-for-byte comparison with the committed file.
No network access or Python package installation is required.

## llama.cpp load/generation proof

The GGUF half of the shared native suite loads this file through the pinned
`llama-cpp-2` adapter with mmap disabled, runs prompt prefill and incremental
decode through `RuntimeCommand::Generate`, and checks generated tokens and
terminal lifecycle state:

```sh
cargo test -p inference-runtime --test native_backend_generation \
  gguf_runs_shared_native_backend_suite -- --exact --nocapture
```

That test is the executable llama.cpp load-and-generation proof, not merely a
metadata parser check.

The pinned llama.cpp build rounds the requested 16-token native context arena up
to its 256-token minimum and logs `n_ctx_seq (256) > n_ctx_train (16)`. The GGUF
training metadata and E0 sequence admission intentionally remain at 16 to match
the Candle fixture, and these tests use no more than 14 positions. The warning is
therefore an allocation-floor quirk, not context overflow during the suite.
