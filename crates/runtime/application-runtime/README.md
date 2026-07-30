# application-runtime

Frontend-neutral E1 orchestration for local model selection, artifact resolution,
persistence, lifecycle, direct completion, compatible chat, bounded output,
cancellation, unload, and explicit shutdown.

The crate owns no UI toolkit types. Slint, Tauri, TUI/CLI, or another host can
drive the same application state, conversation, and generation machines.

## Supported local products

The public `ModelSelection` vocabulary is deliberately closed to two CPU
products:

| Selection | Source | Backend | Device | Format |
| --- | --- | --- | --- | --- |
| `HuggingFaceSafetensors` | Hugging Face Hub | Candle | CPU | Safetensors |
| `LocalGguf` | local file | llama.cpp | CPU | GGUF |

Hosted inference and peer inference are not selection variants. Backend, source,
device, and format are derived from `LocalModelProduct`, so unsupported product
cross-combinations are not representable. `ResolvedModel` and `LoadedModel`
expose application-owned product, scalar/quantization compatibility, and
immutable identity summaries; adapter source types remain private.

`ApplicationRuntime::resolve_model` and `ApplicationRuntime::load_model` accept
the complete selection. Hub resolution retains its existing immutable commit and
cached Safetensors behavior. Local GGUF resolution canonicalizes the file,
performs bounded metadata inspection, hashes its exact bytes, and constructs a
GGUF-native tokenizer verified against the same SHA-256 digest. Loading uses
`GgufSource::new_verified`, so mutation after resolution is rejected by E0 before
a model becomes resident. GGUF context, prefill, and micro-batch bounds are capped
by immutable model metadata from application-owned `ApplicationGgufConfiguration`
defaults; CPU threads, mmap, and mlock are also configured and validated by E1.

Concrete local composition is isolated in a private capability module. Candle
and GGUF each keep a monomorphized E0 worker, while commands, events, token
output, tokenizers, and owned streaming decoders use closed static dispatch to
only the selected active backend. There is still one public `ApplicationRuntime`
and one E1 state/conversation/generation machine. Explicit shutdown stops and
joins both E0 workers and the Hub worker.

## Completion and chat

Direct completion is available for both supported products and uses the
product's verified tokenizer and owned streaming decoder.

Chat compatibility remains deliberately closed: only immutable artifact commit
`fe8a4ea1ffedaf415f4da2f062534de366a451e6` of
`TinyLlama/TinyLlama-1.1B-Chat-v1.0`, with tokenizer `</s>` mapped to EOS ID 2,
uses the built-in textual role renderer and matching EOS policy. GGUF chat is
unsupported because no reviewed immutable profile evidence is registered; E1
does not infer a template from vocabulary size, model name, or embedded template
metadata. Unknown chat compatibility returns an explicit error while direct
completion remains available.

Request-local `ContextEntry` planning units are derived from raw conversation
state. Completed historical user/assistant turns are selected atomically while
diagnostics retain raw record identities. Units are selected by
`context-planner`, rendered in conversation order, exactly tokenized, and
corrected with a strictly shrinking bounded retry set before E0 admission.

The independently stateful corrective workflow remains owned by the
`corrective-workflow` capability engine.
