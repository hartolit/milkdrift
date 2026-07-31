# desktop-slint

Thin Slint presentation adapter over `application-runtime` (E1).

## Supported local model flow

The frontend accepts a Hugging Face repository and revision. E1 resolves immutable Safetensors artifacts for Candle execution and reports the resolved Hub commit identity.

Current local execution uses the CPU. The UI presents engine, artifact source, device, model format, scalar type, and immutable identity as separate facts for selected, resolved, and loaded state. Repository and revision inputs remain locked while a lifecycle operation is busy or a model is loaded.

## Generation presentation

When E1 reports verified chat compatibility for the loaded model, the composer is labeled **Chat** and delegates message submission, regeneration, conversation ownership, prompt rendering, and conversation snapshots to E1.

Otherwise the composer is labeled **Direct completion** and delegates the prompt to E1's ordinary `start_generation` lifecycle. The frontend displays exactly one prompt/completion transcript, does not infer chat roles, history, or template semantics, and does not offer regeneration. Clearing is available only after active generation has ended and resets the presentation.

## Frontend boundary

This crate owns only the Slint event loop, widget callbacks, presentation mapping, and per-user database path. It preserves one 16 millisecond frame cadence, processes at most 64 E1 events per frame, performs one bounded output pull, and appends decoded text once per frame batch while preserving transcript selection and viewport state.

The crate depends on E1's application types and APIs. It does **not** import adapter crates, concrete Candle or Hugging Face source types, redb, Flume, tokenizer internals, or inference commands. Model resolution and loading, compatibility verification, immutable identity, generation lifecycle, cancellation, conversation state, and terminal ownership remain in E1.

The binary entry point delegates to the library so process startup remains lean.

Run with:

```text
cargo run -p desktop-slint
```
