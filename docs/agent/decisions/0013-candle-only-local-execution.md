# ADR-0013: Use Candle as the sole local execution engine

- **Status:** Accepted
- **Date:** 2026-07-31
- **Supersedes:** [ADR-0012](0012-local-native-composition.md)
- **Device dimension amended by:** [ADR-0019](0019-explicit-cuda-execution-foundation.md)

## Context

The first complete local inference slice used Candle with immutable Hugging Face artifacts and Safetensors on CPU. Phase 8 later added a llama.cpp/GGUF product and made E1 maintain two monomorphized E0 workers, backend routing, two tokenizer/decoder paths, GGUF-specific identity and configuration, and a second native toolchain even though E1 permits only one resident model.

That composition treated an execution engine, a serialization format, an artifact source, and an execution device as one product axis. The premise was incorrect: CPU or future GPU support does not require llama.cpp, and GGUF is a model format rather than an execution engine. The duplicate runtime path therefore adds ownership and shutdown complexity without establishing an independently required capability.

[ADR-0002](0002-candle-cpu-first-vertical-slice.md) correctly chose Candle CPU for the first vertical slice. Its Candle-first intent is affirmed. Its anticipated progression to a llama.cpp/GGUF product is replaced by this decision.

## Decision

Candle is the sole local execution engine.

The current supported local composition is:

- **execution engine:** Candle;
- **model format:** Safetensors;
- **artifact source:** immutable-revision Hugging Face artifacts resolved through the Rust adapter;
- **execution device:** mandatory/default CPU, plus explicit feature-gated CUDA only within ADR-0019’s accepted matrix.

These are separate architectural dimensions. A new format, artifact source, or device does not by itself justify another execution engine or another E0 ownership architecture.

E0 `inference-runtime` remains backend-neutral at its project-owned contracts and continues to own loaded model resources, sequences, request admission, token scheduling, sampling, cancellation boundaries, output backpressure, cleanup quarantine, accounting, unload, and shutdown. Production token-sensitive execution remains statically dispatched through one concrete Candle source and one hosted E0 worker. This decision does not introduce trait objects, a plugin registry, or public shared model ownership.

E1 `application-runtime` remains a frontend-neutral, non-generic façade. Its private local composition owns one Candle endpoint, one inference worker/thread, one Hugging Face tokenizer path, and one streaming decoder path. Frontends continue to receive application-owned state and commands rather than Candle loaders, sources, tensors, devices, tokenizers, or inference commands.

The llama.cpp adapter, GGUF product path, active-backend routing, dormant second worker, GGUF-specific public configuration and identity, and placeholder variants for unsupported products are removed. Hosted providers and peers remain coarse execution targets above E0 rather than local backend variants.

Candle-native GGUF or other quantized loading is not implemented by this correction. If pursued, it belongs under the Candle adapter and requires separate reviewed evidence for model-family compatibility, tokenizer provenance, quantization behavior, immutable artifacts, and supported devices. Current code and UI must not imply that support exists.

This decision supersedes ADR-0012. It retains ADR-0012's valid conclusions that E1 stays non-generic, concrete local composition stays private while it has one consumer, and token-sensitive execution stays statically dispatched.

## Rejected alternatives

- **Keep llama.cpp as a dormant or optional second engine:** a disabled path still preserves duplicate dependencies, public concepts, lifecycle code, native tooling, and maintenance obligations without a current product requirement.
- **Keep GGUF as a backend name or map GGUF permanently to llama.cpp:** serialization format and execution engine are independent concerns.
- **Replace the closed dispatch with a public plugin registry or trait objects:** there is one production engine, and E0's generic contracts already provide the required substitution boundary without dynamic dispatch in the hot path.
- **Make `ApplicationRuntime` generic over the Candle stack:** that would leak concrete composition into every frontend without improving resource ownership.
- **Implement Candle-native GGUF during this cleanup:** model compatibility, tokenization, quantization, and device behavior require their own reviewed implementation and evidence.
- **Move local inference lifecycle into a new runtime solely to reduce E1 dependencies:** one consumer and one composition do not establish an independent lifecycle or reusable capability.

## Consequences

- E1 starts, routes to, shuts down, and joins one local inference worker instead of two.
- Local inference has one concrete source, tokenizer, and decoder path while preserving E0's backend-neutral lifecycle and fault-injection coverage.
- Public application vocabulary reports only facts derived from the supported composition and no longer represents unsupported backend/source/format cross-products.
- The selected dependency graph no longer includes llama.cpp, its Rust bindings, or native build dependencies retained solely for that path.
- Direct completion, the verified TinyLlama chat profile, context planning, cancellation, bounded output, cleanup, unload, persistence, and explicit shutdown remain application requirements.
- CUDA support is deliberate, explicit, and limited by [ADR-0019](0019-explicit-cuda-execution-foundation.md); it does not introduce another engine or generic GPU capability. Candle-native GGUF remains separate future work.

## Review trigger

Review this decision only when evidence shows that Candle cannot satisfy a required local execution contract, or when a materially different execution model cannot honestly fit E0's ownership semantics. A request for another model format, artifact source, quantization, or device is not by itself a trigger for a second engine. Any proposal must include compatibility, lifecycle, cancellation, cleanup, dependency, tooling, and frontend-boundary evidence.
