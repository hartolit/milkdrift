# Current Implementation Status

**Status date:** 2026-07-23

**Source baseline:** uploaded Phase 3 archive, including `Cargo.lock`; repository VCS metadata was not included

**Execution position:** Phase 4 source package implemented; locked validation and the pinned external-model smoke remain required before Phase 4 completion

**Canonical plan:** [LLM App Execution Plan](../execution/execution-plan.md)

This document is the canonical statement of what the delivered source tree claims. It deliberately separates implemented source from validation evidence.

## Supported devices and backends

| Backend | Device | Adapter/E0 boundary | `application-runtime` (E1) | Slint UI |
|---|---|---:|---:|---:|
| Candle 0.11 Llama/Safetensors | CPU | Phase 4 source path implemented | Yes, lifecycle composition | Yes, lifecycle only |
| GGUF via llama.cpp | CPU | Lifecycle and backend primitives | No | No |
| Candle or GGUF | CUDA/Metal/other GPU | No supported product path | No | No |

The repository remains CPU-only. Candle Llama can now be driven through the E0 generation scheduler at source level; E1 and the UI still expose lifecycle only.

## Completed Phase 3 foundation

The Phase 3 completion report records a passing canonical locked validation for its exact source tree. That foundation includes worker-owned prefill/sampling/decode, bounded output, cancellation and stop conditions, exact admission accounting, deterministic cleanup quarantine and retry exhaustion, and fault-injection coverage.

The Phase 4 patch preserves those generic contracts and changes Candle-specific behavior only at the adapter and integration-test boundaries.

## Phase 4 source implementation

The delivered source adds:

- deterministic Candle CPU fixture semantics for prompt positions, final-position prefill logits, independent incremental decode progression, cancellation boundaries, exact vocabulary output, sequence destruction, and unload;
- F32 and F16 native CPU execution plus BF16 source compatibility through admission-accounted F32 upcasting; Candle logits are normalized to the backend-independent F32 output contract;
- actual Candle Llama execution through the hosted E0 generation scheduler, including token-limit completion, EOS completion, bounded output backpressure, cancellation, terminal/released publication, exact accounting release, model unload, shutdown, and worker join;
- an opt-in pinned real-model smoke example that accepts local model files and caller-supplied token IDs, verifies the loaded architecture and context, produces tokens through E0, cancels a second generation, checks cleanup/accounting, unloads, and records diagnostic timings and RSS;
- a reproducible smoke procedure at [Phase 4 Candle Llama Smoke Procedure](../execution/phase4-candle-smoke.md).

Ordinary CI remains download-free: its Candle integration tests use deterministic tiny local Safetensors fixtures.

## Integration depth

| Capability | E0 inference runtime | E1 application runtime | Slint UI |
|---|---:|---:|---:|
| Model load, generation-safe handle, drain, cancellation, unload | Yes | Yes for Candle lifecycle | Yes for Candle lifecycle |
| Backend prefill and decode primitives | Yes | Not exposed as generation | No |
| Backend-independent generation scheduler | Fake backend plus Candle CPU integration source | Not exposed | No |
| Sampling algorithm | Integrated inside E0 | Not exposed | No |
| Bounded streamed token output | Pull-oriented token/state batches | No | No |
| Direct-completion real-model loop | Opt-in E0 smoke source | No | No |
| Tokenization and decoded text streaming | Separate foundations only | Not integrated | No |
| General chat templates/history | No | No | No |

## Validation status for this delivered patch

The canonical Phase 4 commands were **not executed in the artifact-editing environment** because it contains no Rust toolchain (`cargo`, `rustc`, or `rustfmt`) and external network access was unavailable. The pinned external-model smoke was also not executed here. Therefore this tree does not yet claim warning-free compilation, passing tests, successful real-model execution, or recorded local measurements.

Run the following from the repository root with the pinned toolchain in `rust-toolchain.toml`:

```text
cargo fmt --all --check
cargo run --locked --bin llm-app -- verify
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo deny --workspace --locked check advisories bans licenses sources
lychee --offline --no-progress "**/*.md"
git diff --check
```

Then run [the pinned Phase 4 smoke procedure](../execution/phase4-candle-smoke.md). The exact completion rule and implementation matrix are in the [Phase 4 implementation report](../execution/PHASE4_IMPLEMENTATION_REPORT.md).

## Known limitations

- The E0 request starts from caller-supplied token IDs and emits token IDs. Tokenizer ownership, incremental text decoding, E1 generation commands, and frontend pulls remain Phase 5 work.
- The selected smoke fixture is a tiny random test model. It proves architecture and lifecycle integration, not output quality.
- The deterministic cleanup policy uses a total-attempt limit, not wall-clock retry backoff. Exhausted resources remain quarantined and accounted.
- Strict allocation-free Candle execution is not claimed because upstream Candle allocates intermediate and KV-cache tensors.
- GPU execution, remote/browser transport, general chat, GGUF UI selection, and multi-model E1 state are unsupported.

## Historical implementation record

The recovered [implementation plan](implementation-plan.md) is retained as historical context and is not authoritative. The execution plan supersedes its old phase sequence and proposed repository shape.
