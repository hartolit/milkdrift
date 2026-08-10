# Inference runtime

`crates/runtime/inference-runtime` is E0: the backend-independent, single-owner
local inference registry and generation scheduler. It is generic over one concrete
`ModelLoader` and sits below the optional E1 reference application-services kit and
any future workflow/control plane.

E0 owns native model and sequence lifecycles, exact prepared-load transactions,
generation workspaces, scheduling, cancellation boundaries, cleanup quarantine,
accounting, unload, and terminal shutdown. It does not own artifact download,
tokenizers, conversations, workflow definitions, provider/peer transports,
presentation, or frontend labels.

## Ownership and accounting

```text
InferenceRuntime<L>
├── committed model slots with exact final ownership
├── pending model cleanup
│   ├── failed preparations with exact accepted loading peak
│   ├── verified models with exact final footprint
│   └── incompatible complete models with unverified evidence
├── active and cleanup-retained sequences
├── exact aggregate reservation
├── separate unverified ownership + admission lock
└── generation workspaces retained through output release
```

Native owners are never exposed through public `Arc<Model>` ownership. Clients
hold typed identities and generation-safe handles only. Failed preparations and
incompatible complete models are not normal model slots, and retained model owners
are not simultaneously reported as ordinary loaded models.

`RuntimeSnapshot::reserved_footprint` contains only verified exact ownership.
`unverified_ownership` and `admission_blocked` separately report complete models
whose exact upper bound is not established. A zero exact aggregate therefore does
not prove that an unverified owner was released.

## Exact prepared-load transaction

Model admission is one staged transaction:

```text
1. preflight model identity, generation, count, and remaining budget
2. loader.prepare_load(source, exact LoadConfiguration)
3. copy one stable LoadPlan from the opaque preparation
4. validate generic plan invariants
5. admit both R₀ + loading peak and R₀ + final footprint
6. reserve the loading peak
7. loader.load_prepared(the same preparation)
8. verify the complete loaded result
9. commit the model and replace peak reservation with final ownership
10. publish LoadReceipt
```

The preparation is bound to source, handle, requested `ExecutionDevice`, and
remaining `MemoryBudget`. `load_prepared` consumes that same value; it cannot ask
E0 to replan or switch artifacts.

The named generic checks are split by responsibility:

- `validate_load_plan` requires exact `accepted_configuration`, checked host/device
  totals, and component-wise loading-peak containment of the final footprint;
- descriptor validation requires a nonempty observed `ScalarTypeSet`, nonzero and
  coherent limits, and coherent portable capabilities;
- `admit_footprint` uses checked component arithmetic for both loading and final
  aggregate reservations against `RuntimeLimits::memory_budget`; and
- complete-result verification checks handle/generation, descriptor, requested
  versus actual device, planned versus actual execution scalar, final planned
  versus reported footprint, and load lifecycle completion.

These are generic contract checks, not a second Candle loader. E0 does not infer a
required primary scalar, duplicate a Safetensors conversion matrix, or impose
CPU/CUDA tensor placement. A nonempty observed set may include `F32`, `F16`,
`BF16`, `I8`, `U8`, or `Other`; the concrete adapter decides which tensors are
required and which types can execute. E1 likewise treats nonemptiness as its
generic observed-evidence condition and does not reject truthful unused extras.

`MemoryFootprint` has four concrete ownership components: host/device weights and
host/device working memory. `sequence_cache_bytes_per_token` is a planning rate,
not a fifth current-ownership component.

Let `R₀` be existing exact reservation, `P` the loading peak, `F` the final
footprint, and `B` the aggregate budget. E0 checks and admits both:

```text
R₀ + P <= B
R₀ + F <= B
```

It sets aggregate exact reservation to `R₀ + P` before materialization. On commit,
the reservation becomes `R₀ + F`; the receipt's execution scalar/device and final
footprint are verified actual-result facts. Candle owns the exact CPU/CUDA loading
and final formulas.

## Failed-load cleanup and full retry state

`load_prepared` returns either a complete model or
`FailedLoad<L::Prepared>`. The failure separates the primary materialization error
from the cleanup owner. E0 immediately calls explicit cleanup while the accepted
loading peak remains reserved.

If cleanup succeeds, E0 restores `R₀`, publishes no model or receipt, and returns
the original `RuntimeError::Load(primary)`.

If cleanup fails, E0 retains the preparation, loading-peak reservation, and
model-generation identity. Crucially,
`RuntimeError::CleanupFailed(CleanupRetryState)` carries the **full** retry state;
it is not reduced to one cleanup error or boolean:

```text
CleanupRetryState
├── resource: CleanupResource
├── failure: CleanupFailureReport
│   ├── primary operation/class/detail
│   └── cleanup operation/class/detail
├── ownership: RetainedOwnership
├── attempts
└── maximum_attempts
```

`CleanupRetryExhausted` carries the same complete `CleanupRetryState` when the
bounded lower policy is exhausted. The initial cleanup failure counts as attempt
one. `poll_cleanup` performs at most one retryable retained operation per call and
rotates fairly across owner classes and identities.

`CleanupResource` distinguishes verified model, incompatible model, failed load,
and sequence identity. `RetainedOwnership` distinguishes:

- `Released`, produced only by explicit successful cleanup;
- `Exact(footprint)` for a trustworthy named ownership phase; and
- `Unverified { accepted_loading_peak, reported_footprint,
  conservative_footprint }` for a complete contract-violating model.

`CleanupFailureReport` preserves primary and cleanup operations, classes, and
bounded details independently. Cleanup failure never replaces the primary outcome.

## Incompatible complete models

After materialization, E0 verifies the complete model in order:

1. handle and generation;
2. complete descriptor;
3. requested versus actual execution device;
4. planned versus actual execution scalar;
5. final planned versus adapter-reported footprint; and
6. lifecycle completion.

A contradiction prevents normal publication and triggers explicit unload. If that
unload fails, the complete model becomes
`RetainedOwnership::Unverified`. The accepted loading peak and contradictory report
remain separate; a checked component-wise maximum is exposed as
`ConservativeFootprint::Known`, or `Overflow` when it cannot be represented. E0
does not promote either contradictory value to exact ownership.

Unverified ownership is excluded from exact aggregate reservation and blocks all
new model, sequence, cache, and generation-workspace admission. Already admitted
healthy work may continue. Explicit cleanup success records `Released`, removes the
owner exactly once, and unlocks admission.

A missing owner in a bounded snapshot, zero exact aggregate bytes, endpoint
disconnect, or hosted worker-handle absence is not release evidence for a retained
resource. Consumers must correlate `CleanupResource` with an explicit released
`CleanupRetryState` or a successful correlated unload/shutdown result.

## Sequence and generation admission

`RuntimeCommand::Generate` carries token-level facts only: request/sequence
identity, prompt tokens, capacities and limits, sampling/seed, EOS and stop tokens,
scheduler quantum, and required bounded output capacity. It carries no tokenizer,
text, path, display, workflow, frontend, or provider DTO.

Before sequence creation E0 checks model/identity/lifecycle, context and prefill
limits, sampling, output capacity, and memory. It reserves the backend sequence
plan plus caller-owned logits, sampling, token-history, stop, and terminal-state
workspaces. Workspace bytes remain reserved until the `Released` output record is
published and scheduler storage is dropped.

A scheduled request progresses through:

```text
admitted -> prefill -> token publication -> decode
         -> terminal publication -> optional cleanup -> released
```

Each worker loop checks bounded control work, advances at most one request
opportunity, performs one cleanup-maintenance opportunity, and flushes bounded
events. A request blocked by full output performs no backend step. Generation
terminal outcome and backend resource release remain separate observable facts.

Output records preserve request identity and ordered state:

- `Yielded(OutputBackpressure)`;
- `Terminal(original outcome)`;
- `CleanupPending { original outcome, failure report, retry state }`;
- `CleanupExhausted { original outcome, failure report, retry state }`; and
- `Released(original outcome)`.

These are E0 token-output records, not the older E1 model-cleanup event shape. E1's
current model event is `ModelCleanupPending { cleanup: ApplicationRetainedModel }`.

## Cancellation, unload, and shutdown

Cancellation is observed before the next backend operation. EOS, limits, stop
patterns, cancellation, unload, and drain timeout all enter explicit sequence
cleanup. Candle sequence destruction and model unload synchronize the selected
device before reporting release; synchronization failure enters retained cleanup.

E0 shutdown consumes the finite remaining cleanup budget, releases scheduler
workspaces without waiting for frontend text draining, publishes one shutdown
result, and stops its worker loop. If native owners remain, it returns
`RuntimeError::TerminalCleanupRetention { first, summary }`. The hosted layer then
uses the named process-lifetime retention disposition rather than invoking
unverified backend destruction.

Worker joining belongs to the host and is independent from cleanup truth. A worker
may exit while ownership is deliberately retained until process exit, and a join
handle may disappear without proving release. Conversely, a clean correlated E0
shutdown result is explicit cleanup evidence even though the host must still join
its thread separately.

## Production adapter boundary and status

Production E0 is instantiated with `CandleLlamaSource`; deterministic loaders can
exercise generic lifecycle and failure contracts without adding another product
engine. External Hub resolution remains above E0. Hosted providers and peers need a
coarser execution boundary rather than pretending remote text generation has local
native ownership semantics.

The package's dedicated harness-free `cuda_hardware` target owns the complete hosted-E0 mixed-fixture generation, accounting, release, unload, and shutdown hardware boundary. The deterministic `fault_injection` target remains separate and runs in full under the CUDA feature graph; neither suite is selected through a workflow list of function names.

This guide makes no current-tree validation, external-checkpoint, or hardware support claim. The canonical evidence and support matrix remains in [implementation status](implementation-status.md).