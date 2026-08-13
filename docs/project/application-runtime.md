# Application runtime

## Responsibility

`application-runtime` is the optional E1 reference application-services kit above
E0. It supplies reusable native application semantics for the current local-model
vertical slice. It is not Milkdrift's only API, workflow runtime, workspace,
provider/peer boundary, plugin plane, or future control plane.

A host chooses E1 when it wants this concrete package of behavior:

- normalized Hugging Face repository/revision selection;
- selected CPU/CUDA identity, bounded discovery, and persisted preferences;
- immutable artifact resolution on one bounded Hub worker;
- one private Candle E0 worker and one resident-model lifecycle;
- completion, exact compatible chat, conversation/context planning, cancellation,
  unload, retained cleanup, events, and bounded output; and
- explicit shutdown of the Hub and inference workers.

E1 does not own model tensor policy, backend sequences, per-token scheduling,
corrective-workflow state, provider/peer transports, Slint types, or OS data-path
selection.

## Public construction and state

Hosts construct the configuration with:

```rust
let configuration = ApplicationRuntimeConfiguration::new(database_path);
let mut runtime = ApplicationRuntime::start(configuration)?;
```

The constructor installs bounded frontend-neutral defaults; public configuration
fields permit deliberate overrides. Stable coarse operations include:

```text
select_device(device)
resolve_model(selection)
load_model(&selection)
start_generation(input, settings)
submit_user_message(content, settings)
regenerate_last_response(settings)
cancel_generation(request_id)
unload_model_with_behavior(behavior)
retry_model_cleanup()
poll_event()
pull_output(callback)
shutdown()
```

`ApplicationState` exposes lifecycle predicates and facts. It does not expose
Candle, Safetensors, `hf-hub`, redb, Flume, or frontend toolkit values.
`ApplicationDeviceSummary` contains structured device identity and observations,
including an optional backend-reported `display_name`; E1 does not manufacture
frontend labels.

## Selection and resolution

`ModelSelection` contains only normalized `repository` and `revision`. Device
selection is independent state using
`ApplicationDevice::{Cpu, Cuda { ordinal: u32 }}`. CPU is mandatory/default;
feature-gated CUDA is explicit and never falls back to CPU.

The Hub adapter resolves the complete selection to an immutable commit and the
supported configuration, tokenizer, and Safetensors shards. Configuration scalar
declarations have a strict state machine:

| Configuration state | Resolution result |
| --- | --- |
| `dtype`/`torch_dtype` absent or null | No declaration; continue |
| One recognized value, or two recognized equal values | `F32`, `F16`, or `BF16`; continue |
| Present unsupported string | `HubErrorKind::UnsupportedScalarDeclaration`; fail |
| Malformed JSON, wrong/duplicate field type, or non-object | `HubErrorKind::MalformedConfiguration`; fail |
| Two recognized unequal values | `HubErrorKind::ConflictingScalarDeclarations`; fail |

E1 maps the three failure cases to stable `ApplicationFailure` values with kinds
`UnsupportedArtifactDeclaration`, `MalformedArtifactConfiguration`, and
`ConflictingArtifactDeclaration`. Messages contain no raw declaration value or
vendor detail.

### `ResolvedModel`

Resolution is device-independent. Public `ResolvedModel` exposes:

- normalized selection;
- immutable repository/commit identity;
- validated tokenizer vocabulary size;
- recognized-or-absent configuration declaration; and
- unit `ChatCompatibility::{Supported, Unsupported}`.

It does not expose engine, source, format, private artifact/source helper types,
cache paths, prompt-profile internals, observed tensor sets, required scalar policy,
execution scalar, or execution device. The declaration is producer intent, not
tensor-homogeneity evidence. Absence therefore does not authorize E1 to infer a
mixed-layout primary; the selected local adapter rejects ambiguous mixed required
sets below E1.

### `LoadedModel`

A normal loaded model exposes generation-safe identity, selection/immutable
identity, vocabulary and limits, generation mode, and the **actual** execution
scalar and device from E0's verified load receipt. It does not copy the
configuration declaration or expose observed/required tensor policy. Successful
unload clears loaded execution facts while preserving independent selection and
resolution.

Private publication uses one validated load commit containing receipt-derived
execution facts, limits, identity, mode, and the canonical
`domain_contracts::MemoryFootprint`. The application boundary does not maintain a
field-for-field footprint DTO or accept those verified facts as swappable scalar
constructor arguments.

## Correlated load transaction

`load_model` is staged rather than treating a lower success as automatic
publication:

1. **Resolution snapshot** — require idle state, no loaded or retained model, exact
   visible selection match, private immutable artifacts, and tokenizer consistency.
2. **Admission snapshot** — re-probe the selected device, construct the private
   `CandleLlamaSource`, submit one ticketed E0 command, and retain
   `ModelLoadTransaction { ticket, resolved, admission }`.
3. **E0 prepared load** — E0 prepares once, applies generic plan/admission checks,
   reserves the loading peak, consumes the ordinary-drop-safe preparation, verifies
   the complete result, and either commits or retains the distinct failed-load or
   complete-model cleanup owner.
4. **E1 receipt validation** — correlate and apply generic application checks before
   constructing `LoadedModel`.

The named E1 mismatch classes are:

```text
MissingTransaction
Ticket
ModelIdentity
Declaration
ExecutionScalar
ExecutionDevice
SelectedDevice
MemoryBudget
FinalFootprint
ObservedEvidence
Capabilities
Composition
Limits
TokenizerVocabulary
```

Before submission, E1 confirms selection, immutable identity, and tokenizer
vocabulary against its private artifact/tokenizer snapshot; it does not recheck a
copied declaration there. At receipt time, it compares the descriptor's optional
declaration once with the retained E1 resolution. It does not compare a four-copy
declaration policy across admission, resolution, Hub artifacts, and the descriptor.

`ObservedEvidence` requires only a nonempty complete `ScalarTypeSet`. E1 accepts
truthful sets containing unused `F16`, `BF16`, `I8`, `U8`, or `Other` categories;
it does not reproduce Candle's required-tensor matrix or choose conversion.

`FinalFootprint` computes checked host and device totals and verifies each total
against the unchanged startup budget. E1 does not impose a CPU/CUDA component
placement rule and does not reconstruct the preparation loading peak. Device
selection must still agree with the requested and receipt-reported actual device.
The public execution scalar/device come only from the receipt.

No normal `LoadedModel` is published until all stages pass. A lower retained-load
failure or a complete receipt rejected by E1 enters retained cleanup instead.

## Public retained ownership

Retained model state is first-class public application state:

```text
ApplicationRetainedModel
├── resource
├── ownership
├── cleanup disposition
├── primary_failure
└── cleanup_failure (optional)
```

`ApplicationRetainedModelResource` distinguishes `FailedLoad`, `LoadedModel`,
`IncompatibleModel`, `UnconfirmedLoad`, and `UnconfirmedModel`.

`ApplicationRetainedOwnership` distinguishes:

- `Exact(domain_contracts::MemoryFootprint)` for a verified named ownership phase;
- `Unverified { accepted_loading_peak, reported_footprint,
  conservative_footprint }` after a complete model contradicts its contract; and
- `Unknown` when the endpoint disappeared before ownership certainty was observed.

`ApplicationModelCleanupDisposition` distinguishes `Pending`,
`LowerRetryable`, `LowerExhausted`, `CoordinationRetryAvailable`,
`WorkerDisconnected`, and `RetainedUntilProcessExit`. Primary operation failure is
preserved separately from cleanup or coordination failure.

Detailed retained evidence lives durably in `ApplicationState::retained_model()`.
`ApplicationEvent::ModelCleanupPending { resource, disposition }` is a compact
transition notification that tells a host when to read that state; it does not copy
the primary failure, cleanup failure, or unverified footprint through every event.
An explicit lower model-owner release outside an in-flight unload emits
`ModelCleanupReleased { resource }`. A sequence cleanup released while an unload
remains correlated stays pending until `ModelUnloaded`; E1 adopts a successor
cleanup resource for the same model without losing the unload correlation.

Privately, `ApplicationRuntime` owns exactly one model-cleanup coordinator. The
coordinator records one origin (failed materialization, incompatible complete
model, ordinary unload, unconfirmed disconnect, or terminal shutdown), optional
lower resource identity, durable public evidence, lower attempts, and one checked
active action. Actions bind submitted tickets to either incompatible-model unload
or retained-owner inspection. E1 submission attempts are separate from lower
cleanup attempts; explicit E1 retry resets only the former. A lower-exhausted,
disconnected, or process-retained action has no transition back to coordination.

The following are not release evidence:

- inference endpoint disconnect;
- worker-thread or join-handle absence;
- zero exact aggregate `reserved_footprint`;
- an owner missing from a bounded snapshot; or
- E1 submission/inspection exhaustion.

Exact aggregate accounting intentionally excludes unverified and unknown
ownership. E1 therefore keeps retained state visible and never creates a normal
`LoadedModel` beside it. Retention locks selection, resolution, loading, and normal
generation admission.

`ApplicationRuntime::retry_model_cleanup()` is public. It is enabled only for
`CoordinationRetryAvailable` and starts another bounded E1 coordination round. It
does not reset `LowerExhausted` and cannot turn `WorkerDisconnected` or
`RetainedUntilProcessExit` into release.

## Memory boundary

`AcceleratorMemoryPolicy` is `Automatic` or
`Limit { bytes: NonZeroU64 }`. Startup resolves one aggregate E0 budget from the
bounded device catalogue and configured host policy. Load re-probes the selected
explicit device and rejects unavailable or incompatible capacity without fallback.

Candle owns exact required-range inspection, scalar conversion, loading-peak
planning, materialization, and device placement. E0 admits and verifies that plan.
E1's receipt check is deliberately generic: checked host/device totals must fit the
unchanged budget. E1 does not infer placement from CPU/CUDA identity or sampled
available memory.

## Persistence

Application settings remain `LAS1` version 2 writes with exact version-1 reads.
Version 1 maps selection to CPU and maps a zero legacy device limit to `Automatic`.
An unavailable persisted CUDA selection remains selected and visible.

Model catalogue records use latest `LAM1` version 3 writes:

```text
magic "LAM1"
version 3
name
repository
revision
configuration declaration presence:
  0                 -> absent
  1 + scalar code   -> 0 F32 | 1 F16 | 2 BF16
last_resolved_unix_milliseconds
```

Exact version 1 and version 2 records remain readable. Version 1 has a mandatory
scalar code; version 2 uses scalar code `3` for absence. Reading either old version
does not rewrite it. Every explicit upsert writes latest version 3.

Wrong record kind, unknown versions/tags/codes, truncation, invalid UTF-8, trailing
bytes, invalid fields, and redb key versus embedded `name` mismatch remain explicit
storage errors rather than migration guesses.

Runtime facts are not persisted: observed tensor inventory, required policy,
execution scalar/device, loading/final footprints, shard identity/cache paths,
normal loaded state, and retained cleanup state are reconstructed from resolution
and runtime receipts.

## Completion and compatible chat

Direct completion is available for every normal loaded model. The sole built-in
chat profile remains repository `TinyLlama/TinyLlama-1.1B-Chat-v1.0`, immutable
commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`, and tokenizer `</s>` mapped to EOS
ID 2. The public compatibility result is only the unit supported/unsupported fact;
the private prompt renderer/profile does not cross the boundary.

Completed historical user/assistant turns remain atomic context-planning units.
Chat preparation is pure and staged as a validated context inventory, planned
selection, profile-rendered prompt, exact encoded prompt, and final preparation
with diagnostics. Selected units are rendered in order, exactly tokenized, and
corrected with a strictly shrinking bounded set before E0 admission. No
conversation record or diagnostics are mutated during preparation.

Direct and chat generation share one E1 admission transaction. It checks
application state before tokenization, validates settings and prompt/stop bounds,
allocates correlated request/sequence identities, preallocates decoding and any
chat-response commit, constructs one E0 command, and submits it before publishing
the application session and generation state. Queue-full or disconnected
submission drops the provisional transaction without an active E1 attempt. A
user message committed before chat preparation remains in raw history by the
public chat contract; a provisional assistant response is not published and a
prior response is not superseded unless command submission succeeds. Decoded text
uses the typed host text-output wrapper and bounded borrowed pulls rather than
per-token application events; it cannot resolve token-output ranges by
construction.

## Thin Slint reference host

Slint constructs its own labels from structured E1 facts. Its model projection is
limited to repository/revision, optional recognized declaration, selected device,
receipt-reported actual execution scalar/device, and retained state. The unit chat
compatibility fact controls supported behavior without exposing a profile. No
engine/source/format helper, tensor table, conversion selector, or fallback policy
is projected.

The current frame loop runs every 16 ms, drains at most 64 application events, and
performs one bounded output pull.

## Shutdown

Normal hosts call `ApplicationRuntime::shutdown`; `Drop` does not perform an
unbounded join. Shutdown requests and joins for inference and Hub workers are
attempted independently, even after an earlier failure. A retryable timeout retains
unfinished handles for another call.

Cleanup outcome and worker join are separate axes. Only a correlated explicit
release, successful unload, or clean E0 shutdown result proves model release.
Disconnect or handle absence does not. Terminal E0 cleanup retention remains
sticky after worker exit and is exposed as `RetainedUntilProcessExit`.

The package's dedicated harness-free `cuda_hardware` target runs the complete E1 hardware boundary, including explicit unavailable-CUDA no-fallback behavior and the real fixture's selected/actual device, execution scalar, generation, unload, and shutdown lifecycle. Adding a registered case changes the suite without changing workflow YAML, and absent hardware opt-in is a failure rather than a successful skip.

This guide makes no current-tree validation or hardware-support claim. The canonical evidence and support matrix remains in [implementation status](implementation-status.md).
