# Validation

This document owns repeatable project validation procedures. [Implementation status](implementation-status.md) records whether the current source baseline has passed the required gates; [execution history](../agent/execution/history.md) preserves older run evidence.

## Canonical repository gate

Run from the repository root on the exact tree being evaluated:

```sh
cargo run --locked --bin llm-app -- verify
```

Use focused commands to diagnose a failure without treating them as a substitute for the canonical gate:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo bench --workspace --no-run --locked
git diff --check
```

Dependency/supply-chain gates are documented in [dependency policy](dependency-policy.md). Cross-target checks are documented in [portability](portability.md). Performance measurements have separate methodology in [performance evidence](performance.md).

For validation evidence, record the exact source revision:

```sh
git rev-parse HEAD
```

A successful command on a different commit is not evidence for the current tree.

## GGUF fixture and shared native E0 suite

The repository fixture is
`crates/runtime/inference-runtime/tests/fixtures/gguf-llama/tiny-llama-f32.gguf`.
It is a deterministic 6,144-byte GGUF v3 F32 Llama model with one block, a
16-token context, and a complete 16-token test vocabulary. Its expected SHA-256
is `c3e55952008029142e0db9cf18674657c5827b67c4c221d6beced60d7d144ac7`.

The adjacent
`crates/runtime/inference-runtime/tests/fixtures/gguf-llama/generate_gguf.py`
uses only the Python standard library. It verifies the hashes and complete tensor
schema of the committed Candle fixture, applies the required Q/K RoPE row
permutation, and emits deterministic aligned GGUF v3 bytes. No network access,
model download, or Python package installation is required.

### Verify or intentionally rebuild the fixture

Run the non-mutating reproducibility check from the repository root:

```sh
python3 crates/runtime/inference-runtime/tests/fixtures/gguf-llama/generate_gguf.py --check
```

`--check` regenerates the bytes in memory, compares them byte for byte with the
committed fixture, and reports the size and SHA-256. If an intentional source
fixture change requires rebuilding the committed GGUF, run the generator without
`--check` and then repeat the check:

```sh
python3 crates/runtime/inference-runtime/tests/fixtures/gguf-llama/generate_gguf.py
python3 crates/runtime/inference-runtime/tests/fixtures/gguf-llama/generate_gguf.py --check
```

### Run the focused adapter and E0 tests

The tokenizer test loads a vocabulary-only llama.cpp model and exercises digest
identity, portable encode/decode policy, independent BOS/EOS handling, special
skipping, and borrowed/owned stateful decoder construction:

```sh
cargo test --locked -p gguf-backend --test tokenizer
```

The shared native suite is generic over the loader and runs the same contract for
both the committed Candle and GGUF fixtures:

```sh
cargo test --locked -p inference-runtime --test native_backend_generation
```

To isolate the GGUF/llama.cpp instance while diagnosing a native failure:

```sh
cargo test --locked -p inference-runtime --test native_backend_generation \
  gguf_runs_shared_native_backend_suite -- --exact --nocapture
```

The suite covers real model load and `RuntimeCommand::Generate`, prompt prefill,
incremental decode, deterministic greedy output, seeded repeatability, EOS and
token-limit completion, backpressure, cancellation, sequence/workspace cleanup,
unload with empty accounting, terminal shutdown, and worker join. It is
load-and-generation proof rather than compile-only coverage.

These committed fixtures and focused tests require no download and make no claim
about language quality, GPU execution, or allocation-free backend behavior. They
also do not establish the full repository gate. Do not record a current-tree gate
pass until the main agent runs the
[canonical repository gate](#canonical-repository-gate) on the exact tree being
evaluated.

## Candle real-model smoke

The external Candle smoke is an opt-in integration check for the real Llama/Safetensors path through the hosted E0 worker. Ordinary workspace tests use project-authored fixtures and do not download a model.

### Pinned fixture

| Field | Required value |
|---|---|
| Repository | `neubla/tiny-random-LlamaForCausalLM` |
| Revision | `39ca1f8a1fc940377c5cb49a21aff73bb99b52f5` |
| Expected architecture | Hugging Face `LlamaForCausalLM`; runtime `ModelArchitecture::Llama` |
| Scalar type | F32 |
| Required files | `config.json`, `model.safetensors` |
| `model.safetensors` SHA-256 | `49c20f32c6c597480fcaec5df2f86c645eabea765cbea1e67886dbae45e5c992` |

The fixture is a tiny random model. It validates execution and lifecycle integration, not language quality.

Downloaded model files are local validation inputs, not repository fixtures. Store them in ignored `.phase4/` storage and do not commit the config, weights, or machine-specific transcript.

### Download the exact revision

Install the Hugging Face `hf` CLI, then run:

```sh
MODEL_DIR="$PWD/.phase4/tiny-random-llama"
MODEL_REVISION="39ca1f8a1fc940377c5cb49a21aff73bb99b52f5"

mkdir -p "$MODEL_DIR"
hf download neubla/tiny-random-LlamaForCausalLM \
  config.json model.safetensors \
  --revision "$MODEL_REVISION" \
  --local-dir "$MODEL_DIR"

printf '%s  %s\n' \
  '49c20f32c6c597480fcaec5df2f86c645eabea765cbea1e67886dbae45e5c992' \
  "$MODEL_DIR/model.safetensors" \
  | sha256sum --check --strict -
```

On a platform without `sha256sum`, use an equivalent SHA-256 tool and compare the complete digest. Use the full pinned revision; do not replace it with `main` or an abbreviated web-interface hash.

### Run the smoke

The default prompt is token IDs `1,2,3`. A different non-empty comma-separated sequence may be supplied, up to 32 prompt tokens.

```sh
export LLM_APP_CANDLE_MODEL_DIR="$MODEL_DIR"
export LLM_APP_CANDLE_MODEL_REVISION="$MODEL_REVISION"
export LLM_APP_CANDLE_PROMPT_TOKENS="1,2,3"

cargo run --locked \
  -p inference-runtime \
  --example candle_llama_smoke
```

The example drives this lifecycle through the hosted E0 worker:

```text
load pinned local Llama
→ admit token-level generation
→ prefill, sample, and incrementally decode eight tokens
→ publish terminal and released records
→ admit a second request
→ force one-token output backpressure
→ cancel between backend calls
→ publish Released(Cancelled(UserRequested))
→ verify request/workspace/cleanup accounting is empty
→ unload the model
→ verify loaded-model, request, workspace, cleanup, memory, and model-list state is empty
→ shut down and join the worker
```

The caller only pulls bounded output to relieve backpressure. It never drives backend prefill or decode one token at a time.

### Expected diagnostic output

A successful run reports:

- exact repository, revision, and expected architecture;
- prompt token count and generated token IDs;
- model load duration;
- time to first generated token;
- decode throughput after the first token;
- cancellation latency;
- model unload duration;
- process RSS before load, after load, at first generated token, and after unload.

RSS is read from `/proc/self/status`. Non-Linux platforms report it as unavailable rather than failing the lifecycle smoke. RSS observations are diagnostic evidence, not portable benchmark thresholds or proof of allocator release.

### Failure classification

The executable separates failures into:

- `configuration error` — missing environment variables, wrong pinned revision, missing files, invalid token syntax, or an oversized prompt;
- `runtime error` — adapter inspection/load failure, descriptor mismatch, generation admission/execution failure, missing terminal/release records, retained accounting, unload failure, non-empty post-unload state, or worker shutdown failure.

A successful download alone does not validate the integration. Record the complete smoke output together with `git rev-parse HEAD` so the evidence is tied to one source revision.
