# ADR-0002: Use Candle CPU for the first vertical slice

- **Status:** Accepted
- **Date:** 2026-07-22

## Context

The application façade and Slint frontend are already composed around Candle, Hugging Face artifact resolution, and the Hugging Face tokenizer. A GGUF CPU adapter exists at the lower backend boundary, but it is not wired through E1 and lacks an application-level tokenizer/product path. GPU correctness and feature-matrix infrastructure do not yet exist.

## Decision

Use the Candle CPU composition to prove the first complete prompt-to-stream generation slice. Establish correctness, cancellation, backpressure, cleanup, and baseline measurements on CPU before adding GGUF product parity or GPU execution.

## Rejected alternatives

- **Build Candle and GGUF product paths simultaneously:** rejected because tokenizer and composition differences would expand the first integration surface.
- **Start with GGUF because it supports quantized local files:** rejected because E1/UI already compose Candle and Hugging Face components.
- **Start with GPU execution:** rejected because device admission, fallback, CI matrices, and controlled measurements are not established.

## Consequences

- The first supported generation model is constrained to the Candle CPU model family already represented by the adapter.
- GGUF remains supported only at the adapter/E0 boundary until a shared generation contract passes.
- Performance findings from the first slice are CPU-specific and cannot be generalized to GPU behavior.

## Review trigger

Review after the Candle CPU real-model smoke path streams output and reliably cancels, unloads, and shuts down, or earlier if Candle cannot provide the semantics required by the backend-independent generation contract.
