# application-runtime

Frontend-neutral application orchestration for model acquisition, persistence,
lifecycle, direct completion, compatible chat, bounded output, cancellation, and explicit shutdown.

The crate owns no UI toolkit types. Slint, Tauri, TUI/CLI, a headless node, or
another host can drive the same application behavior without duplicating model
lifecycle and generation state machines.

The first production composition is deliberately concrete: Candle CPU, Hugging
Face resolution/tokenization, redb, host workers, and E0 are wired here while the
product establishes its real seams. That composition is not the semantic
definition of E1. In-memory conversation records, response-attempt provenance,
regeneration, and context diagnostics contain no Slint, Candle source, provider SDK,
or transport identity.

Chat compatibility is deliberately closed: only
`TinyLlama/TinyLlama-1.1B-Chat-v1.0` with tokenizer `</s>` mapped to EOS ID 2 uses
the built-in textual role renderer and matching EOS policy. Unknown compatibility returns an
explicit error; the separate direct-completion API remains available. Request-local
`ContextEntry` values are derived from raw conversation state, selected by
`context-planner`, rendered in order, exactly tokenized, and corrected with a
strictly shrinking bounded retry set before E0 admission.

The independently stateful corrective workflow is owned by the
`corrective-workflow` capability engine.
