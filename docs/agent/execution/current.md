# Current execution context

**Reviewed baseline:** `phase-6` commit `68438648c09bc008e628508ebf269456c6299096` plus source-level review closure
**Current target:** Phase 7 — real chat and context planning
**Gate state:** The canonical locked gate passed locally on the Phase 6 closure tree; no independent CI run or committed Phase 6 revision is recorded
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

`application-runtime` owns prompt encoding, generation settings, request-local streaming decode, bounded text output, state/events, cancellation, unload behavior, and shutdown. Chat/history/template/context behavior must be added here so frontends do not duplicate it. See [application runtime](../../project/application-runtime.md).

### Existing context planner

The portable `context-planner` crate already owns deterministic capacity selection contracts. Phase 7 should integrate it through typed E1 conversation input rather than moving UI or tokenizer/vendor types into the feature layer.

## Phase 7 work packages

1. Define frontend-neutral conversation messages with role, identity/order, text, optional provenance, retention/pinning policy, and measured or conservative token estimates.
2. Define explicit prompt-rendering compatibility for a verified model family/profile or tested resolved chat template. Unknown compatibility must fail; never guess a template.
3. Reserve output capacity, select context deterministically, render in conversation order, tokenize the final prompt, verify actual model capacity, and retry selection or fail clearly when estimates were insufficient.
4. Add E1 operations for user-message submission, allowed regeneration, conversation clearing, context diagnostics, and cancellation. Keep persistence out until in-memory semantics stabilize.
5. Replace the direct prompt/output presentation with message records while retaining lifecycle controls and batched assistant updates.

## Architectural invariants

1. Conversation state and rendering compatibility are frontend-neutral E1 behavior.
2. Widget types and backend-specific templates do not enter conversation-domain types.
3. E0 remains the token-step and model-resource owner.
4. Pinned content either fits or returns `PinnedBudgetExceeded`; it is never silently dropped.
5. Actual tokenized input plus reserved output cannot exceed model capacity.
6. Unknown model/template compatibility fails explicitly.
7. Assistant streaming remains bounded and pull-oriented; Slint does not receive one callback per token.
8. Direct completion remains an honest mode until a supported chat renderer is proven.
9. Do not extract a new rendering crate before independent renderers or consumers justify it.

## Explicit non-goals

- no GGUF product selection;
- no GPU execution;
- no browser/remote transport;
- no multi-model E1 residency;
- no guessed universal Llama prompt template;
- no conversation persistence before in-memory semantics stabilize;
- no broad crate/folder reorganization.

## Acceptance criteria

- a known supported instruct model receives the verified prompt format;
- context planning changes real generation input;
- actual token count cannot exceed capacity;
- pinned content is never silently discarded;
- E1 owns conversation history and assistant streaming;
- unknown template compatibility fails explicitly.

## Validation and documentation

Run the canonical gate on the exact final tree:

```sh
cargo run --locked --bin llm-app -- verify
```

Update the canonical application runtime, desktop runtime, implementation status, and execution history documents when behavior changes. The full Phase 7 specification is in [execution-plan.md](execution-plan.md#phase-7--add-real-chat-and-context-planning).
