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
│   ├── failed-materialization owners with exact peak or unverified evidence
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
`unverified_ownership` and `admission_blocked` separately report any retained
backend owner whose exact upper bound is not established, including a failed-load
owner that mutates or substitutes its accepted plan. A zero exact aggregate
therefore does not prove that an unverified owner was released.

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
`FailedLoad<L::FailedPreparation>`. The ordinary-drop-safe preparation and the
resource-bearing failed typestate are distinct associated types. The failure
separates the primary materialization error from the sole cleanup owner. E0 reads
the failed owner's accepted-plan report, calls explicit cleanup while the accepted
loading peak remains reserved, then reads the report again.

If both reports match and cleanup succeeds, E0 restores `R₀`, publishes no model
or receipt, and returns the original `RuntimeError::Load(primary)`.

If cleanup fails with matching reports, E0 retains the failed typestate, exact
loading-peak reservation, and model-generation identity. If either report differs
from the accepted plan, E0 records a backend-contract failure, removes the now
unproven quantity from exact aggregate accounting, preserves monotonic conservative
evidence, and blocks every new admission until explicit release. Crucially,
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
- `Unverified { accepted_footprint, reported_footprint,
  conservative_footprint }` for any retained owner that contradicts its accepted
  contract, including a complete model or failed-materialization owner.

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

Cold admission is represented by one composed transaction. A
`GenerationAdmissionTransaction` owns the validated request, sampler, every
preallocated caller workspace, the not-yet-visible scheduled task, and a nested
`SequenceAdmissionTransaction`. The nested transaction owns the sole backend
sequence and its prepared lifecycle transition until commit. Validation and exact
aggregate preflight therefore happen before allocation and native creation;
runtime indexes, ownership, and accounting commit before scheduler visibility.
Dropping or explicitly rejecting an uncommitted transaction has one destruction
path, and a destruction failure moves the same sole sequence owner into retained
cleanup rather than losing or double-releasing it.

Before sequence creation E0 checks model/identity/lifecycle, context and prefill
limits, sampling, output capacity, and memory. It validates that the proposed
`SequenceReservation` has checked persistent/transient components and an exact
checked total, admits that complete backend total, and only then calls native
sequence creation. Caller-owned logits, sampling, token-history, stop, output,
and terminal-state workspaces are admitted independently; they are never hidden
inside the backend plan. Aggregate model + sequence + workspace capacity rejects
before registry or backend mutation.

The created sequence must report the accepted identity, token capacity, and
complete immutable `SequencePlan`. E0 repeats identity/capacity/plan checks after
successful prefill and decode and reads the plan on both sides of destruction.
The backend total remains in exact aggregate and model-slot accounting until one
successful destruction, including while terminal output awaits release and while
a conforming cleanup owner is quarantined for retry.

If creation or a later operation contradicts identity, capacity, or plan, E0
reports a backend-contract violation and explicitly destroys the unpublished or
terminal sequence. If destruction also fails, E0 retains the sole owner as
`RetainedOwnership::Unverified { accepted_footprint, reported_footprint,
conservative_footprint }`, removes any
formerly exact sequence amount once, degrades the model, and blocks new
admission. Matching numeric reports do not turn an identity/capacity
contradiction into exact physical ownership. Successful destruction is still
release evidence and removes the owner exactly once.

Workspace bytes remain reserved until the `Released` output record is published
and scheduler storage is dropped.

E0 publishes through `host-runtime`'s typed token-output wrapper. The wrapper and
E1's typed text output share one private bounded storage/concurrency core, while
their cursor, range, record, and batch types remain semantically distinct.
Capacity and consumer-busy results retain pending output for retry; a poisoned
output mutex is terminal and routes the hosted worker through bounded shutdown.

A scheduled request progresses through explicit statically dispatched phases:

```text
admitted -> prefill -> token publication -> decode
         -> terminal publication -> optional cleanup -> released
```

The private `Prefill`, `PendingToken`, `Decode`, and staged terminal-publication
values carry the data valid for each transition. One scheduler opportunity runs
at most one backend operation. A busy or full output retains the exact pending
token and performs no decode; cancellation is observed before the next phase; and
terminal, cleanup-pending, cleanup-exhausted, and released records advance only
after the preceding publication succeeds. These transition methods allocate no
storage and use no dynamic dispatch. Contract-failure evidence is tied to the
phase that observed it: a short prefill logits result records `Prefill`, while the
corresponding incremental result records `Decode`.

Each worker loop checks bounded control work, advances at most one request
opportunity, performs one cleanup-maintenance opportunity, and flushes bounded
events. Round-robin selection rotates over the requests that are active at each
selection point; a newly arriving request cannot retroactively alternate with
work completed before its admission. A request blocked by full output performs
no backend step. Generation terminal outcome and backend resource release remain
separate observable facts.

The hosted loop is one private owned `WorkerState`. It owns the scheduler,
pending and queued events, unload correlation, maintenance events, terminal stop,
clock, and poll configuration. Immediate and timeout command receipt both enter
the same command-application method, so event queueing, unload correlation, and
terminal-stop handling have one implementation. Each turn accepts at most eight
commands, publishes at most one pending event, advances one generation request,
and gives cleanup and unload maintenance one bounded opportunity. A full internal
or external event queue sleeps for the configured poll interval while preserving
maintenance polling, preventing an unbounded busy loop without introducing a
blocking producer.

Output records preserve request identity and ordered state:

- `Yielded(OutputBackpressure)`;
- `Terminal(original outcome)`;
- `CleanupPending { original outcome, failure report, retry state }`;
- `CleanupExhausted { original outcome, failure report, retry state }`; and
- `Released(original outcome)`.

These are E0 token-output records, not E1 model-cleanup events. E1 stores complete
retained evidence in `ApplicationState` and publishes the compact transition event
`ModelCleanupPending { resource, disposition }`.

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
