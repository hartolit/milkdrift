# ADR-0012: Keep local native composition private inside E1

- **Status:** Superseded by [ADR-0013](0013-candle-only-local-execution.md)
- **Date:** 2026-07-30
- **Superseded:** 2026-07-31

## Context

Phase 8 introduced a second production local backend. E1 now composes Candle/Safetensors artifacts resolved through Hugging Face and llama.cpp/GGUF artifacts selected from a local file, while E0 continues to own native model resources and token-level scheduling.

The second backend provides real evidence for evaluating the composition boundary, but it does not yet provide evidence for another runtime. `application-runtime` is the only consumer of this production composition, and the local composition has no lifecycle or API demand independent from E1's model selection, resolution, generation, conversation, unload, and shutdown semantics. Extracting it now would create a second coordinator whose boundary would be defined by E1's current implementation rather than by an independently reusable capability.

The public application façade also must remain usable by multiple frontends without exposing backend loader types or becoming generic over storage, resolver, tokenizer, or backend implementations.

## Decision

Keep `application-runtime` as the public, frontend-neutral, non-generic E1 façade.

- Isolate Candle, GGUF, Hugging Face, llama.cpp, tokenizer, and E0 production composition in E1's private closed `local.rs` module.
- Maintain two concrete monomorphized E0 workers: one for `CandleLlamaSource` and one for `GgufSource`. Route commands, events, token output, tokenizers, and streaming decoders through closed enum dispatch; do not introduce dynamic dispatch in token-sensitive execution.
- Expose only the two reviewed local products: Hugging Face Hub + Candle + Safetensors + CPU, and local file + llama.cpp + GGUF + CPU. Unsupported backend/source/device/format cross-products are not representable.
- Keep hosted-provider and peer execution outside the local product vocabulary and outside E0 backend selection.
- Keep redb persistence in E1 because its present ownership is application preferences and catalogue state, not an independently reusable local-model lifecycle.
- Do not create a separate local-model runtime crate yet, and do not create an `application-api` crate without a real process, browser, or network consumer.

## Rejected alternatives

- **Make `ApplicationRuntime` generic over every production dependency:** this would leak composition choices into every frontend and make the reusable façade harder to use without improving hot-path static dispatch.
- **Extract a local-model runtime immediately:** E1 is its only consumer, and no independent lifecycle or API currently separates the proposed runtime from E1 coordination. The extraction would create a second coordinator rather than a proven capability.
- **Use trait objects or a plugin registry for native backends:** the supported native set is closed, and static monomorphized E0 paths preserve the established token-sensitive execution model.
- **Represent hosted providers or peers as local backend variants:** those are coarse execution targets above E0, not native model-resource implementations.
- **Move redb or Hub composition solely to reduce E1 dependency count:** dependency count alone does not establish a new ownership boundary.
- **Create `application-api` preemptively:** there is no transported consumer whose serialization and compatibility requirements could define that API honestly.

## Consequences

- Frontends continue to use one non-generic E1 API and cannot construct Candle, GGUF, Hugging Face, or llama.cpp source types directly.
- One E1 lifecycle, generation, conversation, context, output, and shutdown state machine serves both local products.
- Native execution remains statically dispatched, at the cost of starting and explicitly shutting down two concrete E0 workers while only one application model is selected and resident at a time.
- E1 retains reviewed production dependencies on both native adapters, Hugging Face resolution/tokenization, host execution, and redb. This is accepted composition, not a claim that those concerns are application semantics.
- Adding another backend or product remains an explicit closed-set change with compatibility and shared-suite obligations.
- Phase 9 may narrow documentation, APIs, and internal modules, but it should not extract another runtime merely to reduce file size or dependency count.

## Review trigger

Review this decision during Phase 9 if local composition starts changing independently from E1 application behavior, gains another real consumer, or obscures E1's model lifecycle and generation semantics. Any extraction should produce an independently coherent capability rather than a second application coordinator. Also review if a real transported frontend establishes requirements for an `application-api` boundary.

The Candle-only architectural correction triggered this review. [ADR-0013](0013-candle-only-local-execution.md) replaces the two-worker/two-product composition while retaining the non-generic E1 façade, private concrete composition, and static token-sensitive execution.
