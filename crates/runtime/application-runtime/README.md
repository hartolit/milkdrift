# application-runtime

Frontend-neutral application orchestration for model acquisition, persistence,
lifecycle, direct completion, bounded output, cancellation, and explicit shutdown.

The crate owns no UI toolkit types. Slint, Tauri, TUI/CLI, a headless node, or
another host can drive the same application behavior without duplicating model
lifecycle and generation state machines.

The first production composition is deliberately concrete: Candle CPU, Hugging
Face resolution/tokenization, redb, host workers, and E0 are wired here while the
product establishes its real seams. That composition is not the semantic
definition of E1. Conversation state must not depend on Slint, Candle source
types, provider SDKs, or transport DTOs.

The independently stateful corrective workflow is owned by the
`corrective-workflow` capability engine.
