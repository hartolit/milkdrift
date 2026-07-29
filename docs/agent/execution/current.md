# Current execution context

**Reviewed baseline:** committed Phase 7 closure through `3b4541f50fcf614bc65938d448b383f507d27fcd` plus the final semantic-closure working tree
**Current target:** Phase 8 — GGUF parity and native composition evidence
**Gate state:** earlier Phase 7 validation predates the final turn-atomic/provenance closure; run the canonical locked gate on the exact resulting tree before Phase 8 work begins
**Canonical plan:** [execution-plan.md](execution-plan.md)
**Current product truth:** [project implementation status](../../project/implementation-status.md)

This document is the dense handoff for the next phase. Phase 7 behavior is canonical in the application/desktop guides and its closure evidence belongs in execution history.

## Immediate objective

Make GGUF a correct second local E0-backed product path without duplicating E1 conversation semantics or pretending that an arbitrary Hugging Face tokenizer is compatible:

```text
verified GGUF tokenizer/model metadata
→ closed local backend/source selection
→ shared E0 and E1 generation behavior
→ evidence-based native composition decision
→ backend selection in Slint only after parity
```

## Architecture entering Phase 8

- `application-runtime` remains E1 and now owns in-memory raw conversation records, response-attempt provenance, regeneration/supersession, context diagnostics, prompt compatibility coordination, bounded decoded output, and model lifecycle policy.
- `context-planner` owns deterministic estimate selection and the exact-correction candidate order. E1 derives request-local planning units and `ContextEntry` views, keeps completed historical user/assistant turns atomic, pins the current user and stored pinned content, expands selected units back into records, renders, tokenizes exactly, and admits only a capacity-safe prompt.
- Chat compatibility is closed to immutable TinyLlama Chat v1 artifact commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`; repository identity, that resolved commit, tokenizer `</s>` → ID 2, role markers, and EOS policy form one compatibility claim. Unknown or unreviewed provenance fails; direct completion remains a separate honest E1 mode.
- Slint is a thin chat presenter. It retains the 16 ms cadence, drains at most 64 events, performs one bounded output pull, and appends one frame-batched assistant fragment while conversation ownership remains in E1.
- E0 still exclusively owns native model resources, token scheduling, cancellation boundaries, cleanup, and unload. Hosted and peer execution remain future coarse targets above E0.

## Phase 7 invariants to preserve

1. Raw history and the active-context view remain distinct; failed, cancelled, and superseded attempts are inspectable but excluded from ordinary future context.
2. Regeneration preserves prior attempts and creates one replacement for the same user turn; no arbitrary branch tree is implied.
3. Completed historical user/assistant turns are atomic context-planning units; neither side is retained without the other.
4. Actual rendered input plus reserved output never exceeds model context, and rendered input never exceeds prefill capacity.
5. Exact-token correction strictly removes one planner-selected non-pinned unit per retry and is bounded by the initial droppable-unit count plus one.
6. Pinned content is never silently dropped.
7. Conversation records contain no Candle source, model handle, provider DTO, peer connection, or transport identity.
8. Built-in prompt/termination compatibility requires the reviewed immutable artifact commit and tokenizer evidence; repository naming alone is insufficient.
9. Direct completion remains available for models without the one supported chat profile.
10. Assistant streaming stays bounded and pull-oriented; frontends do not drive per-token work.
11. Conversation persistence remains out of scope until in-memory semantics stabilize.
12. Response-attempt terminal semantics are independent from E0 cleanup/release; cleanup exhaustion cannot leave a terminal response marked as streaming.
13. Regeneration is allowed only for the newest responded turn; a later unanswered user record blocks regeneration of older turns.
14. Slint chat controls and transcript snapshots derive from E1 chat compatibility and canonical history rather than generic generation readiness or stale presentation state.

## Phase 8 work packages

1. Implement a GGUF-compatible tokenizer path from llama.cpp/GGUF metadata or verified immutable external metadata. Prompt encoding and stateful streaming decode must satisfy the existing portable tokenization contracts.
2. Add a closed local native backend/source selection. Do not genericize the public E1 façade, and do not model hosted/peer targets as local backend variants.
3. Use real two-backend pressure to decide whether concrete local composition should move beneath E1. Record the result in an ADR; do not create `application-api` without a real transported consumer.
4. Run one shared backend generation suite covering load, start, prefill, greedy/seeded decode where defined, EOS/token limit, cancellation, backpressure, cleanup, and unload.
5. Expose backend/source selection in Slint only after parity is proven; the frontend must not construct backend source types.

## Explicit non-goals

- no GPU path;
- no hosted-provider or peer implementation;
- no browser transport or speculative `application-api` crate;
- no arbitrary HF-tokenizer/GGUF pairing by vocabulary size;
- no duplicated application/conversation state machine;
- no broad architecture split before the second backend supplies evidence.

## Acceptance criteria

- Candle and GGUF complete the same E1 generation scenario;
- tokenization is model-compatible for both paths;
- the UI contains no backend construction logic;
- switching backend does not duplicate lifecycle, conversation, context, or streaming semantics;
- the composition decision is captured in an ADR.

## Validation rule

Run the canonical gate on the exact final tree:

```sh
git rev-parse HEAD
cargo run --locked --bin llm-app -- verify
```

Record whether the tree is committed or a working tree. Historical validation on the Phase 7 tree is not evidence for later Phase 8 edits.
