# Current Implementation Status

**Status date:** 2026-07-25

**Source baseline:** Phase 4 implementation commit `8de2ebf2811d5158e3439efe2114379de59322d0` plus the lifecycle, provenance, and fixture-hygiene closure patch

**Execution position:** Phase 4 implementation complete; final locked validation and the pinned smoke must be rerun on the closure commit before Phase 5 begins

**Canonical plan:** [LLM App Execution Plan](../execution/execution-plan.md)

This document is the canonical statement of what the delivered source tree claims. It deliberately separates implemented source, recorded baseline evidence, and validation still required after source changes.

## Supported devices and backends

| Backend | Device | Adapter/E0 boundary | `application-runtime` (E1) | Slint UI |
|---|---|---:|---:|---:|
| Candle 0.11 Llama/Safetensors | CPU | Phase 4 vertical slice implemented | Yes, lifecycle composition | Yes, lifecycle only |
| GGUF via llama.cpp | CPU | Lifecycle and backend primitives | No | No |
| Candle or GGUF | CUDA/Metal/other GPU | No supported product path | No | No |

The repository remains CPU-only. Candle Llama is driven through the E0 generation scheduler; E1 and the UI still expose lifecycle only.

## Completed Phase 3 foundation

The Phase 3 completion report records a passing canonical locked validation for its exact source tree. That foundation includes worker-owned prefill/sampling/decode, bounded output, cancellation and stop conditions, exact admission accounting, deterministic cleanup quarantine and retry exhaustion, and fault-injection coverage.

The Phase 4 implementation preserves those generic contracts and changes Candle-specific behavior only at the adapter and integration-test boundaries.

## Phase 4 implementation

The delivered source includes:

- deterministic Candle CPU fixture semantics for prompt positions, final-position prefill logits, independent incremental decode progression, cancellation boundaries, exact vocabulary output, explicit sequence destruction, and unload;
- F32 and F16 native CPU execution plus BF16 source compatibility through admission-accounted F32 upcasting; Candle logits are normalized to the backend-independent F32 output contract;
- actual Candle Llama execution through the hosted E0 generation scheduler, including token-limit completion, EOS completion, bounded output backpressure, cancellation, terminal/released publication, exact accounting release, model unload, a final empty post-unload snapshot, shutdown, and worker join;
- an opt-in pinned real-model smoke example that accepts local model files and caller-supplied token IDs, verifies the loaded architecture and context, produces tokens through E0, cancels a second generation, checks release and post-unload accounting, unloads, and records diagnostic timings and RSS;
- a reproducible smoke procedure at [Phase 4 Candle Llama Smoke Procedure](../execution/phase4-candle-smoke.md).

Ordinary CI remains download-free: Candle integration tests use the tiny project-authored Safetensors fixture under `crates/engines/inference-runtime/tests/fixtures/`. The pinned external smoke model is downloaded into ignored `.phase4/` storage and is not committed.

## Integration depth

| Capability | E0 inference runtime | E1 application runtime | Slint UI |
|---|---:|---:|---:|
| Model load, generation-safe handle, drain, cancellation, unload | Yes | Yes for Candle lifecycle | Yes for Candle lifecycle |
| Backend prefill and decode primitives | Yes | Not exposed as generation | No |
| Backend-independent generation scheduler | Fake backend plus Candle CPU integration | Not exposed | No |
| Sampling algorithm | Integrated inside E0 | Not exposed | No |
| Bounded streamed token output | Pull-oriented token/state batches | No | No |
| Direct-completion real-model loop | Opt-in E0 smoke | No | No |
| Tokenization and decoded text streaming | Separate foundations only | Not integrated | No |
| General chat templates/history | No | No | No |

## Validation evidence and remaining gate

The recorded Phase 4 baseline locally passed:

```text
cargo run --locked --bin llm-app -- verify
```

It also passed the pinned smoke for `neubla/tiny-random-LlamaForCausalLM` revision `39ca1f8a1fc940377c5cb49a21aff73bb99b52f5`; the complete recorded diagnostics are retained in the [Phase 4 closure report](../execution/PHASE4_IMPLEMENTATION_REPORT.md). That evidence was captured before the squashed Phase 4 commit and is local evidence rather than a GitHub Actions result.

The closure patch changes source and tests by making successful Candle destruction transition the sequence to `Finished` and by asserting a completely empty runtime/model snapshot after unload. Therefore the final closure commit must rerun both gates before Phase 4 is formally closed:

```text
cargo run --locked --bin llm-app -- verify

MODEL_DIR="$PWD/.phase4/tiny-random-llama"
MODEL_REVISION="39ca1f8a1fc940377c5cb49a21aff73bb99b52f5"
export LLM_APP_CANDLE_MODEL_DIR="$MODEL_DIR"
export LLM_APP_CANDLE_MODEL_REVISION="$MODEL_REVISION"
export LLM_APP_CANDLE_PROMPT_TOKENS="1,2,3"
cargo run --locked -p inference-runtime --example candle_llama_smoke
```

Record `git rev-parse HEAD` together with the complete command output. Phase 5 begins only after both commands pass on that exact commit.

## Known limitations

- The E0 request starts from caller-supplied token IDs and emits token IDs. Tokenizer ownership, incremental text decoding, E1 generation commands, and frontend pulls remain Phase 5 work.
- The selected smoke fixture is a tiny random test model. It proves architecture and lifecycle integration, not output quality.
- The deterministic cleanup policy uses a total-attempt limit, not wall-clock retry backoff. Exhausted resources remain quarantined and accounted.
- Strict allocation-free Candle execution is not claimed because upstream Candle allocates intermediate and KV-cache tensors.
- GPU execution, remote/browser transport, general chat, GGUF UI selection, and multi-model E1 state are unsupported.

## Historical implementation record

The recovered [implementation plan](implementation-plan.md) is retained as historical context and is not authoritative. The execution plan supersedes its old phase sequence and proposed repository shape.
