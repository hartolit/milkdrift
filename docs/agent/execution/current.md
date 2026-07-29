# Current execution context

**Reviewed baseline:** `main` commit `f8b3396cc23085696123b95c9dcb4b17c3d9c214` plus this documentation-only Phase 7 preparation closure
**Current target:** Phase 7 — real chat and context planning
**Gate state:** Source and architecture readiness review is complete; the canonical locked gate must be recorded on the exact final Phase 7-preparation tree before implementation evidence is called current
**Canonical plan:** [execution-plan.md](execution-plan.md)
**Current product truth:** [project implementation status](../../project/implementation-status.md)

This document is the dense working set for the active phase. It is derived context, not a replacement for the plan, ADRs, status page, or component guides.

## Immediate objective

Turn the proven direct-completion product path into honest conversation behavior and connect deterministic context planning to real generation input:

```text
typed conversation messages
→ explicit model/template compatibility
→ deterministic context selection with reserved output capacity
→ render selected messages in order
→ tokenize and verify actual capacity
→ submit through E1
→ stream assistant text as conversation state
```

Phase 7 changes conversation semantics. It must preserve the Phase 6 lifecycle controls, bounded frame-aligned presentation updates, E0 scheduling ownership, E1 façade, cancellation behavior, unload policy, and explicit shutdown.

## Architecture entering Phase 7

- Physical roots are `domain`, `platform`, `adapters`, `runtime`, and `apps`; runtime and platform roles are explicit rather than granted by directory placement.
- `inference-runtime` is E0, `corrective-workflow` is an independently stateful capability runtime, and `application-runtime` is E1. Runtime-to-infrastructure and runtime-to-runtime production edges require exact reviewed composition entries.
- E1 remains the current concrete Candle/Hugging Face/redb composition root until a second backend or deployment proves the correct local composition seam. Phase 7 must not pre-empt that evidence with a generic service graph.
- Future peer, rented-GPU, and hosted-provider execution belongs behind a coarse request/stream boundary above E0. Conversation state must survive a target change without containing provider SDK, peer transport, or local model-resource identity.

## What Phase 7 inherits

### First usable Slint product

`desktop-slint` now exposes:

- repository and revision controls;
- resolve, CPU load, and drain-unload actions;
- direct-completion prompt and generated-output views;
- generate, cancel, and clear-output actions;
- prompt/generated usage, status, terminal reason, and a Candle/CPU label;
- one 16 millisecond cadence that drains at most 64 events, pulls one bounded decoded-output batch, appends only the new frame fragment while preserving selection/viewport state, and synchronizes from `ApplicationState`;
- explicit shutdown after the event loop and after post-runtime window-construction failure.

This is the presentation baseline to evolve, not a second inference path. See [desktop runtime](../../project/desktop-runtime.md).

### E1 direct-completion boundary

`application-runtime` owns prompt encoding, generation settings, request-local streaming decode, bounded text output, state/events, cancellation, unload behavior, and shutdown. Phase 7 adds reusable conversation semantics and coordinates context planning/rendering through their own boundaries; it must not absorb the planner algorithm, provider transports, or workflow engine. See [application runtime](../../project/application-runtime.md).

### Existing context planner

The portable `context-planner` crate already owns deterministic capacity selection contracts. Phase 7 should integrate it through typed E1 conversation input rather than moving UI or tokenizer/vendor types into the domain layer. `ContextEntry` is a derived planner input, not the canonical representation of conversation history.

## Phase 7 work packages

1. Define frontend-neutral conversation records with stable identity/order, role, UTF-8 content, provenance, retention/pinning policy, measured or conservative token estimates, and response-attempt terminal state where applicable. User messages become committed history immediately; assistant streaming is an active response attempt. A cancelled or failed attempt preserves its partial text and terminal provenance for inspection but is not silently promoted into ordinary future context.
2. Define explicit prompt-rendering compatibility for a verified model family/profile or tested resolved chat template. Rendering produces the local prompt plus the model/profile termination semantics required for one assistant turn. Unknown compatibility must fail; never guess a template or silently invent EOS/stop behavior.
3. Derive `ContextEntry` values from conversation state for each request, reserve output capacity, select deterministically, render in conversation order, tokenize the final prompt, and verify actual capacity. If estimates were insufficient, each correction pass must strictly reduce the selected non-pinned set; unchanged retries are forbidden. The number of render/tokenize attempts is bounded by the initial selected droppable-entry count plus one. If no droppable entry remains, fail explicitly rather than dropping pinned content or looping.
4. Add E1 operations for user-message submission, regeneration, conversation clearing, context diagnostics, and cancellation. Regeneration creates a new assistant response attempt for the same user turn and supersedes the previous attempt in the active-context view without deleting the raw prior record. Phase 7 does not need a general branch tree. Clearing while a response is active is rejected; callers cancel and observe terminal state before clearing. Keep persistence out until in-memory semantics stabilize.
5. Replace the direct prompt/output presentation with conversation records while retaining lifecycle controls and batched assistant updates. The UI must represent successful, cancelled, and failed assistant attempts without converting presentation state into conversation ownership.

## Architectural invariants

1. Conversation semantics and rendering compatibility coordination are frontend-neutral E1 behavior.
2. Widget types, local model handles, provider DTOs, transport types, and backend-specific templates do not enter conversation-domain types.
3. E0 remains the local token-step and native model-resource owner; it is not the abstraction for hosted or peer execution.
4. Pinned content either fits or returns `PinnedBudgetExceeded`; it is never silently dropped.
5. Actual tokenized input plus reserved output cannot exceed model capacity.
6. Unknown model/template compatibility fails explicitly.
7. Assistant streaming remains bounded and pull-oriented; Slint does not receive one callback per token.
8. Direct completion remains an honest mode until a supported chat renderer is proven.
9. Do not extract a new rendering crate before independent renderers or consumers justify it.
10. Raw conversation provenance and the active context view are distinct: regeneration or failed attempts may change what is selected next without erasing what happened.
11. Planner records are request-local derived views over conversation state; `context-planner` does not become the conversation store.
12. Context correction after exact tokenization is finite and monotonic; every retry removes at least one droppable entry.
13. Chat termination policy is part of model/rendering compatibility and is tested alongside prompt formatting.

## Explicit non-goals

- no GGUF product selection;
- no GPU execution;
- no browser/remote transport;
- no hosted-provider or peer execution implementation;
- no multi-model E1 residency;
- no guessed universal Llama prompt template;
- no conversation persistence before in-memory semantics stabilize;
- no arbitrary conversation branch tree beyond the regeneration/supersession semantics required by this phase;
- no general workflow, long-term-memory, tool/permission, or peer-routing framework pulled into Phase 7;
- no broad crate/folder reorganization.

## Acceptance criteria

- a known supported instruct model receives the verified prompt format;
- the same compatibility profile supplies tested assistant-turn termination behavior rather than relying on accidental default stops;
- context planning changes real generation input;
- actual token count cannot exceed capacity;
- pinned content is never silently discarded;
- exact-token correction is bounded and cannot retry the same selection indefinitely;
- E1 owns conversation history and assistant streaming semantics without binding message state to the current local execution implementation;
- cancelled/failed partial assistant output remains inspectable but is not silently treated as a normal successful history message;
- regeneration preserves the previous attempt for provenance while the active-context view uses the replacement;
- `ContextEntry` remains a derived planner representation rather than stored conversation identity;
- unknown template compatibility fails explicitly.

## Validation and documentation

Run the canonical gate on the exact final tree:

```sh
git rev-parse HEAD
cargo run --locked --bin llm-app -- verify
```

Record the exact commit together with the complete gate result. Update the canonical application runtime, desktop runtime, implementation status, and execution history documents when behavior changes. The full Phase 7 specification is in [execution-plan.md](execution-plan.md#phase-7--add-real-chat-and-context-planning).
