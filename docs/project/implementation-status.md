# Current implementation status

**Status date:** 2026-07-30
**Reviewed source baseline:** `797ba0f` plus the current Phase 8 working tree
**Execution position:** Phase 8 is complete and canonically validated; Phase 9 is next
**Validation state:** the canonical full locked gate passed on the exact uncommitted Phase 8 working tree
**Canonical plan:** [LLM App Execution Plan](../agent/execution/execution-plan.md)
**Current working context:** [Phase 9 execution context](../agent/execution/current.md)

This is the canonical product-level status page. Component behavior belongs in the corresponding project guide, accepted rationale belongs in [architecture decisions](../agent/decisions/README.md), and phase-specific evidence belongs in [execution history](../agent/execution/history.md).

## Supported devices and products

| Product | Device | E0/native path | `application-runtime` (E1) | Slint UI |
|---|---|---|---|---|
| Hugging Face Hub + Candle Llama + Safetensors | CPU | Shared native generation contract | Direct completion plus verified TinyLlama Chat v1 | Closed selection; chat only when E1 reports the verified profile, otherwise direct completion |
| Local file + llama.cpp + GGUF | CPU | Shared native generation contract plus GGUF-native tokenizer | Direct completion only | Closed selection and direct completion |
| Candle or GGUF | CUDA/Metal/other GPU | No supported product path | No | No |
| Hosted provider or peer | Remote | Not an E0 backend | Not implemented | No |

The product is CPU-only and deliberately single-model at E1: exactly one selected model may be loaded at a time. `ApplicationRuntime` starts two concrete monomorphized E0 workers, one Candle and one GGUF, but routes one active local product through the shared application state machine. Both workers are stopped and joined during explicit application shutdown.

Direct completion is supported for both local products. Chat support is narrower: only the Hugging Face `TinyLlama/TinyLlama-1.1B-Chat-v1.0` artifact at immutable commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`, with tokenizer `</s>` resolving to token ID 2, receives the built-in role renderer and EOS policy. GGUF and every other unreviewed model remain direct-completion-only; E1 does not infer a chat template.

## Current runtime and composition boundaries

`application-runtime` remains the public frontend-neutral E1 façade and is not generic over production services. It owns application selection, model resolution, redb-backed preferences/catalogue state, one model lifecycle, direct completion, compatible conversation semantics, bounded decoded output, cancellation, unload policy, and explicit shutdown. No `application-api` crate exists.

Concrete local production composition is isolated in E1's private closed `local.rs` module as recorded by [ADR-0012](../agent/decisions/0012-local-native-composition.md):

- `HostedRuntime<CandleLlamaSource>` and `HostedRuntime<GgufSource>` remain separately monomorphized;
- commands, events, token output, tokenizers, and owned streaming decoders use closed enum/static dispatch;
- the public product vocabulary contains only Hugging Face/Candle/Safetensors/CPU and local-file/llama.cpp/GGUF/CPU;
- hosted and peer targets are not native backend variants;
- extraction into another runtime is deferred because E1 is the only consumer and no independent lifecycle/API has been demonstrated.

E0 still exclusively owns native model resources, sequence state, token-level scheduling and sampling, cancellation boundaries, cleanup, unload, and backend-contract verification. `corrective-workflow` remains an independent capability runtime rather than an E1 subsystem.

## GGUF tokenization and immutable identity

The GGUF path does not pair a model with an arbitrary Hugging Face tokenizer by vocabulary size.

- Local resolution canonicalizes the selected path, performs bounded GGUF metadata inspection, and computes SHA-256 before and after inspection so a concurrent content change fails.
- A llama.cpp vocabulary-only model supplies prompt encoding, boundary/control-token evidence, token-to-piece decoding, and request-local stateful streaming decode through the existing portable tokenization contracts.
- Tokenizer construction hashes the GGUF bytes before and after native vocabulary loading and retains the digest in the tokenizer.
- The verified `GgufSource`, resolved immutable identity, tokenizer digest, inspected metadata, loaded descriptor, capabilities, scalar/quantization values, vocabulary, and context limits must agree before the loaded model is accepted.
- Mutation after resolution is rejected rather than loading bytes different from the resolved SHA-256 identity.

## Shared behavior and UI integration

The same E0 native-backend suite is instantiated for Candle and GGUF. It covers load, generation start, prompt prefill, greedy decode, reproducible seeded sampling, EOS and token-limit completion, output backpressure, cancellation, released cleanup state, unload, empty post-unload accounting, shutdown, and worker join.

E1 also runs one shared direct-completion scenario for Candle and GGUF, covering prompt encoding, generation lifecycle, decoded text, usage, terminal release, unload, and shutdown. Existing E1 tests continue to cover cancellation, backpressure, unload policies, immutable selection checks, GGUF mutation rejection, worker disconnection, and the verified TinyLlama conversation path.

Slint exposes a closed two-product selector and sends only application-owned `ModelSelection` values to E1. It does not construct adapter sources or low-level GGUF configuration. Selected, resolved, and loaded summaries report backend, source, CPU device, format, scalar/quantization compatibility, and immutable identity. The composer uses E1 evidence to choose either:

- **Chat** for the one verified Hugging Face TinyLlama profile, preserving E1-owned history, rendering, context planning, and regeneration; or
- **Direct completion** for GGUF and every loaded model without verified chat compatibility, presenting one prompt/completion transcript without inferred roles or regeneration.

The frontend retains one 16 ms timer, processes at most 64 events per frame, performs one bounded output pull, and applies one frame-batched text update.

## Validation state

On 2026-07-30, the canonical full locked gate passed on the uncommitted Phase 8 working tree based on `797ba0f90b3eac154fe44ec871f4c7bf755a06ef`:

```text
git rev-parse HEAD
797ba0f90b3eac154fe44ec871f4c7bf755a06ef
cargo run --locked --bin llm-app -- verify
```

The gate validated the exact working tree's architecture/dependency policy, formatting, workspace checks, complete test/doctest suite, workspace strict Clippy, rustdoc, and benchmark compilation. It includes the native GGUF tokenizer/digest fixture, the shared Candle/GGUF E0 suite, the shared E1 direct-completion scenario, application lifecycle/compatibility coverage, and Slint presenter/generated-binding tests.

Focused Phase 8 validation also passed for `gguf-backend`, the shared native-backend target, `application-runtime`, and `desktop-slint`, including strict Clippy for the affected packages. Fixture regeneration remains byte-for-byte reproducible through the documented standard-library generator.

No manual external graphical acceptance session was performed. The recorded product evidence is download-free and fixture-based; it proves integration and lifecycle behavior, not external model availability, language quality, or a human-observed desktop session.

## Known limitations

- CPU is the only supported execution device.
- E1 supports one selected/resident model at a time even though two concrete E0 worker threads are maintained for static native dispatch.
- Chat compatibility is limited to the reviewed Hugging Face TinyLlama Chat v1 commit and tokenizer/EOS evidence; GGUF is direct-completion-only.
- Conversation history is in memory only; persistence and arbitrary branch trees are not implemented.
- Slint uses E1's default generation settings; no settings panel is exposed.
- The Candle and GGUF fixtures prove integration rather than language quality.
- Strict allocation-free Candle, Hugging Face, llama.cpp, or GGUF tokenization/decoding execution is not claimed because upstream libraries allocate internally.
- GPU execution, hosted-provider execution, peer execution, remote/browser transport, and `application-api` are not implemented.
- Manual graphical acceptance against external model artifacts remains unrecorded.

## Historical context

The [recovered implementation plan](implementation-plan.md) is retained as historical source material and is not authoritative. The active roadmap is the [execution plan](../agent/execution/execution-plan.md), the current working set is [current execution context](../agent/execution/current.md), and closed-phase evidence is consolidated in [execution history](../agent/execution/history.md).
