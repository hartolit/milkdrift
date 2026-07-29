# Application runtime

## Responsibility

`application-runtime` is the E1 frontend-neutral application coordinator. It owns
application semantics shared by frontends while delegating token-level inference
and independently stateful capabilities to their own owners.

It currently owns:

- persisted application preferences;
- one bounded synchronous Hub worker;
- immutable artifact resolution;
- tokenizer validation and vocabulary compatibility;
- exact repository/revision selection checks;
- one hosted local `inference-runtime` endpoint;
- explicit single-model product residency;
- model load plus application-owned reject/cancel/drain unload behavior;
- terminal unload completion and bounded shutdown commands;
- direct-completion prompt encoding;
- frontend-neutral in-memory conversation records and response-attempt provenance;
- explicit `TinyLlama/TinyLlama-1.1B-Chat-v1.0` prompt/termination compatibility;
- request-local context derivation, deterministic planning, exact-token correction, and diagnostics;
- submit, regenerate, clear, and cancellation semantics for conversation turns;
- stable application-level generation settings and translation to E0 contracts;
- one request-local owned streaming decoder;
- bounded token-to-text translation and frontend text pulls;
- public generation state, usage, cancellation, terminal results, and cleanup events.

It does not own:

- Slint, Tauri, browser, terminal, or transport presentation types;
- model tensors, logits, sampling workspaces, or backend sequence state;
- per-token generation scheduling;
- corrective workflow execution or workflow artifact state;
- provider SDK/wire DTOs or peer transport implementations;
- OS-specific application-data path policy.

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
start_generation(input, settings) -> RequestId
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

`unload_model()` remains the default bounded-drain path. `ModelUnloadBehavior`
provides application-owned `RejectIfBusy`, `CancelActive`, and `Drain` choices
without exposing E0's `UnloadPolicy` contract to frontends.

`GenerationSettings` is owned by E1. It exposes stable completion controls rather
than re-exporting the sampling crate. Direct completion validates settings, encodes
ordinary prompt text without automatic boundary tokens, and translates the result
to E0. Chat preserves sampling controls but its compatibility profile replaces
caller EOS/text-stop values with the tested assistant-turn termination policy.

Generated token IDs remain private below E1. E1 pulls bounded E0 token/state
batches, advances one owned request-local Hugging Face streaming decoder, appends
each decoded fragment to the active assistant attempt exactly once, and republishes
bounded UTF-8 text plus compact generation state. A token whose decoded
fragment cannot yet be published remains in E1 pending state until frontend output
capacity becomes available; E0 is not advanced by frontend per-token commands.

The public boundary exposes application-owned values and stable domain types.
Candle tensors, Hugging Face implementation types, host-runtime accumulator types,
inference commands/events, provider DTOs, and transport connections remain private
to their implementation boundaries.

Immediate admission or queue failures are returned as `ApplicationError`.
Asynchronous worker outcomes are returned as structured `ApplicationEvent` values.
High-frequency generated text is pulled separately in borrowed batches. Vendor
failures are normalized into `ApplicationFailure` with a stable category and owned
cold-path diagnostic.

## Conversation and context semantics

Raw `ConversationRecord` values have stable record/attempt identities, monotonic
order, semantic role/content, provenance, retention policy, token estimate, and
assistant terminal state. They contain no local model handle, provider DTO, peer
connection, or transport state. Successful unsuperseded assistant attempts enter
the default active-context view; streaming, failed, cancelled, and superseded
attempts remain inspectable but are excluded.

For every chat request E1 derives temporary `ContextEntry` values, pins the target
user record and stored pinned content, reserves output positions, and invokes
`context-planner`. Selected records render in conversation order. E1 then tokenizes
the complete rendered prompt against the smaller of model context and prefill
capacity. Overflow removes exactly one planner-selected non-pinned entry per retry.
Attempts are bounded by the initially selected droppable count plus one; pinned-only
overflow returns `PinnedBudgetExceeded` and unchanged correction fails explicitly.
The admitted exact count is exposed through `ContextDiagnostics` and generation
usage.

The only built-in profile is `TinyLlama/TinyLlama-1.1B-Chat-v1.0`. Compatibility
requires `</s>` to resolve to EOS token ID 2. The role markers are verified template
text and need not each be one added token. Rendering and EOS token 2 are tested together. Unknown repositories
or incompatible tokenizer metadata return `UnsupportedChatCompatibility`; E1 never
applies this template to another model family.

Regeneration creates a new attempt for the latest responded-to user record and marks
all prior attempts for that turn superseded without deleting them. Clearing is
rejected while a conversation response is active. Conversation persistence and a
general branch tree are intentionally absent.

## Current composition versus application semantics

`ApplicationRuntime` currently constructs Candle CPU, Hugging Face, redb, host
workers, and E0 directly. That is accepted for the first production composition.
It should not be copied into another frontend, and it should not be mistaken for
the long-term semantic boundary.

A second local backend, deployment mode, or remote execution target is the trigger
for extracting the coarse composition seam supported by evidence. Do not replace
the present concrete code with a façade generic over every resolver, store,
tokenizer, backend, clock, and transport.

## Model execution targets

The current generation target is local E0. Future work may execute a model request
on a peer machine, rented GPU host, or hosted model service. Those targets operate
at request/stream granularity and are not E0 backends. E0 continues to describe
local native model ownership and token scheduling.

When a second execution kind is implemented, add a coarse boundary above E0 for:

- target identity and reported capabilities;
- complete generation request admission;
- cancellation intent and guarantees;
- bounded streamed output;
- usage and terminal state.

Local execution adapts this boundary to E0. Peer/provider implementations keep
their networking, authentication, request DTOs, and response translation in
adapter/composition code. Conversation records should contain semantic messages
and context policy, not execution connections. Context limits, message/prompt
format, token accounting, sampling controls, tools, and privacy boundaries are
explicit target capabilities rather than assumed common behavior.

See [ADR-0008](../agent/decisions/0008-capability-and-execution-boundaries.md).

## Generation state

`ApplicationState` records:

- the loaded local model and its Candle/CPU execution target;
- resolved chat compatibility (`Supported(TinyLlamaChatV1)` or `Unsupported`);
- the active request identity, phase, exact prompt/generated usage;
- whether generation can start or be cancelled;
- the last terminal completion/failure summary.

Raw conversation history and context diagnostics are queried separately from this
compact lifecycle state.

Generation completion and E0 resource release remain distinct. `Terminal`, cleanup
pending/exhausted, and `Released` states are preserved in the pulled output stream,
and cleanup failures remain observable as low-frequency application events. Remote
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
application-runtime (E1: application semantics and coordination)
      ├── corrective-workflow and future capability engines
      └── inference-runtime (E0: current local model execution)
              ↓
          adapters/features
```

E1 may depend on capability engines and E0. Neither may depend on E1. This keeps
application policy centralized without turning E1 into the implementation home for
every subsystem it coordinates.

## Frontend and node replacement

A native Slint, Tauri, TUI/CLI, or headless host can call E1 directly and reuse the
same application behavior. A browser frontend requires an explicit transport to a
native or remote host. The node/service lifetime is independent from an attached
frontend; closing a terminal or window must not define server lifetime.

The current E1 boundary establishes the reusable frontend-neutral product API.
Transport DTOs should be introduced only when a real separate-process/browser
consumer exists, rather than serializing E1 internals preemptively.
