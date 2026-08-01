# Application runtime

## Responsibility

`application-runtime` is the E1 frontend-neutral application coordinator. It owns application semantics shared by frontends while delegating token-level inference and independently stateful capabilities to their own owners.

It currently owns:

- persisted application preferences and model catalogue state;
- a `ModelSelection` containing a normalized Hugging Face repository and revision;
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

No `application-api`, hosted-provider, peer-execution, GPU, local-file, or multiple-model boundary is implemented.

Conversation semantics live in E1 because every frontend must observe the same raw history, active-context policy, regeneration behavior, and cancellation state. Context selection remains in `context-planner`; the verified prompt renderer is internal compatibility logic; corrective workflows remain in `corrective-workflow`. Coordination does not imply implementation ownership.

## Selection, resolution, and reported facts

`ModelSelection` is a structure containing only:

```text
repository
revision
```

`resolve_model` validates the selection and sends it to the bounded Hub worker. `hf-hub-adapter` resolves the requested branch, tag, reference, or commit to an immutable Hub commit before downloading required artifacts. E1 loads the matching `tokenizer.json`, retains the exact resolution, and permits loading only while the complete visible repository/revision selection still matches.

Resolved and loaded models report application-owned facts derived from the supported composition:

| Fact | Current value/evidence |
|---|---|
| Engine | Candle |
| Artifact source | Hugging Face Hub |
| Device | CPU |
| Format | Safetensors |
| Scalar | F32, F16, or BF16 when supported and validated |
| Immutable identity | Hub repository plus resolved commit |
| Tokenizer evidence | validated vocabulary size and exact tokenizer used by E1 |

The public API does not accept arbitrary engine, source, device, or format combinations and exposes no Candle types. GGUF is unsupported; possible Candle-native GGUF or other quantized work is deferred to a separate reviewed implementation.

## Public boundary

Frontends construct `ApplicationRuntimeConfiguration`, start `ApplicationRuntime`, inspect `ApplicationState`, submit coarse model/generation operations, pull bounded output, and explicitly shut down:

```text
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

`local.rs` owns only the concrete Candle E0 endpoint. It has no active-backend switch, dispatch enum, dormant worker, or placeholder backend variant. E1 generation is internally split by responsibility into admission, the E0/text bridge, bounded output, and settings; that source organization does not create a new layer or imply new public façade operations. [ADR-0013](../agent/decisions/0013-candle-only-local-execution.md) supersedes the former two-worker Phase 8 composition while retaining private static dispatch and the non-generic façade.

Startup is a transaction across the two worker creations. Configuration, output storage, persistence, and preferences are prepared first; E1 then starts local inference and the Hub worker. If Hub startup fails after inference has started, E1 attempts bounded inference shutdown/join before returning the primary Hub startup failure. If the rollback bound expires, the complete `LocalInference` owner and timing policy remain in a private process-level cleanup quarantine; a later production startup retries one quarantined cleanup without holding the registry lock during the wait.

## Bounded output, cleanup, and unload

Generated token IDs remain private below E1. E1 pulls bounded E0 token/state batches, advances the active request's `HfOwnedStreamingDecoder`, appends each decoded fragment exactly once, and republishes bounded UTF-8 text plus compact generation state. A fragment that cannot yet be published remains pending until frontend capacity becomes available; no frontend command drives an individual token step.

Generation completion and E0 resource release remain distinct. Pulled output preserves `Terminal`, optional `CleanupPending`, optional `CleanupExhausted`, and `Released` states. E1 keeps the generation lifecycle active until release so unresolved ownership remains visible and conversation clearing stays blocked. Cleanup retry/exhaustion and accounting semantics are owned by E0 and described in [inference runtime](inference-runtime.md).

A model-load receipt is likewise not equivalent to a published resident model. E1 first compares the receipt with the retained immutable resolution, scalar, backend, quantization, and tokenizer evidence. On incompatibility, public loaded state remains empty while E1 stores the exact `ModelHandle`, compatibility failure, and automatic-unload state in a private cleanup record. E0 continues to own and account for the model. E1 retries bounded unload submission, and neither submission exhaustion nor E0 cleanup exhaustion discards the handle; the record remains private and accounted until absence, inference disconnection, or confirmed worker stop permits release.

The application configures E0 for one resident model and does not expose a misleading model-count setting. Multi-model application state is outside the current product boundary.

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

The current target is local Candle CPU execution through E0. A future peer or hosted target would use a coarse boundary above E0 for complete request admission, target capabilities, cancellation intent, bounded output, usage, and terminal state. It must not be represented as a local E0 backend. See [ADR-0008](../agent/decisions/0008-capability-and-execution-boundaries.md).
