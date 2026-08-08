# application-runtime

Frontend-neutral E1 orchestration for Candle local model selection, immutable
Hugging Face artifact resolution, persistence, lifecycle, direct completion,
compatible chat, bounded output, cancellation, unload, and explicit shutdown.

The crate owns no UI toolkit types. Slint, Tauri, TUI/CLI, or another host can
drive the same application state, conversation, and generation machines.

## Supported local execution

The sole local execution engine is Candle. `ModelSelection` contains only a
normalized Hugging Face repository and revision. E1 resolves it to immutable
Safetensors Llama artifacts and a Hugging Face tokenizer independently from the
execution device, then constructs one `CandleLlamaSource` behind its private
composition boundary.

`ResolvedModel` reports artifacts, source, format, tokenizer, immutable identity,
compatibility, and optional configuration-declared scalar metadata. That declaration
is producer-intent evidence, not a tensor-homogeneity claim. Selected device is
separate E1 state using `ApplicationDevice::{Cpu, Cuda { ordinal: u32 }}` and
`ApplicationDeviceSummary`; `LoadedModel` reports only the actual execution scalar
and device from E0's verified load receipt. Detailed observed per-tensor data remains
below E1. No Candle or `cudarc` type crosses this public boundary.

CPU always exists and is the fresh-install default. Initial bounded discovery
probes CUDA 0 and, when different, the persisted selected CUDA ordinal. Structured
failure leaves an unavailable persisted CUDA selection visible and selected.
`select_device` follows `can_select_device`; load re-probes the selected device,
blocks unavailable selection, preserves it, and never falls back to CPU.

Concrete local composition uses one monomorphized
`HostedRuntime<CandleLlamaSource>`, one inference worker thread, one
`HfTokenizer`, and request-local `HfOwnedStreamingDecoder` values. E1 passes the
exact selected `ExecutionDevice`. E0 retains exclusive ownership of model
resources, sequences, scheduling, sampling, cancellation boundaries, cleanup,
accounting, unload, and shutdown.

The crate's non-default `cuda` feature forwards only to `candle-backend/cuda`.
No default graph reaches CUDA; generic `gpu`, `cudnn`, `flash-attn`, and `nccl`
features are not provided.

Generation code is split internally by responsibility into admission, the E0/text
bridge, bounded output, and settings. Runtime coordination is likewise organized
privately around startup, devices, model operations, retained cleanup, and event
lifecycle. This is source organization inside the existing E1 crate, not a new
layer or a claim of additional public API.

## Startup and incompatible-load cleanup

Startup is transactional across worker creation. If the Hub worker cannot start
after inference has started, E1 attempts bounded inference shutdown and join
before returning the primary Hub failure. A rollback timeout retains the complete
inference owner in a private startup-cleanup quarantine; a later startup retries
that cleanup instead of detaching the unresolved worker.

A successful E0 load receipt is published only after the admission ticket,
logical model ID and handle, immutable resolution/artifacts, optional configuration
declaration, supported compact observed-scalar classification, E0-verified execution
scalar, Llama/Candle/Safetensors evidence, tokenizer vocabulary and operations,
selected-versus-actual device, and bounded reserved footprint agree. The reserved
footprint is E0 admission/ownership accounting, not physical residency. E1 does not
compare the declaration with observed tensor dtypes or execution scalar and does not
reproduce Candle's conversion or device-aware scalar policy. If application-level
evidence is unsupported or disagrees, E1 keeps the incompatible `ModelHandle` and
compatibility failure in the existing private cleanup record while E0 continues to
own and account for the model. The public loaded-model state remains empty and the
application remains unloading while bounded unload submission retries proceed.
Submission exhaustion or E0 cleanup exhaustion does not discard that record; it is
released only after the model is confirmed absent or the inference worker is
confirmed disconnected or stopped. A load error that reports retained cleanup emits
an explicit pending or exhausted application event and returns to idle only when a
private E0 snapshot proves zero aggregate ownership. Successful unload clears actual
loaded execution facts but preserves resolution and selection.

## Memory and persistence

`AcceleratorMemoryPolicy` is `Automatic` or
`Limit { bytes: NonZeroU64 }`. E0's aggregate budget is fixed at startup, so
Automatic uses the least physical total across every CUDA row in the bounded
startup catalogue; an unavailable row or missing total contributes zero and
fails closed. A limit applies a lower user cap. Load re-probes require that the
fixed nonzero budget still fit the selected device's latest physical total;
otherwise loading stays disabled and returns a structured no-fallback error until
restart. CPU host budgeting is unchanged, while selected-device Candle planning
checks current available VRAM before partial residency. Host RAM is not used to
infer CUDA capacity, and no undocumented `u64::MAX` shortcut is used. One resident
model remains.

`LAS1` settings version 2 tags selected CPU/CUDA identity and memory policy.
Exact version 1 remains readable as CPU, with zero legacy device bytes mapped to
Automatic and nonzero bytes to Limit. New writes are version 2, fresh empty
repository defaults are valid, and unavailable persisted CUDA is not migrated.
`LAM1` model records are written as version 2 with optional configuration-declared
scalar metadata. Exact version 1 records remain readable in memory as present
configuration declarations. Per-tensor inventory, execution scalar, device, and cache
paths are not persisted.

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
and the bounded Hugging Face resolver worker. Its private state distinguishes
running, stopping, clean stop, retryable failure, and terminal failure. A command,
wait, or join timeout retains unfinished worker handles; a later `shutdown()` can
complete cleanly after the E0 shutdown succeeds. E0 cleanup exhaustion is terminal:
the structured failure remains sticky after worker exit/join, and process exit is
the reclamation boundary for the deliberately retained runtime allocation. The
independently stateful corrective workflow remains owned by the
`corrective-workflow` capability engine.
