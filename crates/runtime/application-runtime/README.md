# application-runtime

Frontend-neutral E1 orchestration for Candle local model selection, immutable
Hugging Face artifact resolution, persistence, lifecycle, direct completion,
compatible chat, bounded output, cancellation, unload, and explicit shutdown.

The crate owns no UI toolkit types. Slint, Tauri, TUI/CLI, or another host can
drive the same application state, conversation, and generation machines.

## Supported local execution

The sole local execution engine is Candle. `ModelSelection` contains a normalized
Hugging Face repository and revision. E1 resolves that selection to immutable
Safetensors Llama artifacts and a Hugging Face tokenizer, then constructs one
`CandleLlamaSource` behind its private composition boundary.

Resolved and loaded state report engine, artifact source, CPU device,
Safetensors format, supported scalar type, tokenizer vocabulary, and immutable
repository/commit identity as application-owned values. Those facts are derived
from the resolved product; callers cannot construct arbitrary engine, source,
format, or device combinations.

Concrete local composition uses one monomorphized
`HostedRuntime<CandleLlamaSource>`, one inference worker thread, one
`HfTokenizer`, and request-local `HfOwnedStreamingDecoder` values. E0 retains
exclusive ownership of model resources, sequences, scheduling, sampling,
cancellation boundaries, cleanup, accounting, unload, and shutdown.

## Completion and chat

Direct completion uses ordinary prompt-text encoding and is available for every
successfully loaded model.

Chat compatibility remains deliberately closed: only immutable artifact commit
`fe8a4ea1ffedaf415f4da2f062534de366a451e6` of
`TinyLlama/TinyLlama-1.1B-Chat-v1.0`, with tokenizer `</s>` mapped to EOS ID 2,
uses the built-in textual role renderer and matching EOS policy. E1 does not
infer a template from vocabulary size or model name. Unknown chat compatibility
returns an explicit error while direct completion remains available.

Request-local `ContextEntry` planning units are derived from raw conversation
state. Completed historical user/assistant turns are selected atomically while
diagnostics retain raw record identities. Units are selected by
`context-planner`, rendered in conversation order, exactly tokenized, and
corrected with a strictly shrinking bounded retry set before E0 admission.

Explicit application shutdown cooperatively stops and joins the sole E0 worker
and the bounded Hugging Face resolver worker. The independently stateful
corrective workflow remains owned by the `corrective-workflow` capability
engine.
