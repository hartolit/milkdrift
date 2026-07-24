# Phase 4 Candle Llama Smoke Procedure

This is the opt-in real-model validation path for the Phase 4 Candle CPU vertical
slice. It is deliberately separate from ordinary CI: the workspace tests create
small deterministic Safetensors fixtures locally and do not download a model.

## Pinned fixture

| Field | Required value |
|---|---|
| Repository | `neubla/tiny-random-LlamaForCausalLM` |
| Revision | `39ca1f8a1fc940377c5cb49a21aff73bb99b52f5` |
| Expected architecture | Hugging Face `LlamaForCausalLM`; runtime `ModelArchitecture::Llama` |
| Scalar type | F32 |
| Required files | `config.json`, `model.safetensors` |
| `model.safetensors` SHA-256 | `49c20f32c6c597480fcaec5df2f86c645eabea765cbea1e67886dbae45e5c992` |

This is a tiny random test model, not a quality or language benchmark. The smoke
accepts caller-supplied token IDs and prints generated token IDs because tokenizer
and decoded-text integration remain Phase 5 work.

## Download the exact revision

Install the current Hugging Face `hf` CLI, then run from the repository root:

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

On a platform without `sha256sum`, use an equivalent SHA-256 tool and compare the
full digest exactly. The full revision is mandatory; do not replace it with
`main` or the abbreviated commit shown by a web interface.

## Run the smoke

The default prompt is token IDs `1,2,3`. A different non-empty comma-separated
sequence may be supplied, up to 32 prompt tokens.

```sh
export LLM_APP_CANDLE_MODEL_DIR="$MODEL_DIR"
export LLM_APP_CANDLE_MODEL_REVISION="$MODEL_REVISION"
export LLM_APP_CANDLE_PROMPT_TOKENS="1,2,3"

cargo run --locked \
  -p inference-runtime \
  --example candle_llama_smoke
```

The example performs this lifecycle through the hosted E0 worker:

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
→ shut down and join the worker
```

The frontend side only pulls bounded output to relieve backpressure. It never
calls backend prefill or decode and does not advance model execution itself.

## Diagnostic output

A successful run prints:

- exact repository, revision, and expected architecture;
- prompt token count and generated token IDs;
- model load duration;
- time to first generated token;
- decode tokens per second after the first token;
- cancellation latency;
- model unload duration;
- process RSS before load, after load, at first generated token, and after unload.

RSS is read from `/proc/self/status`. Non-Linux platforms report it as unavailable
rather than failing the lifecycle smoke. These numbers are diagnostic evidence,
not optimization claims or portable benchmark results.

## Failure classification

The executable prefixes failures with one of two categories:

- `configuration error`: missing environment variables, wrong pinned revision,
  missing files, invalid token syntax, or an oversized prompt;
- `runtime error`: adapter inspection/load failure, descriptor mismatch, generation
  admission or execution failure, missing terminal/release records, retained
  accounting, unload failure, or worker shutdown failure.

A successful model download alone is not evidence that Phase 4 passes. Record the
complete command output together with the source commit used for the run.
