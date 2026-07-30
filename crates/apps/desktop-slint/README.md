# desktop-slint

Thin Slint presentation adapter over `application-runtime` (E1).

## Supported local products

The frontend exposes one closed selector with exactly the two product combinations owned by E1:

1. **Candle + Hugging Face Hub + Safetensors** — the user supplies a repository and revision. E1 resolves immutable Hub artifacts and reports the resolved commit identity.
2. **llama.cpp + local file + GGUF** — the user supplies a local GGUF path. E1 inspects the file and reports the SHA-256 identity of the exact bytes.

Both products run on CPU. The UI displays the selected, resolved, and loaded backend, source, device, format, scalar type, quantization, and immutable identity. Product selection and source inputs remain locked while a lifecycle operation is busy or a model is loaded. llama.cpp execution tuning remains an E1 concern and is intentionally not exposed by this frontend.

## Generation presentation

When E1 reports verified chat compatibility for the loaded model, the composer is labeled **Chat** and delegates message submission, regeneration, conversation ownership, prompt rendering, and conversation snapshots to E1.

Otherwise the composer is labeled **Direct completion** and delegates the prompt to E1's ordinary `start_generation` lifecycle. The frontend displays exactly one prompt/completion transcript, does not infer chat roles, history, or template semantics, and does not offer regeneration. Clearing is available only after active generation has ended and resets the presentation.

## Frontend boundary

This crate owns only the Slint event loop, widget callbacks, presentation mapping, and per-user database path. It preserves one 16 millisecond frame cadence, processes at most 64 E1 events per frame, performs one bounded output pull, and appends decoded text once per frame batch while preserving transcript selection and viewport state.

The crate depends on E1's closed application types and APIs. It does **not** import adapter crates, Candle/GGUF/Hugging Face concrete source types, redb, Flume, tokenizer internals, inference commands, or low-level GGUF execution configuration. Model resolution/loading, compatibility verification, immutable identity, generation lifecycle, cancellation, conversation state, and terminal ownership remain in E1.

The binary entry point delegates to the library so process startup remains lean.

Run with:

```text
cargo run -p desktop-slint
```
