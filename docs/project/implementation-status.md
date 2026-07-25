# Current implementation status

**Status date:** 2026-07-26
**Source baseline:** `main` commit `bda24063ad04dd21527cbfa88e2c2e4ed8313d22`
**Execution position:** Phase 5 source closure is present on `main`; a successful canonical gate is not recorded for this exact commit, so Phase 6 remains gated on fresh validation
**Canonical plan:** [LLM App Execution Plan](../execution/execution-plan.md)

This is the canonical product-level status page. Component behavior lives in the corresponding project guide; historical phase evidence lives in [execution history](../execution/history.md).

## Supported devices and backends

| Backend | Device | Adapter/E0 boundary | `application-runtime` (E1) | Slint UI |
|---|---|---:|---:|---:|
| Candle 0.11 Llama/Safetensors | CPU | Generation vertical slice | Direct-completion façade implemented | Lifecycle only |
| GGUF via llama.cpp | CPU | Lifecycle and backend primitives | No generation composition | No |
| Candle or GGUF | CUDA/Metal/other GPU | No supported product path | No | No |

The product remains CPU-only. Candle Llama is driven through the E0 generation scheduler and exposed through the frontend-neutral E1 generation boundary. Slint generation controls are not yet wired. GGUF is not yet selectable through E1 or the UI.

## Current E1 generation boundary

The source tree contains:

- stable E1 `GenerationSettings` for maximum new tokens, sampling controls, repetition policy, seed policy, EOS tokens, and textual stop suffixes;
- direct-completion prompt encoding through the resolved `HfTokenizer`, with special-token behavior explicit and no chat-template claim;
- request-local owned Hugging Face streaming decode state without full-history re-decode;
- application-owned generation state for start, running, cancellation, terminal cleanup/exhaustion, usage, and last terminal result;
- bounded UTF-8/state accumulation with E1 types that hide host-runtime implementation details from frontends;
- bounded translation of E0 token/state pulls into decoded text/state pulls without frontend-driven per-token commands;
- explicit single-model E1 configuration;
- application-owned `ModelUnloadBehavior::{RejectIfBusy, CancelActive, Drain}` rather than exposing E0 unload policy types;
- download-free E1/Candle integration coverage for generation, backpressure, cancellation, unload behavior, worker disconnection, and shutdown;
- Slint compatibility for low-frequency generation events without generation controls.

## Integration depth

| Capability | E0 inference runtime | E1 application runtime | Slint UI |
|---|---:|---:|---:|
| Model load, generation-safe handle, drain, cancellation, unload | Yes | Yes | Yes for lifecycle |
| Backend-independent generation scheduler | Yes | Submitted as one complete request | No direct access |
| Sampling algorithm | Integrated inside E0 | Stable settings translated at admission | No |
| Bounded streamed token output | Pull-oriented token/state batches | Consumed internally | No |
| Prompt tokenization | Token IDs only | Direct-completion prompt encoded once | No generation control yet |
| Stateful decoded text streaming | No text ownership | Bounded request-local UTF-8 pulls | Wiring pending |
| Generation start/cancel state | Runtime command/state | Public E1 API/state/events | Controls pending |
| General chat templates/history | No | No | No |

## Validation state

The current `main` baseline is `bda24063ad04dd21527cbfa88e2c2e4ed8313d22`. No GitHub Actions run is attached to this commit in the connected repository, and the older Phase 5 closure evidence explicitly required validation after its closure patch. Therefore this page does not claim that the current baseline has passed the final locked gate.

Run the canonical procedure in [validation](validation.md) on the exact current tree and record the resulting commit/CI provenance before advancing the execution state. Older Phase 3–5 evidence remains available in [execution history](../execution/history.md) but does not substitute for current-tree validation.

## Known limitations

- Direct completion is implemented; general chat templates, message history, context rendering, and conversation persistence are not.
- Slint exposes lifecycle controls only; prompt input, generated output, generate/cancel controls, usage display, and frame-aligned text application remain unimplemented in the frontend.
- The E1 product path is deliberately single-model even though lower E0 contracts can represent more general residency.
- The Candle external smoke fixture is a tiny random test model. It proves execution/lifecycle integration, not language quality.
- Strict allocation-free Candle or Hugging Face tokenization/decoding execution is not claimed because upstream libraries allocate internally.
- GPU execution, remote/browser transport, and GGUF UI selection remain unsupported.

## Historical context

The [recovered implementation plan](implementation-plan.md) is retained as historical source material and is not authoritative. The active roadmap is the [execution plan](../execution/execution-plan.md); closed-phase evidence is consolidated in [execution history](../execution/history.md).
