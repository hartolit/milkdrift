# ADR-0004: Deliver direct completion before general chat

- **Status:** Accepted
- **Date:** 2026-07-22

## Context

The repository has tokenization, context planning, sampling, model primitives, and lifecycle infrastructure but no integrated generation loop. General chat support additionally requires model-specific prompt rendering, conversation roles/history, context budgeting, and stop compatibility. Implementing those simultaneously would obscure whether failures originate in the generation kernel or chat semantics.

## Decision

The first product generation mode is explicitly labelled direct completion. It proves prompt tokenization, admission, prefill, sampling, incremental decode, bounded output, cancellation, and cleanup without claiming model-independent chat behavior.

Add conversation-domain input and model-compatible prompt rendering only after direct completion works through E0, E1, and Slint.

## Rejected alternatives

- **Claim plain text input is chat:** rejected because chat semantics depend on the selected model’s rendering and stop conventions.
- **Hardcode one vendor chat template as a general API:** rejected because it would misrepresent compatibility.
- **Build a universal template abstraction before generation:** rejected because the kernel has not yet supplied the integration evidence needed to shape it.

## Consequences

- Early UI and API text must say “completion,” not “chat.”
- Conversation history is not part of the first generation milestone.
- The generation kernel can be tested with minimal prompt semantics.
- A later phase must add rendering compatibility, context planning, history, and stop tests before claiming chat support.

## Review trigger

Review after direct completion streams through the application façade, or earlier if the only viable first model requires a minimal documented wrapper to produce valid completion behavior.
