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

Generation code is split internally by responsibility into admission, the E0/text
bridge, bounded output, and settings. This is source organization inside the
existing E1 crate, not a new layer or a claim of additional public API.

## Startup and incompatible-load cleanup

Startup is transactional across worker creation. If the Hub worker cannot start
after inference has started, E1 attempts bounded inference shutdown and join
before returning the primary Hub failure. A rollback timeout retains the complete
inference owner in a private startup-cleanup quarantine; a later startup retries
that cleanup instead of detaching the unresolved worker.

A successful E0 load receipt is published only after immutable identity,
descriptor, scalar, backend, quantization, and tokenizer evidence agree. If they
do not, E1 keeps the incompatible `ModelHandle` and compatibility failure in a
private cleanup record while E0 continues to own and account for the model. The
public loaded-model state remains empty and the application remains unloading
while bounded unload submission retries proceed. Submission exhaustion or E0
cleanup exhaustion does not discard that record; it is released only after the
model is confirmed absent or the inference worker is confirmed disconnected or
stopped.

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

## Shutdown

Explicit application shutdown cooperatively stops and joins the sole E0 worker
and the bounded Hugging Face resolver worker. Its private state progresses through
running, stopping, stopped, and failed/retryable states. A command, wait, or join
timeout returns a bounded error but retains unfinished worker handles; a later
`shutdown()` call retries the remaining work. A timeout does not detach either
worker. The independently stateful corrective workflow remains owned by the
`corrective-workflow` capability engine.
