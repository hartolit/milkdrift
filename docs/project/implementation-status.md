# Current Implementation Status

**Status date:** 2026-07-25

**Source baseline:** `main` commit `f6ac1806c33d4a1d84dfabb66c14f3475af5872a` plus the Phase 5 closure patch

**Execution position:** Phase 5 closure candidate; rerun the locked repository gate on the resulting tree before Phase 6

**Canonical plan:** [LLM App Execution Plan](../execution/execution-plan.md)

This document is the canonical statement of what the delivered source tree claims. It deliberately separates implemented source from validation evidence.

## Supported devices and backends

| Backend | Device | Adapter/E0 boundary | `application-runtime` (E1) | Slint UI |
|---|---|---:|---:|---:|
| Candle 0.11 Llama/Safetensors | CPU | Phase 4 generation vertical slice | Phase 5 direct-completion façade implemented | Lifecycle only |
| GGUF via llama.cpp | CPU | Lifecycle and backend primitives | No generation composition | No |
| Candle or GGUF | CUDA/Metal/other GPU | No supported product path | No | No |

The repository remains CPU-only. Candle Llama is driven through the E0 generation scheduler. Phase 5 adds the frontend-neutral E1 generation boundary; Slint generation controls remain Phase 6.

## Phase 5 source implementation

The Phase 5 tree contains:

- stable E1 `GenerationSettings` with maximum new tokens, temperature, top-k, top-p, min-p, repetition penalty/window, seed policy, explicit EOS tokens, and textual stop suffixes;
- direct-completion prompt encoding through the resolved `HfTokenizer`, with boundary-special-token behavior explicit and no chat-template claim;
- owned request-local Hugging Face streaming decode state that does not borrow `ApplicationRuntime`'s tokenizer and does not re-decode the complete generated history;
- application-owned request state for starting, running, cancelling, terminal cleanup, cleanup exhaustion, usage, and the last terminal result;
- a bounded generic UTF-8/state accumulator in `host-runtime` and an E1 wrapper that hides host-runtime implementation types from frontends;
- translation of E0 token/state pulls into decoded text/state pulls without frontend-driven per-token commands;
- explicit single-model E1 configuration: E1 configures E0 for one resident model and no longer exposes a misleading `maximum_models` setting;
- application-owned `ModelUnloadBehavior` for reject, safe-boundary cancel, or bounded-drain unload without leaking E0 policy types;
- tokenizer, text-accumulator, application-state, and download-free E1/Candle integration tests covering generation, backpressure, cancellation, unload policies, worker disconnection, and shutdown;
- Slint presentation compatibility for low-frequency generation events without adding Phase 6 generation controls.

## Integration depth

| Capability | E0 inference runtime | E1 application runtime | Slint UI |
|---|---:|---:|---:|
| Model load, generation-safe handle, drain, cancellation, unload | Yes | Yes | Yes for lifecycle |
| Backend-independent generation scheduler | Yes | Submitted as one complete request | No direct access |
| Sampling algorithm | Integrated inside E0 | Stable settings translated at admission | No |
| Bounded streamed token output | Pull-oriented token/state batches | Consumed internally | No |
| Prompt tokenization | Token IDs only | Direct-completion prompt encoded once | No generation control yet |
| Stateful decoded text streaming | No text ownership | Bounded request-local UTF-8 pulls | Phase 6 wiring pending |
| Generation start/cancel state | Runtime command/state | Public E1 API/state/events | Phase 6 controls pending |
| General chat templates/history | No | No | No |

## Validation evidence and remaining gate

The committed Phase 5 implementation reached `f6ac1806c33d4a1d84dfabb66c14f3475af5872a`. This closure patch changes E1 unload behavior, application-level tests, presenter structure, and documentation, so validation of the baseline commit is not evidence for the resulting tree.

Run the repository's canonical locked gate after applying the closure patch:

```text
cargo run --locked --bin llm-app -- verify
```

For focused diagnosis before or after the canonical gate:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo bench --workspace --no-run --locked
git diff --check
```

Phase 5 is complete only after the canonical locked verification passes on the exact resulting commit/tree. Record a successful CI run for that same commit when available; any compile, Clippy, test, rustdoc, or runtime failure remains a Phase 5 defect.

## Known limitations

- Phase 5 is direct completion only. General chat templates, message history, context rendering, and conversation persistence remain later phases.
- Slint still exposes lifecycle controls only; Phase 6 adds prompt input, generated output, generate/cancel controls, usage display, and frame-aligned text application.
- The E1 product path is deliberately single-model even though lower E0 contracts can represent more general residency.
- The selected Candle smoke fixture is a tiny random test model. It proves architecture and lifecycle integration, not output quality.
- Strict allocation-free Candle or Hugging Face tokenization/decoding execution is not claimed because upstream libraries allocate internally.
- GPU execution, remote/browser transport, and GGUF UI selection remain unsupported.

## Historical implementation record

The recovered [implementation plan](implementation-plan.md) is retained as historical context and is not authoritative. The execution plan supersedes its old phase sequence and proposed repository shape.
