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

Phase 7 adds conversation semantics to E1 because every frontend should observe the
same message history, context policy, regeneration behavior, and cancellation
state. The context-selection algorithm remains in `context-planner`; prompt
rendering keeps a distinct compatibility boundary; corrective workflows live in
`corrective-workflow`. Coordination does not imply implementation ownership.

## Public boundary

Frontends construct `ApplicationRuntimeConfiguration`, start `ApplicationRuntime`,
inspect `ApplicationState`, submit model-lifecycle operations, and use the narrow
direct-completion API:

```text
start_generation(input, settings) -> RequestId
cancel_generation(request_id)
unload_model_with_behavior(behavior)
poll_event() -> Option<ApplicationEvent>
pull_output(callback)
```

`unload_model()` remains the default bounded-drain path. `ModelUnloadBehavior`
provides application-owned `RejectIfBusy`, `CancelActive`, and `Drain` choices
without exposing E0's `UnloadPolicy` contract to frontends.

`GenerationSettings` is owned by E1. It exposes stable completion controls rather
than re-exporting the sampling crate. E1 validates the settings, encodes the prompt
and textual stop suffixes once, and translates them into the E0 `GenerationRequest`.
Beginning/end special-token policy is explicit: the direct-completion mode encodes
ordinary prompt text without automatically adding boundary tokens.

Generated token IDs remain private below E1. E1 pulls bounded E0 token/state
batches, advances one owned request-local Hugging Face streaming decoder, and
republishes bounded UTF-8 text plus compact generation state. A token whose decoded
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

`ApplicationState` currently records:

- the loaded local model and its Candle/CPU execution target;
- the active request identity, phase, prompt/generated usage;
- whether generation can start or be cancelled;
- the last terminal completion/failure summary.

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
