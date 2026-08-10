# Inference runtime

`crates/runtime/inference-runtime` is E0: the backend-independent, single-owner local inference registry and generation scheduler. It is generic over one concrete `ModelLoader` and owns every prepared/loaded model transaction, backend sequence, generation workspace, lifecycle transition, cleanup quarantine, and aggregate reservation.

E0 is an execution kernel below the current application kit and future workflow control plane. It does not own artifact download, tokenizers, conversations, workflow definitions, provider/peer transports, or presentation.

## Ownership and accounting

```text
Hosted worker
├── InferenceRuntime<L>
│   ├── normal model registry
│   │   └── ModelSlot<L::Model>
│   │       ├── exclusively owned complete model
│   │       ├── verified descriptor/device/execution scalar
│   │       ├── final model reservation
│   │       ├── ModelLifecycle
│   │       ├── active request sequences
│   │       └── quarantined sequences
│   ├── pending model cleanup
│   │   ├── VerifiedModel(L::Model) with exact final ownership
│   │   ├── FailedPreparation(L::Prepared) with exact accepted loading ownership
│   │   └── IncompatibleModel(L::Model) with unverified ownership evidence
│   ├── active and pending-cleanup generation-safe identity indexes
│   ├── aggregate exact reservation
│   ├── separate unverified conservative evidence + admission lock
│   └── generation-workspace accounting retained through output release
├── rotating bounded cleanup scheduler
├── fair generation scheduler
└── nonblocking token-output producer
```

Models, prepared-load owners, and sequences are never placed in public `Arc` ownership or borrowed across the command boundary. Public clients retain typed identifiers and generation-safe handles only. Cleanup success is the only ordinary transition that authorizes owner drop and produces the explicit `Released` certainty state. Exact owners remain in exact accounting until that transition; an owner whose exact upper bound is no longer trustworthy remains explicitly unverified and admission-blocking instead of being forced into the exact aggregate.

`RuntimeSnapshot::reserved_footprint` is only the aggregate of verified exact ownership. `unverified_ownership` separately reports the number of incompatible complete-model owners and their checked conservative evidence, while `admission_blocked` makes the fail-closed policy explicit. Pending/exhausted cleanup counts, `last_cleanup`, and bounded `retained_models` records expose retry state, generation-safe `CleanupResource` identity, structured primary/cleanup details, and `RetainedOwnership::{Released, Exact, Unverified}` without leaking backend types. Normal per-model snapshots still expose only committed models and their verified descriptor, execution scalar/device, exact reservation, request counts, and degraded state. Failed preparations and incompatible complete models are never misreported as resident models.

## Exact prepared-load transaction

Model admission is one prepare, validate, reserve, materialize, verify, commit transaction:

```text
preflight identity and model-count limits
-> derive exact handle and remaining aggregate budget
-> loader.prepare_load(source, exact LoadConfiguration)
-> copy and validate that preparation's LoadPlan
-> admit both loading peak and final footprint
-> reserve loading peak
-> loader.load_prepared(the same preparation)
-> verify complete loaded result
-> commit model slot and replace peak with final reservation
-> publish LoadReceipt
```

`ModelLoader::prepare_load` returns an opaque associated `PreparedLoad`. Its public `plan()` is bound to the exact source, handle, execution device, and remaining memory budget. `load_prepared` consumes that same value and may not replan or switch artifacts. If E0 rejects the plan or aggregate peak before materialization, the unmaterialized preparation is ordinary-drop-safe and no explicit cleanup owner exists.

E0 validates a prepared plan before any backend materialization:

- `accepted_configuration` exactly equals E0's handle/device/budget configuration;
- the descriptor has a nonempty observed tensor scalar set, nonzero ordered limits, and coherent capabilities;
- checked final and loading host/device totals do not overflow;
- every loading-peak ownership component contains the corresponding final component;
- loading host/device totals are at least final totals.

`MemoryFootprint` contains only four concrete byte-ownership components: host/device weights and host/device working memory. `ModelDescriptor::sequence_cache_bytes_per_token` is a separate arithmetic planning rate used to derive an exact concrete `SequencePlan`; it is not sampled or current ownership and is never added as a fifth footprint component. E0 deliberately does not reproduce Candle's per-tensor layout or conversion matrix. The descriptor and aggregate plan are portable claims; adapter-specific Safetensors policy remains in `candle-backend`.

## E0 peak admission

Let:

- `R₀` be E0's reservation before the load;
- `P` be `LoadPlan::loading_peak_footprint`;
- `F` be `LoadPlan::final_footprint`;
- `B` be the process-wide `RuntimeLimits::memory_budget`.

E0 first gives the adapter a remaining budget formed from checked host/device totals of `R₀`, rejecting over-budget state before exact subtraction from `B`. After preparation it independently computes, component by component with checked arithmetic:

```text
loading reservation = R₀ + P
final reservation   = R₀ + F
```

Both totals must fit `B`. E0 sets the aggregate reservation to `R₀ + P` before calling `load_prepared`. This is the Phase 12 peak-admission invariant: partial materialization cannot create resources before their deterministic loading peak is represented in E0 ownership accounting.

On complete success, the committed model stores `F` and the aggregate reservation becomes `R₀ + F`. `LoadReceipt::reserved_footprint`, `ModelSnapshot::reserved_footprint`, and backend `reported_footprint()` then all refer to the verified final deterministic ownership phase, not the loading peak or sampled physical memory.

The complete per-tensor CPU/CUDA formulas that produce `P` and `F` are owned by [Candle backend](candle-backend.md). E0 validates their generic phase relationship but does not derive them.

## `FailedLoad<PreparedLoad>` cleanup

`load_prepared` returns either a complete model or `FailedLoad<L::Prepared>`. The failed value separates:

- `primary`: why exact materialization failed;
- `cleanup_owner`: the sole owner of completed/pending tensors, retained device work, open shards, and any partially constructed backend model.

E0 immediately calls `cleanup_owner.cleanup()` while the loading peak remains reserved.

### Immediate cleanup succeeds

E0 restores `R₀`, publishes no receipt or model slot, and returns the original `RuntimeError::Load(primary)`. The cleanup result does not replace or obscure the primary failure.

### Cleanup fails

E0 moves the preparation into `pending_models` as `PendingModelOwner::FailedPreparation`, retains the full loading-peak footprint `P`, reserves the model identity/generation, and returns structured retained-cleanup state:

```text
primary operation: ModelLoad
primary class:     Load
cleanup operation: FailedLoadCleanup
cleanup class:     the synchronization/cleanup failure class
resource:          FailedLoad { handle }
ownership:         Exact(accepted loading peak)
attempts:          1
```

The initial failed cleanup is attempt one. `poll_cleanup` performs at most one additional non-exhausted retained operation per call. A rotating class cursor gives pending sequences, failed preparations, and complete models fair opportunities; per-class cursors rotate among owners instead of repeatedly selecting the first ordered-map entry. The total-attempt limit is configurable and defaults to three. A retry failure returns `RetryFailed` or `Exhausted`; later automatic maintenance skips exhausted ownership. Success removes the owner, generation-safe identity, capacity, and exact reservation once. A second load of the same model ID is blocked while cleanup remains retained.

## Incompatible complete-model ownership

A complete native model that contradicts any accepted post-load claim—handle, descriptor, device, execution scalar, or final reported footprint—is not another exact loading-phase owner. The backend has crossed the contract boundary, so the accepted loading peak is no longer proof of a physical upper bound even when the reported footprint is smaller or otherwise plausible. E0 first attempts explicit unload. If unload succeeds, the incompatible model is released without publication. If unload fails, E0 retains `PendingModelOwner::IncompatibleModel` with:

- the accepted loading peak;
- the complete model's reported footprint;
- the checked component-wise maximum of every trustworthy accepted/reported byte component, represented as `ConservativeFootprint::Known`;
- `ConservativeFootprint::Overflow` when a component sum or host/device total cannot be represented, without saturation or a synthetic maximum.

That state is `RetainedOwnership::Unverified`, never `Exact`. It is excluded from `RuntimeSnapshot::reserved_footprint`, exposed through `unverified_ownership` and retained-model snapshots, and blocks every new model, sequence, cache, and generation-workspace admission while it exists. Already admitted healthy model work may continue. Successful cleanup removes the unverified owner exactly once, publishes `RetainedOwnership::Released` in the completed retry state, and unlocks admission; exhaustion remains observable and admission-blocking until process reclamation.

Cleanup exhaustion remains owned. During terminal shutdown, E0 consumes the finite remaining fair cleanup budget and returns `RuntimeError::TerminalCleanupRetention { first, summary }` when native ownership survives. The summary distinguishes failed preparations, verified models, incompatible models, retained sequences, and aggregate unverified conservative evidence. The hosted worker then applies the named `RetainUntilProcessExit` disposition from [ADR-0006](../agent/decisions/0006-explicit-bounded-shutdown.md): it retains the complete runtime allocation until process exit after publishing the terminal result. Worker-handle absence is never proof of release.

## Backend contract verification

Rust trait conformance is necessary but not sufficient. After `load_prepared` returns a complete model, E0 reads and verifies in order:

1. model handle/generation;
2. complete descriptor, including optional configuration declaration and observed scalar set;
3. requested versus actual `ExecutionDevice`;
4. planned versus actual execution scalar;
5. final planned versus adapter-reported footprint;
6. load lifecycle completion.

No normal model slot or receipt is published before every check succeeds. The inspected declaration or observed set is never substituted for the independently planned execution scalar. A mismatch uses explicit unload and, on cleanup failure, unverified retained ownership rather than selecting another scalar, falling back to CPU, or pretending the accepted peak remains exact. E0 can verify portable claim consistency but cannot generically prove physical allocation, placement, backend-hidden aliases, or that an already contract-violating implementation reported every owner; that is the trust boundary which requires fail-closed admission. [ADR-0010](../agent/decisions/0010-verify-backend-contracts-at-e0.md) remains the governing substitution rule; [ADR-0020](../agent/decisions/0020-transactional-prepared-model-loading.md) extends it to exact preparation and retained-ownership certainty.

Sequence plans and operation results remain claims as well. A successful prefill or decode result is accepted only when it:

- uses an advertised operation;
- preserves admitted sequence identity and fixed token capacity;
- leaves the sequence in `Ready` state;
- advances the exact expected position;
- reports the exact consumed prompt count where applicable;
- writes exactly the model vocabulary's logits when requested.

A contradiction becomes `BackendContractViolation` before sampling and enters ordinary explicit sequence destruction/quarantine.

## Sequence and generation admission

`RuntimeCommand::Generate` carries token-level execution facts only:

- request/sequence identity;
- prompt token storage;
- sequence capacity and generated-token limit;
- sampling configuration and seed;
- EOS tokens and owned token stop patterns;
- scheduler quantum;
- required token/record output capacity.

It carries no tokenizer, text, path, display, workflow, frontend, or provider DTO. Before native sequence creation, E0 validates identities, prompt/total lengths, model lifecycle, required capabilities, advertised context/prefill limits, sampling, and output policy. It preflights and reserves backend sequence memory plus caller-owned generation workspaces for logits, sampling indices/epochs, repetition/prompt/generated history, EOS/stop storage, and terminal/backpressure state.

The backend still produces its exact `SequencePlan`. E0 repeats configuration, logits-capacity, identity, state, and footprint checks at commit. Generation workspace bytes remain reserved until the `Released` record is published and scheduler task storage is dropped, so downstream backpressure cannot make retained allocations appear available prematurely.

## Scheduler, sampling, and output

A scheduled request moves through:

```text
admitted -> prefill -> pending token publication -> decode
    -> terminal publication -> cleanup pending (optional) -> released
```

Each worker loop checks one control command, advances one request by a bounded opportunity, processes one cleanup retry/unload maintenance operation, and flushes bounded events. A rotating ordered cursor gives each runnable request an opportunity, while an independent rotating cleanup selector prevents one cleanup class or low-key owner from starving another. A request blocked on full output performs no backend step.

The current correctness baseline performs at most one token-producing backend step before token publication. Prefill occurs once. E0 samples immediately from verified caller-owned host F32 logits using request-owned `sampling::Sampler` state. CPU Candle uses contiguous host copies; CUDA uses Candle's safe device-to-host path. Upstream transfer may allocate a temporary CPU tensor, so no allocation-free CUDA hot-path claim is made.

`host-runtime` supplies a bounded token accumulator with preallocated token and record vectors. Records preserve request identity and ordered state:

- `Yielded(OutputBackpressure)`;
- `Terminal(original outcome)`;
- `CleanupPending { original outcome, failure report, retry state }`;
- `CleanupExhausted { original outcome, failure report, retry state }`;
- `Released(original outcome)`.

A sampled token blocked by output capacity remains request-owned; no decode step runs and no token is dropped or duplicated. Generation completion and backend resource release remain separate observable facts.

## Cancellation, unload, and shutdown

User cancellation is observed before the next backend operation; latency is bounded by the currently executing backend call, one-step quantum, and command polling cadence. EOS, token limits, stop patterns, cancellation, model unload, and drain timeout all enter the same explicit sequence cleanup path.

Immediate unload marks active work with `ModelUnload`; drain expiration marks it with `DrainTimeout`. Candle sequence destruction and model unload synchronize the actual selected device before release. A synchronization failure enters bounded retained cleanup.

Explicit shutdown is terminal. It deterministically releases newly ready models, consumes the finite remaining fair cleanup budget across sequences, complete models, and failed preparations, releases scheduler workspaces without waiting for frontend output draining, publishes exactly one shutdown result, and stops the worker. Retryable client-side join/wait behavior remains an E1 concern; unresolved E0 ownership is summarized as terminal process-lifetime retention and remains fail-closed until process exit.

## Production Candle integration and tests

Backend-independent deterministic loaders in `tests/generation.rs`, `tests/runtime.rs`, and `tests/fault_injection.rs` cover transaction validation, sampling, fairness, cancellation, output backpressure, cleanup retry/exhaustion, unload, accounting, disconnection, and shutdown.

Phase 12 fault injection additionally covers:

- exact preparation consumed once without replanning;
- invalid accepted configuration, empty/contradictory descriptor facts, checked overflow, and component-wise peak/final contradiction before materialization;
- aggregate loading-peak budget rejection before materialization;
- immediate failed-load cleanup preserving the primary error;
- failed-load cleanup retention with the full peak and `CleanupResource::FailedLoad`;
- bounded retry success, exact single release, mutation-resistant accepted accounting, and exhaustion;
- larger-than-peak, reclassified, smaller, identity-only, scalar, device, descriptor, and aggregate-overflow complete-model reports;
- immediate cleanup, fail-once recovery, permanent exhaustion, explicit unverified snapshots, and admission locking for every complete-model mismatch class;
- deterministic cleanup fairness across classes and within ordered owner maps;
- direct primary-operation failure preservation alongside observable retained cleanup;
- duplicate hosted unload rejection without overwriting the original accepted ticket.

`tests/native_backend_generation.rs` drives the committed project-authored F32 fixture and temporary deterministic mixed F16/F32 derivative through the real `CandleLlamaLoader` and hosted scheduler. It verifies exact prepared plan/receipt/snapshot scalar and footprint facts, generation, sampling, backpressure, cancellation, release, unload, empty accounting, shutdown, and join. The temporary derivative strategy and non-claims are recorded in fixture [`PROVENANCE.md`](../../crates/runtime/inference-runtime/tests/fixtures/candle-llama/PROVENANCE.md).

The 2026-08-10 artifact-loading amendment's CUDA evidence remains historical to that exact tree. The subsequent runtime-ownership amendment expanded the backend-independent matrix to 40 fault-injection tests, 25 generation tests, 11 registry/hosted tests, and three native Candle E0 tests on default CPU. Its exact portable, Clippy, rustdoc, and CUDA compile availability are recorded in [implementation status](implementation-status.md); no earlier hardware result is reused as evidence for this later tree.

External resolution remains above E0 in E1. No external mixed-dtype checkpoint has been accepted. Product/evidence status is canonical in [implementation status](implementation-status.md).
