# Current implementation status

**Status date:** 2026-07-27
**Reviewed source baseline:** `phase-6` commit `68438648c09bc008e628508ebf269456c6299096` plus the source-level review closure recorded below
**Execution position:** Phase 6 source closure is present; Phase 7 is the current implementation target
**Canonical plan:** [LLM App Execution Plan](../agent/execution/execution-plan.md)
**Current working context:** [Phase 7 execution context](../agent/execution/current.md)

This is the canonical product-level status page. Component behavior lives in the corresponding project guide; historical phase evidence lives in [execution history](../agent/execution/history.md).

## Supported devices and backends

| Backend | Device | Adapter/E0 boundary | `application-runtime` (E1) | Slint UI |
|---|---|---:|---:|---:|
| Candle 0.11 Llama/Safetensors | CPU | Generation vertical slice | Direct-completion façade implemented | Direct completion implemented |
| GGUF via llama.cpp | CPU | Lifecycle and backend primitives | No generation composition | No |
| Candle or GGUF | CUDA/Metal/other GPU | No supported product path | No | No |

The product remains CPU-only. Candle Llama is driven through the E0 generation scheduler and exposed through the frontend-neutral E1 generation boundary. Slint now wires the first direct-completion loop through E1. GGUF is not yet selectable through E1 or the UI.

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
- Slint generation controls with frame-aligned bounded text pulling, fragment-only widget appends that preserve selection/viewport state, usage, terminal-state, cancellation-pending, and clear-output presentation.

## Integration depth

| Capability | E0 inference runtime | E1 application runtime | Slint UI |
|---|---:|---:|---:|
| Model load, generation-safe handle, drain, cancellation, unload | Yes | Yes | Lifecycle controls |
| Backend-independent generation scheduler | Yes | Submitted as one complete request | No direct access; bounded consumer only |
| Sampling algorithm | Integrated inside E0 | Stable settings translated at admission | Uses E1 defaults |
| Bounded streamed token output | Pull-oriented token/state batches | Consumed internally | One decoded-text pull per frame |
| Prompt tokenization | Token IDs only | Direct-completion prompt encoded once | Prompt control wired through E1 |
| Stateful decoded text streaming | No text ownership | Bounded request-local UTF-8 pulls | Batched presentation output |
| Generation start/cancel state | Runtime command/state | Public E1 API/state/events | Generate/cancel/status/terminal controls |
| General chat templates/history | No | No | No |

## Validation state

The canonical locked repository gate passed locally on 2026-07-27 on `68438648c09bc008e628508ebf269456c6299096` plus the documented Phase 6 review-closure changes. The run covered architecture/dependency validation, formatting, workspace checks, tests/doctests, strict Clippy, rustdoc, and benchmark compilation. Nine focused `desktop-slint` presenter tests and strict all-target Clippy also passed. No independent GitHub Actions run or committed review-closure revision is attached to this local evidence.

The graphical external-model acceptance scenario was not manually exercised in this environment. Download-free E1/Candle integration tests cover the underlying generation, cancellation, unload, backpressure, and shutdown loop; presenter tests cover the new UI mapping. Historical Phase 3–5 evidence remains in [execution history](../agent/execution/history.md).

## Known limitations

- Direct completion is implemented; general chat templates, message history, context rendering, and conversation persistence are not.
- Slint uses the stable E1 default generation settings; a configurable settings panel is not yet exposed.
- The E1 product path is deliberately single-model even though lower E0 contracts can represent more general residency.
- The Candle external smoke fixture is a tiny random test model. It proves execution/lifecycle integration, not language quality.
- Strict allocation-free Candle or Hugging Face tokenization/decoding execution is not claimed because upstream libraries allocate internally.
- GPU execution, remote/browser transport, and GGUF UI selection remain unsupported.

## Historical context

The [recovered implementation plan](implementation-plan.md) is retained as historical source material and is not authoritative. The active roadmap is the [execution plan](../agent/execution/execution-plan.md), the current working set is [current execution context](../agent/execution/current.md), and closed-phase evidence is consolidated in [execution history](../agent/execution/history.md).
