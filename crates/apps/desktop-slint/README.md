# desktop-slint

Thin Slint presentation adapter over `application-runtime`.

The crate owns the Slint event loop, per-user database path, widget callbacks,
one 16 millisecond frame cadence, fragment-only assistant-text appends with preserved
selection/viewport state, control/usage mapping, and response-terminal presentation.
It exposes model resolution/load/unload plus a conversation transcript, message
composer, send/regenerate/cancel, and clear-conversation controls for the verified
`TinyLlama/TinyLlama-1.1B-Chat-v1.0` profile. It does not directly import Candle,
Hugging Face, redb, Flume, tokenizer, or inference command types. Conversation
ownership, context planning, regeneration/supersession, and terminal attempt state
remain in E1.

The binary entry point delegates to the library so process startup remains lean.

Run with:

```text
cargo run -p desktop-slint
```
