# application-runtime

`application-runtime` is an optional, frontend-neutral reference
application-services kit above E0. It composes the current native local-model
vertical slice, but it is not Milkdrift's sole API, workflow plane, execution
boundary, or future control plane. Other applications and workflow hosts may use
lower capability or execution boundaries directly when E1's application semantics
are not appropriate.

The crate owns application selection, device choice, immutable Hub resolution,
persistence, one-resident-model lifecycle, completion, compatible chat,
conversation state, bounded output, cancellation, unload, retained-cleanup
coordination, and explicit shutdown. It owns no Slint or other frontend toolkit
types.

## Current reference composition

The local reference composition is deliberately concrete and private:

- one `HostedRuntime<CandleLlamaSource>` and inference worker;
- one bounded synchronous Hugging Face resolver worker;
- one `HfTokenizer` and request-local `HfOwnedStreamingDecoder` values;
- immutable Hugging Face Llama Safetensors artifacts;
- mandatory/default CPU or explicit feature-gated CUDA selection; and
- redb-backed settings and model-catalogue persistence.

`ModelSelection` contains only a normalized repository and requested revision.
Device selection is separate application state using
`ApplicationDevice::{Cpu, Cuda { ordinal }}`. `ApplicationDeviceSummary` exposes
structured identity, availability, memory, and capability facts plus an optional
backend-reported `display_name`; it does not provide frontend-shaped labels.
Consumers format their own labels and never parse them back into identity.

The public configuration entry point is
`ApplicationRuntimeConfiguration::new(database_path)`. The remaining public
fields permit bounded host-specific overrides. The `cuda` feature forwards only
to `candle-backend/cuda`; the default graph remains CPU-only and explicit CUDA
failure never falls back to CPU.

## Resolution and declaration states

`hf-hub-adapter` pins repository/revision selection to an immutable commit before
resolving the supported artifacts. Its `dtype`/`torch_dtype` declaration state is
strict:

- absent or null fields produce no declaration;
- one recognized declaration, or two equal recognized declarations, produces
  `Some(F32|F16|BF16)` and may continue to load;
- unsupported, malformed, or conflicting present declarations fail during
  resolution with stable `HubErrorKind` and normalized `ApplicationFailure`
  categories; and
- raw vendor declaration values and vendor error details do not cross the E1
  failure boundary.

Public `ResolvedModel` exposes the normalized selection, immutable
repository/commit identity, validated tokenizer vocabulary size, an optional
recognized configuration declaration, and the unit
`ChatCompatibility::{Supported, Unsupported}` result. Backend engine, artifact
source, model format, private prompt profile, cache paths, and source-construction
helper types are not part of `ResolvedModel`.

The optional declaration is producer intent. It is neither tensor-homogeneity
proof nor the execution scalar.

## Load transaction

Loading proceeds as one correlated transaction:

1. E1 verifies selection, immutable identity, and tokenizer vocabulary against its
   private artifact/tokenizer snapshot without rechecking copied declaration data.
2. E1 re-probes the selected device, constructs the private Candle source, submits
   one ticketed E0 load, and retains `ModelLoadTransaction` with the resolution
   snapshot and `LoadAdmission`.
3. E0 prepares, validates, reserves, materializes, verifies, and either commits a
   model or reports retained cleanup.
4. E1 applies the named generic receipt checks before publishing `LoadedModel`.

The transaction mismatch classes are `Ticket`, `ModelIdentity`, `Declaration`,
`ExecutionScalar`, `ExecutionDevice`, `SelectedDevice`, `MemoryBudget`,
`FinalFootprint`, `ObservedEvidence`, `Capabilities`, `Composition`, `Limits`, and
`TokenizerVocabulary`, plus `MissingTransaction` for an uncorrelated receipt.
Declaration consistency is checked between the retained resolution and the E0
descriptor after private snapshot consistency was established; it is not a
four-copy application policy.

E1 requires a nonempty complete observed `ScalarTypeSet`, but does not reject a
truthful set merely because it contains unused `F16`, `BF16`, `I8`, `U8`, or
`Other` entries. Candle owns required-tensor classification, scalar conversion,
and CPU/CUDA materialization policy. E1 does not duplicate Candle's dtype matrix.

For the final footprint, E1 performs checked host and device totals and verifies
those totals against the startup-fixed budget. It has no CPU-versus-CUDA component
placement policy and does not reconstruct Candle's loading peak. Public
`LoadedModel` receives its actual execution scalar and actual execution device only
from the verified E0 receipt.

No normal `LoadedModel` is published until every check succeeds.

## Retained ownership and cleanup

`ApplicationState::retained_model()` exposes one complete
`ApplicationRetainedModel` when release remains unresolved. It preserves:

- `ApplicationRetainedModelResource`, including failed load, loaded model,
  incompatible model, and unconfirmed load/model cases;
- `ApplicationRetainedOwnership::{Exact, Unverified, Unknown}`;
- `ApplicationModelCleanupDisposition::{Pending, LowerRetryable,
  LowerExhausted, CoordinationRetryAvailable, WorkerDisconnected,
  RetainedUntilProcessExit}`; and
- separate primary and cleanup/coordination `ApplicationFailure` values.

Entering retained state clears the normal loaded model and locks selection,
resolution, and loading. `ModelCleanupPending { cleanup }` carries the complete
public retained state. An explicit lower model-owner release outside an in-flight
unload emits `ModelCleanupReleased { resource }`. Sequence or model cleanup released
while an unload remains correlated stays pending until the terminal `ModelUnloaded`
receipt, and successor cleanup owners for the same model are adopted without losing
correlation.

Worker disconnect, worker-handle absence, an omitted owner in a snapshot, or zero
exact aggregate reservation is **not** release proof. Unverified and unknown owners
are intentionally not represented as ordinary exact aggregate bytes. Retained
state therefore remains visible after disconnect and may end as
`RetainedUntilProcessExit`.

`ApplicationRuntime::retry_model_cleanup()` is public. It re-enables bounded E1
submission/inspection only for `CoordinationRetryAvailable`; it does not reset
lower cleanup exhaustion and cannot retry worker disconnection or process-lifetime
retention.

## Persistence

Application settings use `LAS1` version 2 and retain exact version-1 reads.

Model catalogue writes use latest `LAM1` version 3. The declaration encoding is:

- presence tag `0`: absent;
- presence tag `1`, followed by scalar code `0`, `1`, or `2`: `F32`, `F16`, or
  `BF16`.

Exact `LAM1` versions 1 and 2 remain readable; reads do not rewrite old records.
All explicit writes use version 3. Key/embedded-name mismatch, wrong record kind,
unknown version/tag/code, truncation, invalid UTF-8, trailing bytes, and invalid
fields remain explicit errors. The timestamp field is
`last_resolved_unix_milliseconds`.

Observed tensor sets, required scalar policy, execution scalar/device, runtime
footprints, shard/cache paths, active lifecycle, and retained-cleanup facts are not
persisted.

## Completion, chat, and output

Direct completion is available for every successfully loaded model. Built-in chat
remains limited to immutable commit
`fe8a4ea1ffedaf415f4da2f062534de366a451e6` of
`TinyLlama/TinyLlama-1.1B-Chat-v1.0` with the verified tokenizer/EOS contract.
Unknown chat compatibility fails explicitly while direct completion remains
available.

Conversation planning keeps completed user/assistant turns atomic, renders them in
order, exactly tokenizes them, and uses a strictly shrinking bounded correction
set before E0 admission. High-frequency decoded text crosses E1 through bounded
borrowed pulls rather than one application event per token.

## Shutdown

Normal hosts must call `ApplicationRuntime::shutdown`; `Drop` does not perform an
unbounded join. Hub shutdown, E0 shutdown outcome, and both worker joins are
attempted independently so one failure does not skip the others. Retryable timeout
keeps unfinished handles owned for a later call.

Cleanup evidence and thread joining are separate facts. A correlated clean E0
shutdown can establish release; merely disconnecting or observing no join handle
cannot. Terminal E0 cleanup retention remains sticky even after the worker exits
and is represented as `RetainedUntilProcessExit`.