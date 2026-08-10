# Application runtime

## Responsibility

`application-runtime` is E1: the frontend-neutral coordinator for the current reference application kit. It owns application semantics shared by native hosts while delegating token-level inference and independently stateful capabilities to their real owners.

Milkdrift remains workflow-first. E1 is not the future general workflow runtime, workspace, plugin API, provider/peer target boundary, or control plane. It is the current concrete local vertical slice used to prove application behavior above the lifecycle-safe E0 kernel.

E1 currently owns:

- persisted application preferences and model catalogue state;
- normalized Hugging Face repository/revision selection;
- application-owned CPU/CUDA selection, bounded discovery, availability, and accelerator-memory policy;
- one bounded synchronous Hub resolver worker;
- resolved artifact identity, identity-bearing weight shards, tokenizer validation, vocabulary compatibility, and complete-selection checks;
- one process-hosted monomorphized Candle E0 worker/thread behind private composition;
- one resident-model application lifecycle;
- load, reject/cancel/drain unload, and retained model-cleanup coordination;
- direct-completion encoding and request-local Hugging Face streaming decode;
- frontend-neutral conversation, exact TinyLlama chat, context planning, generation settings, text output, events, and shutdown.

It does not own Slint types, Safetensors parsing, tensor dtypes/names/offsets, the required primary scalar, per-tensor conversion, model tensors/logits/backend sequences, per-token scheduling, corrective-workflow execution, provider/peer transports, or OS-specific data-path policy.

## Selection, resolution, and scalar facts

`ModelSelection` contains only:

```text
repository
revision
```

Execution-device selection is separate E1 state using `ApplicationDevice::{Cpu, Cuda { ordinal: u32 }}`. CPU always exists and is the fresh-install default. Initial bounded discovery probes CUDA 0 and, when different, a persisted selected CUDA ordinal. Structured failure leaves unavailable persisted CUDA visible and selected. `select_device` follows `can_select_device`; load re-probes the exact selection and never falls back to CPU.

`resolve_model` sends the complete selection to the bounded Hub worker. `hf-hub-adapter` pins it to an immutable Hub commit and resolves the matching configuration, tokenizer, index when present, and ordered Safetensors shards. Recognized `dtype` or legacy `torch_dtype` becomes an optional **configuration-declared scalar**. It is producer-intent metadata, not verified tensor homogeneity.

### `ResolvedModel`

Resolution is device-independent. Public `ResolvedModel` exposes:

- normalized selection and immutable identity;
- Candle/Hub/Safetensors application categories;
- tokenizer vocabulary and exact chat compatibility;
- optional configuration-declared scalar metadata.

It exposes no complete observed tensor scalar set, required primary, execution scalar, selected/actual device, or per-tensor details. Absence of a recognized declaration no longer makes an otherwise complete resolution unloadable.

### `LoadedModel`

Public E1 `LoadedModel` exposes:

- generation-safe handle, selection, and immutable identity;
- actual execution device verified through E0's receipt;
- actual execution scalar verified through E0's receipt;
- vocabulary, context, and prefill limits.

Scalar-wise, it exposes execution only. It deliberately does not repeat the configuration declaration and does not expose the complete observed tensor set or required primary. Those are artifact/preparation facts carried in lower descriptors for verification, not required application state.

Successful unload clears loaded execution scalar/device and model limits while preserving the separate resolution and selected device.

## E1 does not choose per-tensor conversion

E1 constructs `CandleLlamaSource` from the resolved configuration path and identity-bearing weight shards, mapping Hub LFS proof to verified-immutable authority and project-established hashes to the mutable-source authority. It never injects a declaration; Candle derives it from bounded config bytes. Candle owns complete Safetensors inspection, required-set/primary policy, selective required-tensor casts/transfers, execution selection, and final/loading-peak calculation. E0 owns exact preparation admission and loaded-result verification.

E1 does not:

- infer a required primary from the complete observed set;
- compare the declaration with every tensor dtype;
- compare the declaration with execution scalar;
- select F32/F16/BF16 conversion by device;
- duplicate Candle's exact required `{F32}`, `{F16}`, `{F16,F32}`, `{BF16}`, `{BF16,F32}` matrix;
- infer execution scalar from CPU/CUDA selection;
- repair a lower mismatch by falling back.

This prevents application policy from becoming a second model loader.

## Load admission and receipt validation

`load_model` requires idle state, no resident model, an available inference worker, an exact current resolution/selection match, and a currently available selected device. It re-probes that device, builds the exact source, submits one E0 command, and retains a `LoadAdmission` containing:

- command ticket;
- optional configuration declaration;
- selected E1 device and exact domain execution device;
- startup-fixed E0 memory budget.

A successful E0 receipt is not automatically a public model. Before publication E1 validates:

- pending ticket and logical model ID/handle;
- immutable selection, repository, revision, artifact set, and commit identity;
- optional configuration declaration agreement across pending admission, resolved model, Hub artifacts, and E0 descriptor;
- a nonempty complete observed `ScalarTypeSet`; unused integer or `Other` categories do not become E1 rejection policy;
- representable receipt execution scalar and actual device;
- selected versus current selected versus actual device and exact requested domain device;
- unchanged application/E0 memory budget;
- checked final reserved footprint within that budget and in the expected CPU/CUDA memory domain;
- Candle backend, Llama architecture, unquantized Safetensors category, required capabilities, ordered nonzero limits, and tokenizer vocabulary.

E0 has already verified the complete descriptor, planned/actual execution scalar/device, and final reported footprint against the exact prepared load. E1 relies on that ownership boundary rather than trying to reconstruct the loading peak or per-tensor algorithm from a receipt. A lower incompatible complete model is not exposed as an exact receipt: E0 retains explicit unverified ownership and blocks admission until cleanup succeeds.

Only after all checks pass does E1 construct public `LoadedModel`. Any mismatch publishes no resident model and enters the existing private incompatible-receipt unload/retention path.

## Public boundary

Frontends construct `ApplicationRuntimeConfiguration`, start `ApplicationRuntime`, inspect `ApplicationState`, submit coarse operations, pull bounded output, and explicitly shut down:

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

`unload_model()` remains the bounded-drain convenience path. `ModelUnloadBehavior::{RejectIfBusy, CancelActive, Drain}` does not expose E0 policy types.

Immediate validation/submission failures return `ApplicationError`. Worker outcomes arrive as stable `ApplicationEvent` values. High-frequency decoded text remains in borrowed bounded pulls. Candle, Safetensors, filesystem, and driver failures are normalized into application-owned categories such as unsupported artifact/layout, memory admission, model load, retained cleanup, and incompatible receipt.

## Model cleanup events and retained ownership

Phase 12 makes model cleanup truth explicit:

- `ModelLoadFailed` means no public model became available and E1 has not been told that lower ownership remains retained.
- `ModelCleanupPending { exhausted: false, failure }` means a failed load, unload, or incompatible receipt still has lower-owned resources under retry/verification.
- `ModelCleanupPending { exhausted: true, failure }` means lower cleanup or E1's bounded verification/submission policy is exhausted; ownership is not reported as released.
- `ModelCompatibilityFailed` reports a successful lower receipt that E1 rejected and for which deterministic unload was requested.
- `ModelUnloaded` reports successful unload or confirmed absence.

For an E0 load error carrying retained cleanup, E1 enters `ApplicationActivity::Unloading`, emits `ModelCleanupPending`, and locks device selection. Retryable cleanup triggers bounded private snapshot inspection. E1 returns to idle only when the snapshot proves all of the following are zero/empty:

- loaded models and active requests;
- generation workspaces and their reservation;
- pending and exhausted model/sequence cleanup;
- aggregate reserved footprint;
- maintenance error.

A snapshot showing only non-exhausted retained cleanup schedules another bounded inspection. Inspection submission has a finite three-attempt policy. Inspection failure, incompatible/nonzero ownership facts, lower exhaustion, or submission exhaustion remains explicit and locked.

A post-receipt compatibility failure retains the exact `ModelHandle`, compatibility diagnostic, unload ticket/submission state, and later lower cleanup state. Automatic unload submission is bounded to three attempts. Submission or E0 cleanup exhaustion never discards the private record. Confirmed model absence, inference disconnection, or confirmed worker stop is required before release can be inferred.

Generation completion remains separate from sequence cleanup exactly as before: E1 preserves terminal, cleanup-pending/exhausted, and released states and blocks conversation clearing until release.

## Private concrete composition

```text
ApplicationRuntime
├── LocalInference
│   ├── HostedRuntime<CandleLlamaSource>
│   └── one inference RuntimeThread
├── one bounded Hub worker/thread
├── one resolved HfTokenizer
├── one HfOwnedStreamingDecoder per request
├── one resident-model/application lifecycle
└── redb application persistence
```

`local.rs` maps `ApplicationDevice` to the exact domain `ExecutionDevice`. It contains no active-backend switch, dormant worker, dynamic token-path dispatch, generic GPU alias, or fallback branch. Static Candle execution and the non-generic façade remain governed by [ADR-0013](../agent/decisions/0013-candle-only-local-execution.md).

Startup remains transactional across worker creation. If Hub startup fails after inference starts, E1 attempts bounded E0 shutdown/join before returning the primary Hub failure. Timeout retains the complete `LocalInference` owner in private startup cleanup for later bounded reap rather than detaching it.

## Accelerator memory policy

`AcceleratorMemoryPolicy` is `Automatic` or `Limit { bytes: NonZeroU64 }`. E0's aggregate budget is fixed at startup:

- `Automatic` uses the least physical total across every CUDA row in the bounded startup catalogue;
- unavailable rows or missing totals contribute zero and fail closed;
- `Limit` applies the lower of that capacity and the user cap.

Before load, E1 re-probes the selected CUDA device and requires the fixed budget to remain nonzero and no greater than the latest physical total. Incompatible capacity change requires restart and produces a structured no-fallback error. CPU host budgeting is unchanged. Candle preparation independently admits its exact Phase 12 loading peak against the remaining E0 budget and current device availability before materialization. E1 validates the successful final receipt footprint; it does not mislabel the loading peak as loaded residency.

Host RAM is not used as accelerator capacity, no undocumented `u64::MAX` device shortcut is used, and one resident model remains the product limit.

## Persistence

Application settings continue to use `LAS1`:

- new writes are version 2 with explicit selected device and accelerator-memory policy;
- exact version 1 remains readable as CPU, with zero legacy device bytes mapped to `Automatic` and nonzero bytes mapped to `Limit`;
- unavailable persisted CUDA is not migrated to CPU.

Model catalogue records now use `LAM1` version 2 for new writes. Version 2 stores optional configuration-declared scalar metadata:

```text
F32 | F16 | BF16 | None
```

Exact `LAM1` version 1 records remain readable. Their mandatory scalar code is interpreted in memory as a present declaration (`Some(F32|F16|BF16)`) without rewriting the old record. Observed tensor layouts, required primary, execution scalar/device, loading/final footprints, shard identities, and cache paths are not persisted.

## Completion, chat, and conversation

Direct completion remains available for every successfully loaded model. It encodes ordinary prompt text once and translates stable E1 generation settings into E0 contracts.

The only built-in chat profile remains:

- repository `TinyLlama/TinyLlama-1.1B-Chat-v1.0`;
- immutable commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`;
- tokenizer `</s>` mapped to EOS token ID 2;
- verified TinyLlama role rendering and EOS policy.

Broader scalar-layout compatibility does not broaden chat compatibility. Unknown repositories/commits/tokenizer evidence return `UnsupportedChatCompatibility`, while direct completion remains available.

Raw conversation and context-planning semantics are unchanged. Completed historical user/assistant turns remain atomic planning units; selected units are rendered in order, exactly tokenized, and corrected through a strictly shrinking bounded retry set. Regeneration preserves/supersedes attempts without deletion, and conversation persistence/general branch trees remain absent.

## Thin Slint host

Slint remains presentation-only. It receives `ApplicationState` and events, maps stable Rust-owned device identities to indices, and never parses labels for semantics.

- Resolved summaries may show optional configuration-declared metadata.
- Loaded summaries show only verified execution scalar and execution device.
- No tensor table, required-primary inference, conversion selector, fallback policy, or new workflow responsibility was added.

The frontend still owns only event-loop integration, callbacks, frame-batched output presentation, and platform path selection.

## Shutdown

Normal closure calls `ApplicationRuntime::shutdown`; `Drop` is not an unbounded join protocol. The private controller retains running, stopping, cleanly stopped, retryable failure, and terminal failure.

A command/event/join timeout keeps unfinished worker handles owned so a later call can retry. A terminal E0 cleanup failure remains sticky independently from join handles. E0 may terminate after publishing `TerminalCleanupRetention` with a bounded ownership summary while deliberately retaining native ownership until process exit; E1 never infers clean success from handle absence. Clean idempotent success requires observed clean E0 shutdown and confirmed worker joins. [ADR-0006](../agent/decisions/0006-explicit-bounded-shutdown.md) remains unchanged by Phase 12.

## Execution and validation boundary

CPU remains mandatory/default. `application-runtime/cuda` forwards only to `candle-backend/cuda`; no default feature graph reaches CUDA and explicit CUDA failure never falls back.

The 2026-08-10 artifact-loading amendment passed the full targeted `application-runtime` suite, exact CUDA compile graph, E1 explicit no-fallback test, and guarded CUDA fixture load/device/scalar/unload/shutdown lifecycle locally on the exact RTX 5070 Ti row. The complete canonical clean-target gate remains historical to the 2026-08-08 Phase 12 closure tree. No GitHub self-hosted run or external mixed-checkpoint claim is made. See [implementation status](implementation-status.md).
