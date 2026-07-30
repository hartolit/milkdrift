# Application runtime

## Responsibility

`application-runtime` is the E1 frontend-neutral application coordinator. It owns
application semantics shared by frontends while delegating token-level inference
and independently stateful capabilities to their own owners.

It currently owns:

- persisted application preferences;
- a closed `ModelSelection` for Hugging Face Hub + Candle + Safetensors CPU and local file + llama.cpp + GGUF CPU;
- one bounded synchronous Hub resolver worker and synchronous local GGUF resolution;
- immutable artifact identity, tokenizer validation, vocabulary compatibility, and complete-selection checks;
- two process-hosted, monomorphized E0 workers behind one private local capability boundary;
- routing of lifecycle, generation, events, and output through exactly one active local backend;
- explicit single-model product residency;
- model load plus application-owned reject/cancel/drain unload behavior;
- terminal unload completion and bounded shutdown of both E0 workers;
- direct-completion prompt encoding for both products;
- closed Hugging Face/GGUF tokenizer and request-local streaming-decoder dispatch;
- frontend-neutral in-memory conversation records and response-attempt provenance;
- explicit immutable-artifact `TinyLlama/TinyLlama-1.1B-Chat-v1.0` prompt/termination compatibility;
- request-local turn-atomic context derivation, deterministic planning, exact-token correction, and diagnostics;
- submit, regenerate, clear, and cancellation semantics for compatible conversation turns;
- stable application-level generation settings and translation to E0 contracts;
- bounded token-to-text translation and frontend text pulls;
- public model metadata, generation state, usage, cancellation, terminal results, and cleanup events.

It does not own:

- Slint, Tauri, browser, terminal, or transport presentation types;
- model tensors, logits, sampling workspaces, or backend sequence state;
- per-token generation scheduling;
- corrective workflow execution or workflow artifact state;
- provider SDK/wire DTOs or peer transport implementations;
- OS-specific application-data path policy.

No `application-api`, hosted-provider, peer-execution, or GPU boundary is implemented.

Conversation semantics live in E1 because every frontend must observe the same raw
history, active-context policy, regeneration behavior, and cancellation state. The
context-selection and exact-correction candidate order remain in `context-planner`;
the first prompt renderer is an internal compatibility module rather than a new
crate; corrective workflows remain in `corrective-workflow`. Coordination does not
imply implementation ownership.

## Public boundary

Frontends construct `ApplicationRuntimeConfiguration`, start `ApplicationRuntime`,
inspect `ApplicationState`, submit model-lifecycle operations, and use completion
or compatible conversation operations:

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
```

`ModelSelection` is deliberately closed. Its two variants fix backend, source,
device, and format as reviewed product combinations; callers cannot construct a
Candle/GGUF, Hub/llama.cpp, non-CPU, hosted, or peer cross-product. Resolved and
loaded state report the derived backend, source, device, and format together with
scalar type, quantization, tokenizer vocabulary, and immutable commit or SHA-256
identity.

`unload_model()` remains the default bounded-drain path. `ModelUnloadBehavior`
provides application-owned `RejectIfBusy`, `CancelActive`, and `Drain` choices
without exposing E0's `UnloadPolicy` contract to frontends.

`GenerationSettings` is owned by E1. It exposes stable completion controls rather
than re-exporting the sampling crate. Direct completion validates settings, encodes
ordinary prompt text without automatic boundary tokens, and translates the result
to E0. Chat preserves sampling controls but its compatibility profile replaces
caller EOS/text-stop values with the tested assistant-turn termination policy.

Generated token IDs remain private below E1. E1 pulls bounded E0 token/state
batches, advances one owned request-local decoder selected by its closed Hugging
Face/GGUF dispatch, appends each decoded fragment to the active assistant attempt
exactly once, and republishes bounded UTF-8 text plus compact generation state. A
token whose decoded fragment cannot yet be published remains in E1 pending state
until frontend output capacity becomes available; E0 is not advanced by frontend
per-token commands.

The public boundary exposes application-owned values and stable domain types.
Candle tensors, llama.cpp/GGUF native values, Hugging Face tokenizer instances,
host-runtime accumulator types, inference commands/events, provider DTOs, and
transport connections remain private to their implementation boundaries. Tokenizer
vocabulary inspection returns only a project-owned token identifier. Hub access
tokens are redacted from both adapter and application configuration `Debug` output.

Immediate admission or queue failures are returned as `ApplicationError`.
Asynchronous worker outcomes are returned as structured `ApplicationEvent` values.
High-frequency generated text is pulled separately in borrowed batches. Vendor
failures are normalized into `ApplicationFailure` with a stable category and owned
cold-path diagnostic.

## Conversation and context semantics

Raw `ConversationRecord` values have stable record/attempt identities, monotonic
order, semantic role/content, provenance, retention policy, token estimate, and
assistant terminal state. Generated assistant usage is stored as a generated-token
estimate rather than mislabeled as an exact re-tokenization of decoded text. Records
contain no local model handle, provider DTO, peer connection, or transport state.
Successful unsuperseded assistant attempts enter the default active-context view;
streaming, failed, cancelled, and superseded attempts remain inspectable but are
excluded.

For every chat request E1 derives temporary planning units and `ContextEntry` values,
pins the target user and stored pinned content, reserves output positions, and invokes
`context-planner`. A completed historical user message and its active successful
assistant response form one atomic planning unit: both records are selected or dropped
together. This grouping remains an E1 conversation interpretation rather than changing
the generic planner into a chat-specific algorithm. Diagnostics expand selected and
dropped units back to raw `ConversationRecordId` values. Selected units render their
records in conversation order. E1 then tokenizes the complete rendered prompt against
the smaller of model context and prefill capacity. Overflow removes exactly one
planner-selected non-pinned unit per retry. Attempts are bounded by the initially
selected droppable-unit count plus one; pinned-only overflow returns
`PinnedBudgetExceeded` and unchanged correction fails explicitly. The admitted exact
count and the estimate for the final selected set are exposed through
`ContextDiagnostics` and generation usage.

The only built-in profile is `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable
artifact commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`. Compatibility also requires
`</s>` to resolve to EOS token ID 2. The role markers are verified template text and
need not each be one added token. Rendering and EOS token 2 are tested together.
Unknown repositories, other commits, or incompatible tokenizer metadata return
`UnsupportedChatCompatibility`; E1 never applies this template to another model or
unreviewed revision.

Regeneration creates a new attempt only when the newest semantic record is an
assistant response attempt. A later committed but unanswered user record blocks
regeneration of an older turn rather than creating an implicit branch. Prior attempts
for the regenerated turn are superseded without being deleted. Clearing is rejected
until the generation lifecycle, including backend cleanup, reaches release.
Conversation persistence and a general branch tree are intentionally absent.

## Private local composition boundary

`ApplicationRuntime` remains the local composition root. Its private `local.rs`
module is an internal local-model capability boundary: it starts separate
monomorphized E0 workers for `CandleLlamaSource` and `GgufSource`, retains closed
Hugging Face/GGUF tokenizer and decoder dispatch, and routes load, generate, cancel,
unload, events, and token output through the one active backend. The separate Hub
worker and redb persistence remain coordinated by E1.

This boundary stays private inside E1 because `ApplicationRuntime` is its only
consumer. Extracting a crate, public service trait, or generic façade would add a
second boundary without reuse evidence. [ADR-0012](../agent/decisions/0012-local-native-composition.md)
records the Phase 8 decision; a second real consumer or deployment is the review
trigger. Frontends must not copy
this composition or construct backend source values.

Both E0 workers are started with the application and participate in explicit
bounded shutdown, including the inactive endpoint. Shutdown sends distinct ticketed
commands and attempts bounded joins for both workers before completing Hub-worker
cleanup.

## Model execution targets

The current generation targets are the two local CPU products, both executed by
E0. `ModelSelection` has no peer, hosted-provider, or GPU variant, and there is no
`application-api` crate or transport DTO boundary. A future remote target would
operate at request/stream granularity rather than becoming an E0 backend. E0
continues to describe local native model ownership and token scheduling.

When a second execution kind is implemented, add a coarse boundary above E0 for:

- target identity and reported capabilities;
- complete generation request admission;
- cancellation intent and guarantees;
- bounded streamed output;
- usage and terminal state.

If this boundary is implemented, local execution can adapt it to E0. Peer/provider
implementations would keep their networking, authentication, request DTOs, and
response translation in adapter/composition code. Conversation records should contain semantic messages
and context policy, not execution connections. Context limits, message/prompt
format, token accounting, sampling controls, tools, and privacy boundaries are
explicit target capabilities rather than assumed common behavior.

See [ADR-0008](../agent/decisions/0008-capability-and-execution-boundaries.md).

## Generation state

`ApplicationState` records:

- the resolved and loaded model's complete selection, immutable identity, compatibility, and derived local execution target;
- backend, source, CPU device, format, scalar type, and quantization evidence;
- resolved chat compatibility (`Supported(TinyLlamaChatV1)` or `Unsupported`);
- the active request identity, phase, exact prompt/generated usage;
- whether generation can start or be cancelled;
- the last terminal completion/failure summary.

Raw conversation history and context diagnostics are queried separately from this
compact lifecycle state.

Generation completion and E0 resource release remain distinct. `Terminal`, cleanup
pending/exhausted, and `Released` states are preserved in the pulled output stream.
The assistant attempt becomes completed, cancelled, or failed when E1 accepts the
terminal generation state; later cleanup failure cannot leave a finished response
semantically marked as streaming. E1 keeps the generation lifecycle active until
release, so cleanup remains observable and conversation clearing stays blocked while
native ownership is unresolved. Remote
execution must expose its own honest terminal semantics rather than inheriting E0
cleanup states it does not own.

## Single-model policy

The local application composition deliberately configures E0 for one resident model
and does not expose a misleading `maximum_models` setting. Multi-model application
state is not part of the current product boundary; product-level support is tracked
in [implementation status](implementation-status.md).

## Engine relationships

```text
frontend / host
      ↓
application-runtime (E1: shared application semantics)
      ├── private local capability (one active backend)
      │     ├── monomorphized Candle E0 worker
      │     └── monomorphized GGUF E0 worker
      └── independently composed capability engines, when required
                    ↓
           platform / adapters / domain
```

E1 may depend on capability engines and E0. Neither may depend on E1. This keeps
application policy centralized without turning E1 into the implementation home for
every subsystem it coordinates.

## Frontend and node replacement

A native Slint, Tauri, TUI/CLI, or headless host can call E1 directly and reuse the
same application behavior. A browser frontend requires an explicit transport to a
native or remote host. The node/service lifetime is independent from an attached
frontend; closing a terminal or window must not define server lifetime.

The current E1 boundary is the reusable frontend-neutral in-process product API.
No separate `application-api` exists. Transport DTOs should be introduced only when
a real separate-process or browser consumer exists, rather than serializing E1
internals preemptively.
