# Current execution context

**Reviewed baseline:** `a81a3aefb999f2f5a70fee6d1830dd3f3811d2ba` (`Docs restructure`)
**Current target:** Phase 6 — first usable Slint product
**Gate state:** Phase 5 source closure is present; Phase 6 implementation remains gated on the canonical repository gate passing on the exact working tree after documentation-path repair
**Canonical plan:** [execution-plan.md](execution-plan.md)
**Current product truth:** [project implementation status](../../project/implementation-status.md)

This document is the dense working set for the active phase. It intentionally gathers the context most likely to be needed by an implementation agent, but it is **derived context**, not a replacement for the plan, ADRs, status page, or component guides. When one of those owners changes, update this file rather than letting it become a competing truth source.

## Immediate objective

Deliver the first UI that exercises the already-implemented direct-completion path end to end:

```text
resolve model
→ load Candle CPU model
→ enter prompt
→ start generation through E1
→ pull bounded decoded text on the UI frame cadence
→ show usage and terminal state
→ cancel safely when requested
→ unload after cleanup
→ close with bounded application shutdown
```

The phase is about **presentation and integration**, not a new inference path. E0 already owns token-level generation; E1 already owns prompt encoding, generation settings, decoded-text streaming, application state, cancellation, unload behavior, and the frontend-neutral generation API.

## Gate before implementation

Run the canonical repository gate from the exact tree being used for Phase 6:

```sh
cargo run --locked --bin llm-app -- verify
```

Use [project validation](../../project/validation.md) for the complete procedure and focused diagnostic commands.

The reviewed commit above is a documentation restructure and has no attached GitHub Actions status in the connected repository. Older Phase 3–5 validation remains historical evidence only. Do not treat it as validation of the post-restructure tree.

If this documentation patch is applied before Phase 6 work, the resulting uncommitted/committed tree is the tree that must pass the gate.

## What Phase 6 inherits

The implementation status and E1 guide are canonical; the following is the subset that matters most to this phase.

### E0 already owns the hot generation loop

`inference-runtime` owns:

- model and sequence resources;
- prompt prefill and incremental decode;
- sampling;
- scheduler fairness and bounded advancement;
- cancellation safe points;
- token-output backpressure;
- terminal cleanup, quarantine, retry, and release accounting.

The frontend must not submit one command per token or drive backend prefill/decode directly.

See [inference runtime](../../project/inference-runtime.md).

### E1 already exposes the frontend-neutral generation boundary

`application-runtime` already provides the product-level generation behavior required by the UI:

- direct-completion prompt encoding through the resolved tokenizer;
- application-owned `GenerationSettings`;
- generation start/cancel state;
- request-local owned streaming decode state;
- bounded translation from E0 token/state pulls to UTF-8/state pulls;
- application state and low-frequency events;
- single-model residency policy;
- reject/cancel/drain unload behavior;
- explicit bounded shutdown.

The public generation surface is intentionally coarse: the UI uses E1 rather than E0 commands, raw logits, backend sequence state, or vendor tokenizer/model types.

See [application runtime](../../project/application-runtime.md).

## Current Slint baseline

The current frontend is lifecycle-only.

### `crates/apps/desktop-slint/ui/app-window.slint`

The window currently contains:

- repository input;
- revision input;
- resolve/cache action;
- CPU load action;
- unload action;
- resolved commit display;
- status text.

It does **not** yet expose prompt input, generated text, generate/cancel controls, usage, terminal reason, or clear-output behavior.

### `crates/apps/desktop-slint/src/presenter.rs`

The presenter currently:

- connects resolve, load, and unload callbacks;
- runs a 16 ms repeated frame timer;
- drains at most 64 low-frequency `ApplicationEvent`s per frame;
- maps generation lifecycle events to status text even though generation controls are not wired;
- synchronizes lifecycle controls from `ApplicationState`.

The frame timer currently does **not** pull the E1 decoded-text output accumulator. Phase 6 should extend this existing cadence rather than create a second high-frequency update mechanism.

### `crates/apps/desktop-slint/src/lib.rs`

The runner already:

- constructs the application runtime;
- creates the Slint window;
- connects the presenter;
- starts/stops the frame timer;
- calls explicit `ApplicationRuntime::shutdown()` after the window loop exits.

Preserve that bounded shutdown path. Do not replace it with implicit `Drop` cleanup.

## Phase 6 work packages

### 6.1 Minimum generation interface

Add the smallest coherent direct-completion surface:

- prompt input;
- generated-output view;
- Generate action;
- Cancel action;
- Clear output action;
- status and terminal reason;
- prompt/generated token counts;
- visible CPU/Candle execution label;
- existing repository/revision/resolve/load/unload controls.

Do not build a large settings panel before this path is stable. A small set of sensible defaults or a compact expandable section is sufficient.

### 6.2 Frame-aligned output pulling

Extend the existing frame timer so one UI cadence performs bounded work in this order:

1. drain a bounded number of low-frequency application events;
2. pull one bounded E1 output batch;
3. apply decoded text/state to presentation state;
4. synchronize control enablement and usage from `ApplicationState`.

Do not send one Slint callback per generated token. Do not make token production depend on the frame clock. The UI is a bounded consumer of text already produced through the E0/E1 pipeline.

Avoid rebuilding the complete displayed transcript for every token. Apply text in batches and keep presentation-owned state separate from runtime-owned generation state.

### 6.3 Cancellation, unload, and shutdown

The UI must preserve lifecycle semantics already established below it:

- Cancel remains available while generation is active;
- cancellation can be pending until the next safe backend boundary;
- unload follows the E1 unload policy rather than inventing UI-specific resource behavior;
- another request starts only when application state permits it after cleanup;
- normal window closure runs bounded application shutdown;
- no normal exit path silently detaches the inference worker.

### 6.4 Presenter-focused tests

Keep testable state mapping out of generated Slint callbacks where practical. Add focused coverage for:

- enable/disable state of Generate, Cancel, Resolve, Load, and Unload;
- applying bounded text batches;
- prompt/generated usage display;
- terminal and failure presentation;
- cancellation-pending state;
- unload after generation;
- clear-output presentation behavior without mutating runtime history incorrectly.

Use application/runtime integration tests—not UI rendering tests—for behavior that belongs below presentation.

## Architectural invariants for this phase

These are the constraints most likely to be accidentally violated during UI wiring:

1. **Frontend is presentation, not inference orchestration.** Slint maps user intent into E1 operations and presents E1 state/output.
2. **E0 remains the token-step owner.** No per-token frontend command loop.
3. **E1 remains the frontend-neutral façade.** Do not leak Candle, Hugging Face implementation types, E0 unload policy, sequence state, or raw logits into the UI.
4. **Output remains bounded and pull-oriented.** Full presentation/output capacity must result in controlled backpressure, not unbounded buffering.
5. **Lifecycle truth comes from application state.** Avoid parallel UI booleans that can contradict E1 state; presentation-only state such as local text selection/scroll position is fine.
6. **Cancellation and cleanup are distinct.** A terminal generation outcome does not imply backend/request resources have already reached `Released`.
7. **Shutdown stays explicit.** Preserve the existing normal-exit call to `ApplicationRuntime::shutdown()`.
8. **No speculative abstraction.** Phase 6 needs one working Slint composition, not a generalized UI framework or transport layer.

The reusable architecture model is [Model B: Layered Workspace](../../architecture.md#model-b-layered-workspace); the concrete E0/E1/app ownership graph is in [project architecture](../../project/architecture.md).

## Explicit non-goals

Do not pull later phases into this UI slice:

- no general chat/message-history model;
- no context-planner integration into the product path;
- no guessed chat template support;
- no GGUF backend selector;
- no browser/remote transport;
- no GPU execution;
- no multi-model application residency;
- no broad crate/folder reorganization;
- no performance claims based on UI responsiveness alone.

Those have separate execution phases and should be implemented only after this product milestone is proven.

## Product acceptance scenario

Phase 6 is complete when a user can, through the Slint application alone:

1. resolve a supported immutable model revision;
2. load the model on the supported CPU/Candle path;
3. enter a direct-completion prompt;
4. start generation;
5. observe decoded text arrive incrementally;
6. see prompt/generated usage and terminal state;
7. cancel an active request and observe safe completion/cleanup state;
8. start another request when cleanup permits it;
9. unload the model;
10. close the application without orphaning the normal runtime worker.

This is the first complete user-facing product loop. Chat semantics are not part of the acceptance scenario.

## Likely implementation touchpoints

Primary:

```text
crates/apps/desktop-slint/ui/app-window.slint
crates/apps/desktop-slint/src/presenter.rs
crates/apps/desktop-slint/src/lib.rs
```

Reference rather than bypass:

```text
crates/engines/application-runtime/src/generation.rs
crates/engines/application-runtime/src/state.rs
docs/project/application-runtime.md
docs/project/desktop-runtime.md
```

If Phase 6 appears to require changing E0 scheduling or backend execution, first verify that the requirement is not already expressible through the E1 API. A frontend-driven workaround is not an acceptable substitute for fixing a genuine lower-layer contract gap.

## Documentation and closure

During Phase 6, update the document that owns each changed fact:

- frontend behavior and presentation boundary → [desktop runtime](../../project/desktop-runtime.md);
- E1 behavior, only if its contract actually changes → [application runtime](../../project/application-runtime.md);
- product support and validation state → [implementation status](../../project/implementation-status.md);
- architecture/decision changes → [project architecture](../../project/architecture.md) and an ADR when appropriate;
- repeatable validation procedure → [project validation](../../project/validation.md).

When Phase 6 closes:

1. run the canonical gate on the exact final tree;
2. record the exact commit/provenance;
3. append the closure evidence to [execution history](history.md);
4. advance this file to the next active phase instead of creating a standalone `PHASE6_IMPLEMENTATION_REPORT.md`.
