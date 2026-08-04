# Application runtime

## Responsibility

`application-runtime` is the E1 frontend-neutral application coordinator. It owns application semantics shared by frontends while delegating token-level inference and independently stateful capabilities to their own owners.

It currently owns:

- persisted application preferences and model catalogue state;
- a `ModelSelection` containing a normalized Hugging Face repository and revision;
- application-owned CPU/CUDA selection, bounded discovery, availability diagnostics, and accelerator-memory policy;
- one bounded synchronous Hub resolver worker;
- immutable artifact identity, tokenizer validation, vocabulary compatibility, and complete-selection checks;
- one process-hosted, monomorphized Candle E0 worker/thread behind a private local composition boundary;
- explicit single-model residency;
- model load plus application-owned reject/cancel/drain unload behavior;
- direct-completion prompt encoding and concrete Hugging Face streaming decode;
- frontend-neutral in-memory conversation records and response-attempt provenance;
- exact immutable-artifact TinyLlama Chat v1 prompt/termination compatibility;
- turn-atomic context derivation, deterministic planning, exact-token correction, and diagnostics;
- submit, regenerate, clear, and cancellation semantics for compatible conversation turns;
- stable application-level generation settings and translation to E0 contracts;
- bounded token-to-text translation and frontend text pulls;
- public model metadata, generation state, usage, terminal results, and cleanup events;
- transactional rollback of the already-started inference worker when Hub-worker startup fails;
- private retention and cleanup accounting for an incompatible loaded-model handle;
- bounded shutdown with retryable joins and retained terminal inference failure.

It does not own Slint or transport presentation types, model tensors/logits/backend sequences, per-token scheduling, corrective workflow execution, provider/peer transports, or OS-specific application-data path policy.

No `application-api`, hosted-provider, peer-execution, local-file, multiple-model, Metal, or generic GPU abstraction is implemented; no generic `gpu` feature alias exists.

Conversation semantics live in E1 because every frontend must observe the same raw history, active-context policy, regeneration behavior, and cancellation state. Context selection remains in `context-planner`; the verified prompt renderer is internal compatibility logic; corrective workflows remain in `corrective-workflow`. Coordination does not imply implementation ownership.

## Model selection, device selection, resolution, and reported facts

`ModelSelection` is a structure containing only:

```text
repository
revision
```

Execution-device selection is separate application state. Its public vocabulary is `ApplicationDevice::{Cpu, Cuda { ordinal: u32 }}` plus `ApplicationDeviceSummary` and application-owned compute-capability, unavailability, and discovery-diagnostic values. No Candle or `cudarc` type crosses public E1. CPU always exists in the bounded catalogue and is the fresh-install default.

Initial bounded discovery probes CUDA ordinal 0. When persisted selection names a different CUDA ordinal, E1 also probes that ordinal so the persisted identity remains represented. Probe failures become structured application-owned summaries/diagnostics. An unavailable persisted CUDA device remains selected and visible; E1 neither migrates it to CPU nor silently falls back. `ApplicationRuntime::select_device` changes selection only while `ApplicationState::can_select_device` permits it.

`resolve_model` validates the repository/revision selection and sends it to the bounded Hub worker. `hf-hub-adapter` resolves the requested branch, tag, reference, or commit to an immutable Hub commit before downloading required artifacts. E1 loads the matching `tokenizer.json`, retains the exact resolution, and permits loading only while the complete visible repository/revision selection still matches.

Resolution is device-independent. `ResolvedModel` reports only resolved artifacts, source, format, source scalar, tokenizer evidence, immutable identity, and compatibility evidence. It contains neither selected-device state nor an execution scalar or actual execution device. `LoadedModel` reports the verified source scalar plus the actual execution scalar and device only after E0 returns a receipt and E1 validates them. Source and execution scalars may differ; E1 does not reproduce Candle's device-aware scalar policy or require equality. Unloading clears the loaded source scalar, execution scalar, and actual-device facts while preserving application selection and the resolved source evidence.

The public API does not accept arbitrary engine, source, format, or device cross-products. The implemented compatibility path is Llama through Candle with immutable Hugging Face Safetensors; GGUF is unsupported and requires a separate reviewed implementation.

## Public boundary

Frontends construct `ApplicationRuntimeConfiguration`, start `ApplicationRuntime`, inspect `ApplicationState` including the device catalogue/selection, submit coarse model/generation operations, pull bounded output, and explicitly shut down:

```text
select_device(application_device)
resolve_model(selection)
load_model(&selection)
start_generation(input, settings) -> RequestId
can_submit_chat_message() -> bool
submit_user_message(content, settings) -> RequestId
regenerate_last_response(settings) -> RequestId
clear_conversation()
conversation() -> &[ConversationRecord]
context_diagnostics() -> Option<&ContextDiagnostics>
cancel_generation(request_id)
unload_model_with_behavior(behavior)
poll_event() -> Option<ApplicationEvent>
pull_output(callback)
shutdown()
```

`unload_model()` is the default bounded-drain path. `ModelUnloadBehavior` provides `RejectIfBusy`, `CancelActive`, and `Drain` without exposing E0's unload policy type.

`GenerationSettings` is owned by E1. Direct completion validates settings, encodes ordinary prompt text without automatic boundary tokens, and translates the result to E0. Chat preserves sampling controls but its compatibility profile replaces caller EOS/text-stop values with the tested assistant-turn termination policy.

Immediate admission or queue failures return `ApplicationError`. Asynchronous worker outcomes return structured `ApplicationEvent` values. High-frequency decoded text is pulled separately in borrowed bounded batches. Vendor failures are normalized into `ApplicationFailure` with a stable category and owned cold-path diagnostic. Hub access tokens are redacted from adapter and application configuration `Debug` output.

## Completion and exact chat compatibility

Direct completion is available for every successfully loaded model.

The only built-in chat profile is:

- repository: `TinyLlama/TinyLlama-1.1B-Chat-v1.0`;
- immutable commit: `fe8a4ea1ffedaf415f4da2f062534de366a451e6`;
- tokenizer requirement: `</s>` maps to token ID 2;
- rendering: the verified TinyLlama role markers;
- termination: EOS token 2.

Unknown repositories, other commits, or incompatible tokenizer metadata return `UnsupportedChatCompatibility`; E1 never infers a template from model name or vocabulary size. Direct completion remains available when chat compatibility is unsupported.

## Conversation and context semantics

Raw `ConversationRecord` values have stable record/attempt identities, monotonic order, semantic role/content, provenance, retention policy, token estimate, and assistant terminal state. Records contain no local model handle, provider DTO, peer connection, or transport state. Successful unsuperseded assistant attempts enter the default active-context view; streaming, failed, cancelled, and superseded attempts remain inspectable but are excluded.

For each chat request E1 derives temporary planning units and `ContextEntry` values, pins the target user and stored pinned content, reserves output positions, and invokes `context-planner`. A completed historical user message and its active successful assistant response form one atomic unit: both records are selected or dropped together. Diagnostics expand units back to raw `ConversationRecordId` values.

Selected records render in conversation order and are exactly tokenized against the smaller of model context and prefill capacity. Overflow removes exactly one planner-selected non-pinned unit per retry. Attempts are bounded by the initially selected droppable-unit count plus one; pinned-only overflow and unchanged correction fail explicitly. `ContextDiagnostics` exposes the final estimate, exact admitted token count, selected/dropped records, reserved output, and render-attempt count.

Regeneration creates a new attempt only when the newest semantic record is an assistant response attempt. A later unanswered user record blocks regeneration of an older turn. Prior attempts are superseded without deletion. Clearing is rejected until generation, including backend cleanup, reaches release. Conversation persistence and a general branch tree are intentionally absent.

## Private concrete composition

`ApplicationRuntime` remains the local composition root because it is the only consumer and no independent local-composition lifecycle or API has been demonstrated.

```text
ApplicationRuntime
├── LocalInference
│   ├── HostedRuntime<CandleLlamaSource>
│   └── one inference RuntimeThread
├── one bounded Hub worker/thread
├── one resolved HfTokenizer
├── one request-local HfOwnedStreamingDecoder per generation
├── one resident-model/application lifecycle
└── redb application persistence
```

`local.rs` owns only the concrete Candle E0 endpoint. It maps the selected `ApplicationDevice` to the exact domain `ExecutionDevice`; it has no active-backend switch, dispatch enum, dormant worker, or placeholder backend variant. E1 generation is internally split by responsibility into admission, the E0/text bridge, bounded output, and settings. Runtime coordination is privately organized around startup, devices, model operations, retained cleanup, and event lifecycle. That source organization does not create a new layer or imply new public façade operations. [ADR-0013](../agent/decisions/0013-candle-only-local-execution.md) supersedes the former two-worker Phase 8 composition while retaining private static dispatch and the non-generic façade.

Startup is a transaction across the two worker creations. Configuration, output storage, persistence, preferences, and bounded device discovery are prepared first; E1 then starts local inference and the Hub worker. If Hub startup fails after inference has started, E1 attempts bounded inference shutdown/join before returning the primary Hub startup failure. If the rollback bound expires, the complete `LocalInference` owner and timing policy remain in a private process-level cleanup quarantine; a later production startup retries one quarantined cleanup without holding the registry lock during the wait.

## Accelerator memory and persistence

`AcceleratorMemoryPolicy` is explicit: `Automatic` or `Limit { bytes: NonZeroU64 }`. E0's aggregate budget is fixed at startup, so `Automatic` uses the least reported physical total across every CUDA row in the bounded startup catalogue; an unavailable row or a row without a physical total contributes zero and therefore fails closed. `Limit` applies the lower of that safe capacity and the user cap. Before load, E1 re-probes the selected CUDA device and admits work only when the fixed budget is nonzero and does not exceed the latest physical total; a changed or newly discovered capacity that cannot bound the process budget requires restart and produces a structured no-fallback error. Existing CPU host-memory budget behavior is unchanged. Candle planning separately checks current available VRAM before partial residency begins. E1 does not infer accelerator capacity from host RAM and does not use an undocumented `u64::MAX` device-budget shortcut. The product still admits at most one resident model.

Application settings use the `LAS1` schema at version 2. Version 2 explicitly tags selected CPU/CUDA identity and accelerator-memory policy. Exact version 1 records remain readable: they select CPU, map zero legacy device bytes to `Automatic`, and map a nonzero value to `Limit`. New writes are version 2. A fresh database with an empty default repository is valid. Model catalogue records remain `LAM1` version 1 and persist source scalar only; device-dependent execution scalar is not persisted. Loading settings never rewrites an unavailable persisted CUDA selection to CPU.

## Bounded output, cleanup, and unload

Generated token IDs remain private below E1. E1 pulls bounded E0 token/state batches, advances the active request's `HfOwnedStreamingDecoder`, appends each decoded fragment exactly once, and republishes bounded UTF-8 text plus compact generation state. A fragment that cannot yet be published remains pending until frontend capacity becomes available; no frontend command drives an individual token step.

Generation completion and E0 resource release remain distinct. Pulled output preserves `Terminal`, optional `CleanupPending`, optional `CleanupExhausted`, and `Released` states. E1 keeps the generation lifecycle active until release so unresolved ownership remains visible and conversation clearing stays blocked. Cleanup retry/exhaustion and accounting semantics are owned by E0 and described in [inference runtime](inference-runtime.md).

Load admission passes the exact selected domain `ExecutionDevice` and retained source scalar evidence to E0. A model-load receipt is not equivalent to a published resident model: E1 validates the admission ticket, logical model ID and handle, immutable resolution and artifact set, resolved/descriptor/artifact source scalar agreement, a supported and source-coherent receipt-reported execution scalar, Llama/Candle/Safetensors compatibility evidence, tokenizer vocabulary, selected device versus requested and receipt-reported actual device, and bounded reserved footprint. That reserved footprint is E0 admission/ownership accounting, not physical residency. E0 has already verified the execution scalar against its accepted load plan and loaded backend model; E1 validates only the source/execution evidence pair, without inferring scalar from device or reproducing Candle's device-aware planner. Unsupported execution scalar or any other receipt disagreement uses the private incompatible-model cleanup path. Loading re-probes the selected device first; an unavailable selection or a latest physical total that cannot bound the startup-fixed budget produces a structured error, remains selected, and is not replaced by CPU.

On any receipt mismatch, public `LoadedModel` state remains empty while E1 uses the existing private incompatible-model unload/retention path. It stores the exact `ModelHandle`, compatibility failure, and automatic-unload state while E0 continues to own and account for the model. E1 retries bounded unload submission, and neither submission exhaustion nor E0 cleanup exhaustion discards the handle; the record remains private and accounted until absence, inference disconnection, or confirmed worker stop permits release. A failed E0 load that itself reports retained cleanup likewise leaves E1 unloading and keeps device selection locked. For a retryable cleanup failure, E1 submits one bounded private snapshot inspection and returns to idle only when aggregate E0 state proves zero retained model, request, cleanup, and reservation ownership; inspection failure, nonzero ownership, or cleanup exhaustion stays locked.

The application configures E0 for one resident model and does not expose a misleading model-count setting. Successful unload clears the loaded source scalar, receipt-verified execution scalar, and receipt-verified actual device while preserving resolved source evidence and the separately selected application device. Multi-model application state is outside the current product boundary.

## Shutdown

Normal closure must call `ApplicationRuntime::shutdown`; `Drop` does not perform an unbounded join.

The private shutdown controller tracks running, stopping, cleanly stopped, retryable failure, and terminal failure. Shutdown:

1. stops application admission and requests cooperative Hub shutdown;
2. sends one ticketed shutdown command to the Candle E0 worker, retaining that ticket across retries;
3. waits only to configured checked deadlines;
4. attempts the inference-worker join and Hub-worker join even when an earlier step reports an error;
5. takes and joins a worker handle only after the worker is observed finished;
6. independently retains any terminal inference shutdown failure while continuing joins;
7. returns a retained terminal failure in preference to treating handle absence as cleanup success, otherwise returns the first bounded retryable failure.

If command submission, event wait, or worker join times out, status becomes retryable failure and every unfinished worker handle remains owned by `ApplicationRuntime`. A later `shutdown()` resumes the remaining stop/join work; a timeout does not detach a worker and may later become clean success when the E0 result was successful.

If E0 returns a shutdown failure, the E0 worker terminates after publishing that result. When cleanup is exhausted and backend resources remain, E0 deliberately retains its runtime allocation until process exit. E1 stores the structured `RuntimeError` as terminal state independently from the join handle, joins the worker normally when possible, and returns the normalized failure on every later shutdown call. A missing handle or endpoint disconnection cannot overwrite that terminal state or establish clean completion. Clean idempotent success requires both observed clean E0 shutdown and confirmed joins.

Shutdown and join timeouts are validated before any worker starts as nonzero and no greater than 24 hours. Runtime deadline construction retains checked arithmetic as defense in depth, so invalid timing cannot enter the startup-cleanup quarantine.

An in-flight synchronous Hub operation has no upstream global cancellation handle, and the same safe-Rust limitation applies to an uncooperative in-process backend call. The bounded call may therefore return before the worker finishes, but ownership of the handle remains available for shutdown retry. After explicit backend cleanup exhaustion, process termination—not ordinary Rust drop—is the retained allocation's reclamation boundary. [ADR-0006](../agent/decisions/0006-explicit-bounded-shutdown.md) records the policy.

## Model execution targets

The implemented local target is Candle through E0 with explicit CPU or feature-gated CUDA selection. CPU remains the fresh-install and default-build path. `application-runtime/cuda` forwards only to `candle-backend/cuda`; the complete desktop feature chain and exclusions are canonical in [dependency policy](dependency-policy.md). Explicit CUDA failure never falls back to CPU.

A future peer or hosted target would use a coarse boundary above E0 for complete request admission, target capabilities, cancellation intent, bounded output, usage, and terminal state. It must not be represented as a local E0 backend. See [ADR-0008](../agent/decisions/0008-capability-and-execution-boundaries.md).
