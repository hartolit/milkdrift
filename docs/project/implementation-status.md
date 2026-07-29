# Current implementation status

**Status date:** 2026-07-30
**Reviewed source baseline:** `6cc52de` plus the current composability-hardening working tree
**Execution position:** Phase 7 is revalidated on the current tree; Phase 8 GGUF parity remains the next product integration and has not started
**Canonical plan:** [LLM App Execution Plan](../agent/execution/execution-plan.md)
**Current working context:** [Phase 8 execution context](../agent/execution/current.md)

This is the canonical product-level status page. Component behavior lives in the corresponding project guide; historical phase evidence lives in [execution history](../agent/execution/history.md).

## Supported devices and backends

| Backend | Device | Adapter/E0 boundary | `application-runtime` (E1) | Slint UI |
|---|---|---:|---:|---:|
| Candle 0.11 Llama/Safetensors | CPU | Generation vertical slice | Direct completion plus verified TinyLlama Chat v1 | Conversation UI for verified TinyLlama Chat v1 |
| GGUF via llama.cpp | CPU | Lifecycle and backend primitives | No generation composition | No |
| Candle or GGUF | CUDA/Metal/other GPU | No supported product path | No | No |

The product remains CPU-only. Candle Llama is driven through the E0 generation scheduler and exposed through E1. Direct completion remains available for compatible Candle Llama models. Chat is deliberately narrower: Slint and E1 support only the verified `TinyLlama/TinyLlama-1.1B-Chat-v1.0` prompt/termination profile. GGUF is not yet selectable through E1 or the UI.

## Current runtime boundaries

The corrective workflow is an independent `corrective-workflow` capability runtime
rather than an E1 subsystem. Its model/validator ports now write into engine-owned
bounded output sinks; generated artifacts carry workflow ownership, failed runs roll
back artifacts/events, and successful artifacts have explicit release. E1 remains the
frontend-neutral application coordinator; E0 remains the owner of local model
resources and token-level scheduling. Hosted providers and peer nodes are not
implemented and are not modeled as E0 backends.

E0 now verifies complete loaded descriptors, numeric capability consistency,
sequence plans against advertised limits, operation support, exact ready-state
position/identity/capacity transitions, and full-vocabulary logits before sampling.
This strengthens Candle/GGUF substitution at the existing local backend boundary;
it does not make GGUF an E1 product path.

The workspace now uses `domain`, `platform`, `adapters`, `runtime`, and `apps` as its physical roots. Runtime and platform roles fail closed in the architecture validator, and runtime production dependencies on platform/adapters or another runtime require an exact reviewed composition edge.

## Current E1 generation and conversation boundary

The source tree contains:

- stable E1 `GenerationSettings` for maximum new tokens, sampling controls, repetition policy, seed policy, EOS tokens, and textual stop suffixes;
- direct-completion prompt encoding through the resolved `HfTokenizer`, with special-token behavior explicit;
- exact `TinyLlama/TinyLlama-1.1B-Chat-v1.0` textual role rendering plus EOS token 2 termination compatibility, enabled only for reviewed immutable commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6` when tokenizer `</s>` resolves to ID 2;
- stable frontend-neutral conversation records with raw provenance, retention, measured/generated/conservative token estimates, response-attempt terminal state, and supersession;
- response-attempt terminal state is committed at generation terminal rather than delayed until native cleanup release, while the E1 generation lifecycle remains active through cleanup;
- deterministic derived turn-atomic `ContextEntry` planning, reserved output capacity, ordered raw-record rendering, exact tokenization, strictly shrinking bounded correction, pinned overflow, and selected/dropped raw-record diagnostics;
- submit, regenerate, clear, and cancel operations with active mutation rejection, unanswered-turn regeneration protection, and in-memory-only history;
- request-local owned Hugging Face streaming decode state without full-history re-decode, with fragments appended once to E1 assistant state;
- application-owned generation state for start, running, cancellation, terminal cleanup/exhaustion, usage, and last terminal result;
- bounded UTF-8/state accumulation with E1 types that hide host-runtime implementation details from frontends;
- bounded translation of E0 token/state pulls into decoded text/state pulls without frontend-driven per-token commands;
- explicit single-model E1 configuration;
- application-owned `ModelUnloadBehavior::{RejectIfBusy, CancelActive, Drain}` rather than exposing E0 unload policy types;
- download-free E1/Candle integration coverage for generation, backpressure, cancellation, unload behavior, worker disconnection, and shutdown;
- Slint conversation controls with frame-aligned bounded text pulling, fragment-only assistant appends, canonical transcript resynchronization after commit-then-admission failure and terminal lifecycle changes, chat-compatible send/regenerate admission, usage, and explicit successful/cancelled/failed presentation.

## Integration depth

| Capability | E0 inference runtime | E1 application runtime | Slint UI |
|---|---:|---:|---:|
| Model load, generation-safe handle, drain, cancellation, unload | Yes | Yes | Lifecycle controls |
| Backend-independent generation scheduler | Yes | Submitted as one complete request | No direct access; bounded consumer only |
| Sampling algorithm | Integrated inside E0 | Stable settings translated at admission | Uses E1 defaults |
| Bounded streamed token output | Pull-oriented token/state batches | Consumed internally | One decoded-text pull per frame |
| Prompt tokenization | Token IDs only | Direct completion or planned/rendered TinyLlama chat prompt | Message composer through E1 |
| Stateful decoded text streaming | No text ownership | Bounded request-local UTF-8 pulls plus assistant attempt state | Batched transcript fragments |
| Generation start/cancel state | Runtime command/state | Public E1 API/state/events | Send/regenerate/cancel/status controls |
| Chat templates/history | No | One verified TinyLlama Chat v1 profile; in-memory raw history | Conversation transcript and clear |

## Validation state

On 2026-07-30, the complete canonical locked gate passed on the uncommitted
composability-hardening working tree based on `6cc52de`, including a rerun after the
final status and current-context updates:

```text
cargo run --locked --bin llm-app -- verify
```

It covered architecture/dependency validation, formatting, workspace checks, the
full test/doctest suite, strict Clippy, rustdoc, and benchmark compilation. Focused
fault-injection suites additionally cover malformed descriptors, unsupported
capabilities, over-limit sequence plans, short logits, invalid positions/usage, and
mutable sequence state/identity/capacity. Corrective workflow tests cover bounded
chunk/diagnostic output, non-retryable overflow, late-failure rollback, ownership,
and explicit release. Hub/application tests prove access-token debug redaction.

The graphical external-model acceptance scenario was not manually exercised in
this environment. Download-free E1/Candle integration tests cover rendered prompt
admission, exact usage, planning/correction, regeneration, pinned overflow,
cancellation/unload/backpressure, and shutdown; presenter tests cover the chat UI
mapping. Historical earlier-phase evidence remains in [execution history](../agent/execution/history.md).

## Known limitations

- Chat compatibility is intentionally limited to reviewed TinyLlama Chat v1 commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`; other repositories or commits use no guessed fallback.
- Conversation history is in memory only; persistence and arbitrary branch trees are not implemented.
- Slint uses the stable E1 default generation settings; a configurable settings panel is not yet exposed.
- The E1 product path is deliberately single-model even though lower E0 contracts can represent more general residency.
- The Candle external smoke fixture is a tiny random test model. It proves execution/lifecycle integration, not language quality.
- Strict allocation-free Candle or Hugging Face tokenization/decoding execution is not claimed because upstream libraries allocate internally.
- GPU execution, hosted-provider execution, peer execution, remote/browser transport, and GGUF UI selection remain unsupported.

## Historical context

The [recovered implementation plan](implementation-plan.md) is retained as historical source material and is not authoritative. The active roadmap is the [execution plan](../agent/execution/execution-plan.md), the current working set is [current execution context](../agent/execution/current.md), and closed-phase evidence is consolidated in [execution history](../agent/execution/history.md).
