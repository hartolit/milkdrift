# Application Runtime

## Responsibility

`application-runtime` is the E1 frontend-neutral use-case engine. It coordinates
cold-path infrastructure and direct-completion generation without absorbing tensor
execution or UI behavior.

It owns:

- persisted application preferences;
- one bounded synchronous Hub worker;
- immutable artifact resolution;
- tokenizer validation and vocabulary compatibility;
- exact repository/revision selection checks;
- one hosted `inference-runtime` endpoint;
- explicit single-model product residency;
- model load, bounded drain, terminal unload completion, and shutdown commands;
- direct-completion prompt encoding;
- stable application-level generation settings and translation to E0 contracts;
- one request-local owned streaming decoder;
- bounded token-to-text translation and frontend text pulls;
- public generation state, usage, cancellation, terminal results, and cleanup events;
- typed corrective workflow execution over `task-graph`;
- immutable in-process workflow artifacts and identifier-only routing;
- deterministic diagnostic normalization, retries, and terminal validation outcomes.

It does not own:

- Slint, Tauri, Leptos, terminal, or HTTP types;
- model tensors, logits, sampling workspaces, or backend sequence state;
- per-token generation scheduling;
- general chat-template/history rendering;
- OS-specific application-data path policy.

## Public boundary

Frontends construct `ApplicationRuntimeConfiguration`, start `ApplicationRuntime`,
inspect `ApplicationState`, submit model-lifecycle operations, and use the narrow
Phase 5 direct-completion API:

```text
start_generation(input, settings) -> RequestId
cancel_generation(request_id)
poll_event() -> Option<ApplicationEvent>
pull_output(callback)
```

`GenerationSettings` is owned by E1. It exposes stable completion controls rather
than re-exporting the sampling crate. E1 validates the settings, encodes the prompt
and textual stop suffixes once, and translates them into the E0 `GenerationRequest`.
Beginning/end special-token policy is explicit: the first direct-completion mode
encodes ordinary prompt text without automatically adding boundary tokens.

Generated token IDs remain private below E1. E1 pulls bounded E0 token/state batches,
advances one owned request-local Hugging Face streaming decoder, and republishes
bounded UTF-8 text plus compact generation state. A token whose decoded fragment
cannot yet be published is retained in E1-owned pending state until frontend output
capacity becomes available; E0 is not advanced by frontend per-token commands.

Corrective workflows use the separately composable
`CorrectiveWorkflowExecutor<M, V>`. Concrete model and validator services implement
`ModelTaskExecutor` and `ValidationTaskExecutor`; the E1 executor owns graph state,
retry accounting, immutable artifacts, diagnostic normalization, and identifier-only
workflow events.

The public boundary exposes application-owned values and `domain-contracts` types.
Candle tensors, Hugging Face implementation types, host-runtime accumulator types,
and inference commands/events remain private implementation details.

Immediate admission or queue failures are returned as `ApplicationError`.
Asynchronous worker outcomes are returned as structured `ApplicationEvent` values.
High-frequency generated text is pulled separately in borrowed batches. Vendor
failures are normalized into `ApplicationFailure` with a stable category and owned
cold-path diagnostic.

## Generation state

`ApplicationState` records:

- the loaded model and its Candle/CPU execution target;
- the active request identity, phase, prompt/generated usage;
- whether generation can start or be cancelled;
- the last terminal completion/failure summary.

Generation completion and E0 resource release remain distinct. `Terminal`, cleanup
pending/exhausted, and `Released` states are preserved in the pulled output stream,
and cleanup failures remain observable as low-frequency application events.

## Single-model policy

Phase 5 makes the initial single-model product decision explicit. E1 always configures
E0 for one resident model and does not expose a misleading `maximum_models` setting.
Multi-model application state and UI remain later work.

## Engine tiers

```text
frontend
   ↓
application-runtime (E1: prompt/tokenizer/text/public state)
   ↓
inference-runtime (E0: model/sequence/prefill/sample/decode/scheduler)
   ↓
adapters and feature contracts
```

E1 may depend on E0. E0 never imports E1. This keeps exact model-resource ownership
and the token-sensitive scheduler independent from repository, persistence, and
presentation workflows.

## Frontend replacement

A native Tauri backend or CLI runner can depend directly on `application-runtime`
and reuse the same model and generation APIs. A standalone browser frontend cannot
run Candle or redb directly; it should use a transport adapter to a native or remote
host that owns `ApplicationRuntime`.

Phase 6 adds Slint generation controls and frame-aligned text application. Phase 5
only establishes the reusable frontend-neutral product API and does not add chat
history or general prompt templates.
