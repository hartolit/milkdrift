# ADR-0020: Use transaction-bound prepared model loading

- **Status:** Accepted
- **Date:** 2026-08-08
- **Amended:** 2026-08-10 for artifact loading, retained-ownership certainty, and the application boundary; 2026-08-13 for honest expected-content input, bounded accelerator transfer batches, exact verifier/staging ownership, and loader organization; 2026-08-15 for bounded portable load diagnostics
- **Phase:** 12 plus post-audit artifact-loading, runtime-ownership, and application-boundary hardening
- **Implementation:** `58490fe693fef7a2635956181088664cd90685e8`, `12510695aa29be6a2665dbf3777cccbb8172c2d1`, `d4a1e4324a6793becc56147e4b2e3246189d2693`, `b43d0f47953c5319a41340d9087b7fd8f07b3280`, and `1f91cba691a8099805fa31f576079e79c282c73e`
- **Amends:** [ADR-0019](0019-explicit-cuda-execution-foundation.md) for scalar/source terminology, load planning/materialization, E1 loaded facts, model-catalogue persistence, and Phase 12 evidence claims
- **Preserves:** [ADR-0006](0006-explicit-bounded-shutdown.md), [ADR-0010](0010-verify-backend-contracts-at-e0.md), and [ADR-0013](0013-candle-only-local-execution.md)

## Context

The pre-Phase 12 Candle path treated one scalar declared by model configuration as if it described every serialized tensor. It planned and loaded in separate calls, scaled broad file-size quantities rather than calculating the selected tensor algorithm, and returned only a `LoadError` when materialization failed before a complete model existed.

Those assumptions were too weak for reviewed mixed F16/F32 and BF16/F32 Safetensors layouts:

- configuration metadata is producer intent, not observed tensor truth;
- a plan and later load could inspect materially different path contents;
- final model ownership and transient loading peak are different quantities;
- per-tensor cast/transfer can fail after native resources exist;
- E0 cannot quarantine a partial load if error conversion discards its only owner;
- a complete backend model that has already contradicted its accepted contract cannot be assigned the planned peak as though that remained proven exact ownership;
- E1 and Slint should not become a second Safetensors loader merely to display the change.

The correction must strengthen one local Candle endpoint without changing Milkdrift's workflow-first identity, introducing another engine, weakening explicit CUDA selection, or moving format details into portable workflow/domain boundaries.

## Decision

### Separate four scalar facts

The architecture uses four distinct meanings:

1. **Configuration declaration** is optional recognized `dtype`/`torch_dtype` producer intent derived from the same bounded `config.json` bytes Candle decodes. Callers cannot inject it.
2. **Complete observed scalar set** is a fixed-size, allocation-free `ScalarTypeSet` built from every structurally valid tensor header in every selected shard, including unused auxiliary tensors.
3. **Required scalar set and required primary** are adapter-private facts derived only from tensors consumed by the supported Llama schema. They alone drive declaration compatibility and execution policy.
4. **Execution scalar** is selected by exact device-aware preparation, materialized only for required execution tensors, reported by the loaded backend model, and verified by E0.

The declaration never substitutes for observed headers. Complete observed evidence never selects precision. Unused extras cannot downcast required F32 weights or make a matching declaration contradictory. Detailed tensor names, offsets, shard paths, shapes, source dtypes, and whole-shard identities remain adapter-private.

### Treat declaration presence and precedence honestly

Modern `dtype` has no silent fallback semantics. Both declaration fields absent/null means absence; one recognized field plus one absent/null field selects the recognized value; two equal recognized fields select that value. Any present unsupported value fails, including unsupported modern `dtype` paired with recognized legacy `torch_dtype`. Two different recognized values conflict. Duplicate fields, wrong JSON types, and malformed JSON fail explicitly. Raw vendor strings remain inside the artifact adapters and are never persisted as preferences.

### Accept only the reviewed required layouts

The Candle Llama/Safetensors adapter accepts exactly these **required** sets:

| Required set | Required primary | Permitted recognized declaration |
|---|---|---|
| `{F32}` | F32 | absent or F32 |
| `{F16}` | F16 | absent or F16 |
| `{F16,F32}` | F16 | **F16 required** |
| `{BF16}` | BF16 | absent or BF16 |
| `{BF16,F32}` | BF16 | **BF16 required** |

A genuine required F16+BF16 mixture remains rejected, with or without F32. An empty required set or a required integer, boolean, FP8, bit-packed, complex, or otherwise non-executable category is rejected before device initialization. Structurally understood **unused** tensors may use any Safetensors 0.8 dtype: they remain in complete observed evidence but do not affect compatibility, execution, transfer, or footprint policy. A mixed required set is not itself evidence of primary precision: the same set can describe a lower-precision model with F32 auxiliaries or an F32 model with an incidental lower-precision tensor. Milkdrift therefore requires the matching recognized declaration before a lossy mixed conversion; only homogeneous required sets may use an absent declaration.

The execution policy is:

| Required primary | CPU | Supported CUDA policy |
|---|---|---|
| F32 | F32 | F32 |
| F16 | F16 | F16 |
| BF16 | F32 | BF16 only when the selected device reports support |

Each required tensor is independently converted to the selected execution dtype when needed. Vocabulary logits still cross to E0 as host F32.

### Replace `plan_load`/`load` with one prepared transaction and distinct failure typestate

`ModelLoader` has associated `Prepared: PreparedLoad`, `FailedPreparation: FailedLoadOwner`, and `Model` types plus two load operations:

- `prepare_load(&mut self, source, configuration) -> Result<Prepared, LoadError>` creates one exact source/configuration/device-bound preparation and exposes its `LoadPlan` through `PreparedLoad::plan()`;
- `load_prepared(&mut self, prepared) -> Result<Model, FailedLoad<FailedPreparation>>` consumes that exact preparation without replanning.

`PreparedLoad::Failed` must equal the loader's `FailedPreparation`. An unmaterialized preparation is ordinary-drop-safe. This permits E0 to reject an invalid plan or insufficient aggregate peak without invoking backend cleanup. Its `plan()` is stable for the preparation's lifetime and describes the one accepted source/configuration/device transaction; E0 reads it once and never calls a second planner or reconstructs backend policy.

After materialization begins, failure returns `FailedLoad<FailedPreparation>` containing both the primary `LoadError` and the distinct sole cleanup owner. `FailedLoad` does not expose replaceable public fields, and neither the preparation, failed typestate, nor loaded model may alias cleanup authority elsewhere. `FailedLoadOwner::cleanup(&mut self)` is explicit and retryable: failure leaves the owner valid, complete, and plan-report-stable; success is the only ordinary transition that proves explicit release. Consuming `load_prepared` makes a preparation impossible to materialize twice through the portable API, while separating the associated types prevents one public type from carrying contradictory pre-attempt and post-failure drop semantics.

### Bind accepted weight facts to retained files and whole-shard identities

Candle completes bounded selected-shard/header inspection before device initialization. It sorts complete path/identity pairs, opens and retains every shard, validates all tensor metadata and required Llama shapes, and records exact ranges plus the retained prefix/header digest. It does not scan payloads merely to inspect structure.

There are two Candle verification paths:

- `CandleExpectedContentIdentity` carries only the exact expected length and whole-file SHA-256. It makes no provider-provenance or path-immutability claim and may skip Candle's local pre-admission baseline pass.
- an unverified local shard carries no reusable expectation, so Candle hashes its retained file sequentially before admission and establishes a fresh baseline.

Provider authority remains a separate Hub/application evidence fact. The Hub adapter accepts an exact LFS SHA-256 plus size at the resolved commit as `HuggingFaceLfs`; for a non-LFS file it verifies the exact Git blob SHA-1 at that commit while streaming the bytes, derives SHA-256, and records `HuggingFaceGitBlob`; project-computed local evidence remains `ProjectEstablished`. Cache names, symlink targets, ETags, unverified object identifiers, inode/mtime facts, and provider conventions alone are not proof. E1 passes only exact length/SHA-256 expectations into Candle, which verifies every materialization stream before publication.

Materialization reads every retained shard sequentially from byte zero. It verifies the retained header before payload processing, hashes ignored ranges through one fixed 64 KiB buffer, stages only required ranges, and compares exact EOF/length and the accepted whole-shard digest before model construction/publication. CPU tensors enter final ownership sequentially. Accelerator transfers enter the bounded transaction described below. There are no per-tensor seeks, payload digests, mmap, unsafe code, or whole-model host retention. Deleting or replacing the path cannot redirect the retained file; same-inode mutation, truncation, and extension fail before publication. The parsed configuration, exact load configuration, selected device, inspected shards, immutable batch partition, and plan remain in the opaque preparation.

### Bound inspection metadata independently

One private production limit set rejects hostile growth before device initialization: at most 256 shards; 8 MiB per header and 64 MiB aggregate headers; 16,384 tensors; 512 bytes per name and 8 MiB aggregate names; rank 8; dimension extent 1,048,576 and 131,072 aggregate dimensions; 1,024 metadata entries with 256-byte keys, 4 KiB values, and 4 MiB aggregate metadata strings; and 64 MiB final owned inspection inventory. Configuration bytes are capped at 1 MiB. Custom deserializers enforce these limits while traversing JSON, fixed-rank shape storage avoids per-shape vectors, and checked reservations turn allocation failures into deterministic adapter failures. Parsed configuration/header/inspection and required-name/load-map metadata remain separately bounded. Accelerator admission additionally counts the actual retained heap capacities of `TransferPlan`'s `TransferBatchPlan`/`TransferEntryPlan` vectors and `TransferBatchOwner`'s `TransferBatchEntry` vector in host working memory. Names move from manifest to batch to final map. Map buckets, additional tensor handles outside the counted plan/owner vector allocations, allocator overhead, fragmentation, and other physical observations remain outside exact logical ownership rather than receiving a fabricated portable byte count.

### Use deterministic bounded accelerator transfer transactions

Preparation constructs one immutable `TransferPlan` which the footprint planner and materializer both consume. It stores flat `TransferEntryPlan` inventory plus `TransferBatchPlan` ranges and is not independently rederived during loading. `PREFERRED_BATCH_HOST_STAGING_BYTES` is 256 MiB and `MAXIMUM_BATCH_ENTRIES` is 64. A nonempty accelerator batch closes before adding a tensor when the candidate would exceed either policy. Every shard end fixes a final batch boundary, so no batch crosses shards. The first tensor in an empty batch is always admitted; if its candidate peak exceeds 256 MiB, it becomes an oversized singleton and remains subject to the accepted model/load budget.

Each private `TransferBatchEntry` owns its manifest position, name, and commit state plus its source tensor, optional converted host tensor, and transferred device tensor. The sole `TransferBatchOwner` records checked byte/count, active-batch, synchronization, and committed-entry accounting. Transfers may be enqueued only after these owners are reachable. Full intermediate batches synchronize as their planned boundary is reached. A shard's final planned batch instead remains unsynchronized until exact EOF, length, and whole-shard SHA-256 succeed, so a late identity failure retains every enqueued endpoint for cleanup rather than committing bytes from the wrong source.

When a planned batch is eligible to close—immediately at an intermediate boundary, or after whole-shard identity for the shard-final boundary—endpoint validation and exactly one synchronization must both complete before commit. The owner then inserts one shallow device handle at a time while retaining the original device and every host endpoint in the batch; explicit per-entry commit state makes a mid-commit failure recoverable. Only after all entries commit does the owner release host staging and reset for the next planned batch.

Any failure consumes the prepared value into the same sole failed-preparation owner. That owner retains all earlier committed tensors, the complete current batch, open shards, configuration, device, accepted plan, and any constructed model. Cleanup synchronizes before release. A cleanup synchronization failure changes none of those ownership facts, so retry is idempotent and cannot double-release a committed or pending tensor.

After all batches commit and all shards verify, the locked Candle Llama constructor creates shallow handles over the existing weight tensors. This construction enqueues no distinct device work. Normal loading therefore has one synchronization per nonempty transfer batch and no unconditional final synchronization; cleanup synchronization is a separate failure-release boundary.

### Preserve bounded load provenance without leaking adapter inventory

`LoadError::Backend` carries a `BackendLoadFailure`: the existing stable
backend/kind/code identity plus optional fixed-size `LoadFailureContext`. Context
contains one portable lifecycle stage and, only when truthful, one
`TensorFailureLocation` made of checked shard/tensor ordinals, a stable name
fingerprint, and the project-owned observed scalar classification. These values
are allocation-free, `Copy`, `no_std` domain contracts. They contain no tensor
name, path, offset, digest, native error, vendor type, or serialization DTO.

Candle owns one documented deterministic ordinal/fingerprint algorithm and uses
the same source-scalar classification as complete observed evidence. The canonical
coordinate order, exact fingerprint constants/byte procedure, and per-stage
population rules live in
[Candle backend](../../project/candle-backend.md#bounded-portable-load-diagnostics),
not in portable contracts or E0/E1. The fingerprint remains diagnostic
correlation, not authentication or source identity.

One exact coordinate is attached for required-tensor scalar/shape rejection,
concrete payload read, host materialization, scalar conversion, device transfer,
and retained placement failures. Configuration, missing/duplicate tensor,
global layout/identity/capacity, construction, and batch synchronization failures
remain stage-only when no single tensor is authoritative. E0 preserves an exact
backend load detail through immediate cleanup, retained retry, exhaustion, and
terminal evidence, but generic E0 contract contradictions remain class-only and
never acquire a fabricated tensor coordinate. E1 exposes the same optional
portable diagnostic separately from presentation text and does not persist it.

Cleanup is an independent operation and identity. A partial-load cleanup
synchronization failure cannot replace, reclassify, or erase the primary load
diagnostic; the sole failed owner retains both the primary detail and cleanup
state for retry.

### Distinguish exact final ownership from the loading peak

`LoadPlan::final_footprint` is exact final required execution-tensor byte ownership. `LoadPlan::loading_peak_footprint` is the exact component-wise deterministic byte peak for selective materialization. `MemoryFootprint` contains only host/device weight and host/device working bytes. `ModelDescriptor::sequence_cache_bytes_per_token` is a separate planning rate used to derive concrete `SequencePlan` ownership; a rate is not current ownership. None of these quantities is physical RSS/VRAM or allocator/driver accounting.

For required tensor `i`, let:

- `S_i` be exact source payload bytes;
- `E_i` be exact execution bytes;
- `A_i = S_i + alignment_i - 1` be the aligned staging allocation bound;
- `P_i` be required execution bytes already retained before tensor `i`;
- `R = sum(E_i)` be all required Llama execution bytes;
- `V = 64 KiB` be the verification buffer live for the current shard;
- `M_plan = capacity(TransferPlan.batches) * size_of::<TransferBatchPlan>() + capacity(TransferPlan.entries) * size_of::<TransferEntryPlan>()`;
- `M_owner = capacity(TransferBatchOwner.entries) * size_of::<TransferBatchEntry>()`;
- and `M = M_plan + M_owner`, using the actual checked capacities retained by the prepared load.

Unused tensors enter no formula. CPU creates neither `TransferPlan` nor `TransferBatchOwner`, so it has no `M` term. The CPU final footprint owns `R` host weight bytes. Its host loading peak is:

```text
Hcpu = max(
    R,
    V,
    max_i(V + P_i + A_i + S_i),
    max_cast_i(V + P_i + S_i + E_i)
)
```

The CPU final footprint records `R` host weight bytes. The CPU loading footprint records `R` host weight bytes and `Hcpu - R` host working bytes.

For tensor `i` in accelerator batch `b`, additionally let:

- `Q_i = S_i` when no cast is required, otherwise `Q_i = S_i + E_i`, because the batch retains both source and converted host tensors through synchronization;
- `C_b,i = sum(Q_j)` for entries `j` already retained in batch `b`; and
- `W_b` be the exact implemented live host tensor-staging peak for that batch:

```text
W_b = max(
    sum_i(Q_i),
    max_i(C_b,i + A_i + S_i),
    max_cast_i(C_b,i + S_i + E_i)
)

Haccelerator = M + V + max_b(W_b)
```

`sum_i(Q_i)` is the post-enqueue retained host payload; `C_b,i + A_i + S_i` is the aligned-payload/source construction phase; and the cast term is the simultaneous prior-batch/source/execution phase. `M` and `V` are additive because the plan/owner capacities remain retained throughout materialization and the verifier remains allocated throughout staging and while the shard-final batch waits for identity verification and then synchronizes/commits. The candidate `W_b` is also the quantity compared with the preferred byte target, so the limit is tied to the actual ownership graph rather than added as a generic margin.

The accelerator final footprint records `R` device weight bytes. The accelerator loading footprint records `Haccelerator` host working bytes, `R` device weight bytes, and zero device working bytes. During commit a batch entry and the final map temporarily hold shallow handles to the same storage; model construction adds another shallow handle before the map is released. These aliases do not duplicate payload, and all transferred-but-uncommitted plus committed storage remains within `R`, so it is classified as device weight ownership rather than inventing device working bytes. The descriptor separately reports the exact sequence-cache byte rate at execution width. Every operation uses checked arithmetic. Parsed config/header/inspection and name/map metadata remain governed by their separate structural ceilings; allocator bookkeeping/fragmentation, driver/context allocation, process RSS, and whole-device observations are not represented as exact ownership.

### Admit the peak at E0 before materialization

E0 validates that the preparation accepted its exact `LoadConfiguration`, the descriptor is coherent and has a nonempty observed set, checked final/peak totals do not overflow, and every peak ownership component contains the corresponding final component.

Given a pre-load reservation `R0`, E0 independently admits both `R0 + loading_peak` and `R0 + final` against its fixed aggregate budget. It reserves `R0 + loading_peak` before calling `load_prepared`.

On success, E0 verifies handle, complete descriptor, actual device, actual execution scalar, final reported footprint, and lifecycle transition. Only then does it commit a model slot, replace peak reservation with final reservation, and publish a receipt.

On materialization failure, E0 reads the failed owner's lifetime-stable plan, immediately attempts `FailedLoadOwner::cleanup`, and reads the plan again:

- matching reports plus cleanup success restore `R0` and return the original load failure;
- matching reports plus cleanup failure retain `PendingModelOwner::FailedPreparation { owner, accepted_plan }`, the generation-safe model identity, and the full exact loading peak;
- any plan substitution or mutation becomes a backend-contract failure and `RetainedOwnership::Unverified`; every contradictory report extends conservative evidence monotonically and cannot be erased by a later smaller report;
- the cleanup resource is `FailedLoad { handle }`;
- primary model-load/contract and cleanup failure classes remain separate;
- the initial failure is attempt one, bounded retry is shared with existing cleanup policy, and exhausted ownership remains quarantined/accounted or admission-blocking according to ownership certainty.

A complete model that fails post-load E0 verification first follows explicit unload. If unload succeeds, no owner remains. If unload fails, E0 must not retain the accepted loading peak as exact: the backend has already contradicted its contract, so the peak is not proof that hidden, larger, or differently classified ownership is absent. E0 retains `PendingModelOwner::IncompatibleModel(model)` with `RetainedOwnership::Unverified` containing the accepted peak, the backend-reported footprint, and their checked component-wise conservative maximum. If a component or aggregate host/device total overflows, evidence is `ConservativeFootprint::Overflow`; E0 does not saturate, substitute `u64::MAX`, or use sampled RSS/device memory as ownership accounting.

Unverified ownership is excluded from the exact `reserved_footprint`, exposed separately in snapshots and cleanup state, and blocks every new model, sequence, cache, and workspace admission because no exact upper bound has been established. Existing admitted healthy work remains runnable. A later successful cleanup removes the owner exactly once, records `RetainedOwnership::Released` even on the final permitted attempt, and unlocks admission. Exhaustion remains observable and admission-blocking until process reclamation. The same unverified rule applies when a correct footprint is paired with a wrong handle, descriptor, device, or scalar, and when the report is smaller than planned.

This extends ADR-0010's E0 verification boundary. E0 verifies portable claim consistency but cannot prove a third-party backend's physical allocation, placement, hidden aliases, or completeness after that backend violates the contract.

### Rotate cleanup opportunities fairly

`poll_cleanup` remains bounded to at most one backend cleanup opportunity. Selection rotates across pending sequences, failed preparations, and complete models, then rotates among eligible owners within each class. Retry budgets remain per owner; exhausted owners are observable but skipped automatically. Shutdown deterministically consumes the finite remaining opportunities under the same ordering.

### Preserve explicit terminal cleanup policy

The raw adapter failed typestate performs no hidden abandonment. It is encapsulated by the project-owned `FailedLoad` guard, which is the portable cleanup authority and deliberately retains an unresolved raw owner when a direct caller abandons the failure before cleanup succeeds. Under normal operation E0 remains the reachable owner through retry or exhaustion; the guard's fail-closed drop is the direct-API safety net, not a substitute for E0 cleanup.

ADR-0006 remains authoritative for terminal shutdown: if the finite explicit cleanup budget is exhausted while the complete E0 runtime still owns native resources, shutdown returns `TerminalCleanupRetention` with the first exhausted owner and a bounded summary distinguishing failed preparations, verified models, incompatible models, retained sequences, and aggregate unverified evidence. The worker may then use the named `RetainUntilProcessExit` disposition and retain the complete runtime allocation only after publishing that structured terminal failure. A directly owned synchronous runtime also retains unresolved backend-owner maps on implicit drop, because explicit cleanup success is the only ordinary drop authorization. Process termination remains the reclamation boundary. Endpoint disconnection or worker-handle absence never becomes proof of release. This is distinct from ordinary prepared-load retry.

### Keep loading policy adapter-owned and modules cohesive

Transfer partitioning, ownership, synchronization, and footprint simulation are Candle-adapter responsibilities. E0 admits only the portable plan and owns the opaque failed typestate; E1 retains provider provenance and product lifecycle evidence. CUDA is the only implemented accelerator path. This decision adds no AMD execution and no NVIDIA-specific batch type or policy to E0, E1, or portable workflow contracts. Another accelerator may reuse the adapter-private partition only after its transfer/synchronization semantics establish the same lifetime and cleanup proof.

Loader production code is organized by invariant: `config` and `safetensors` own bounded decoding; `manifest` owns inspected source layout; `identity` owns content establishment; `configuration_policy`, `scalar`, and `schema` own supported execution policy; `payload` owns aligned streaming and CPU tensor construction; `transfer_plan` owns `TransferPlan`, `TransferBatchPlan`, and `TransferEntryPlan`; `transfer_batch` owns the sole live `TransferBatchOwner`; `prepared` coordinates materialization; `construction` builds Llama through a borrowed tensor-map backend; `footprint` consumes the same partition/lifetimes; and `cleanup` owns failed release. The large corpora live in `config/tests.rs`, `safetensors/tests.rs`, and `scalar/tests.rs` instead of inline production bodies. This introduces neither another crate nor hot-path dynamic dispatch.

### Keep E1 and Slint narrow

E1 receives lower descriptor/receipt facts but does not reproduce Candle policy.

- `ResolvedModel` exposes immutable identity and optional configuration-declared metadata.
- Public E1 `LoadedModel` exposes the E0-verified execution scalar and actual execution device, but no declaration, observed set, required primary, or per-tensor inventory.
- E1 checks declaration agreement across artifact/admission/descriptor evidence, a nonempty complete observed set, receipt identity/capabilities/device, and final reserved footprint. Integer or `Other` bits from unused tensors are truthful evidence rather than an E1 compatibility policy.
- E1 does not infer required primary, choose per-tensor conversion, compare declaration with execution, or fall back.
- Retained lower model ownership is durable in `ApplicationState` as `ApplicationRetainedModel`, preserving resource, ownership certainty, cleanup disposition, and independent primary/cleanup failures; `ModelCleanupPending { resource, disposition }` is the compact transition notification, and ordinary owner-free failure remains `ModelLoadFailed`.
- Slint may display the optional declaration in a resolved summary and execution scalar/device in a loaded summary. It gains no tensor table, conversion control, or backend responsibility.

This preserves ADR-0013: Candle remains the sole local engine, E0 remains generic/backend-neutral at portable contracts, E1 remains non-generic/private concrete composition, and token-sensitive work remains statically dispatched.

### Persist declaration only

New `LAM1` writes use version 3 and store optional configuration-declared scalar metadata through an explicit presence tag. Exact version 1 and 2 reads remain supported without automatic rewrite; version 1 decodes its mandatory scalar as present, while version 2 recognizes its historical absence code. Observed sets, required primary, execution scalar/device, per-tensor inventory, footprints, shard identities, and cache paths are not persisted.

`LAS1` settings semantics remain unchanged: version 2 writes and exact version 1 reads continue to own device selection and accelerator-memory policy.

### Keep support evidence narrower than implementation

The implementation and deterministic test matrix do not themselves establish a new hardware run or an external-checkpoint claim. Phase 12 CUDA compile, CUDA hardware execution, canonical clean-target acceptance, and external mixed-checkpoint evidence must be recorded separately in the implementation status. That owner records the passed local compile/hardware and canonical gates and the explicit absence of an accepted external mixed checkpoint. Historical Phase 11 hardware evidence remains historical and is not rewritten as Phase 12 execution.

No default feature reaches CUDA. Explicit CUDA failure never falls back to CPU. No generic `gpu`, unsafe copy path, quantized format, new architecture, provider/peer target, or portable-domain filesystem/vendor type is introduced.

## Rejected alternatives

- **Keep independent `plan_load` and `load`:** rejected because E0 could admit facts not bound to the materialization owner.
- **Scale complete shard file lengths by one scalar width:** rejected because headers, mixed source widths, required versus extra tensors, casts, and transient ownership make that quantity false.
- **Treat configuration metadata as homogeneous observed truth:** rejected because model configuration does not enumerate tensor headers.
- **Accept every floating mixture and cast it:** rejected because precision, model compatibility, and memory behavior need an explicit reviewed matrix.
- **Rely on scope exit after partial CUDA work:** rejected because cleanup/synchronization can fail and must preserve the only owner for retry.
- **Synchronize each transferred tensor:** rejected because the synchronization boundary would no longer match the bounded multi-entry ownership transaction.
- **Allow one unbounded transfer batch:** rejected because neither host tensor staging nor batch-entry metadata would have a deterministic bound.
- **Add a generic batch margin:** rejected because loading admission must simulate the implemented simultaneous source, cast, retained-batch, verification-buffer, and final-weight graph.
- **Retain an unconditional final load synchronization:** rejected because locked Llama construction is handle-only and establishes no distinct pending device-work boundary.
- **Expose tensor/shard DTOs through portable contracts or E1:** rejected because no generic consumer needs Safetensors-specific inventory.
- **Let E1 infer conversions from device or declaration:** rejected because that duplicates adapter policy and can diverge from the accepted preparation.
- **Persist observed/execution facts for display:** rejected because they are load-time evidence, not durable selection/preferences.
- **Expand Slint into a model-inspection UI:** rejected because the reference host remains thin and replaceable.
- **Treat the Phase 11 hardware run as Phase 12 evidence:** rejected because it predates the prepared transaction and mixed-layout implementation.

## Consequences

- Exact preparation, aggregate admission, materialization, and cleanup now share one owner and one plan.
- Mixed F16/F32 and BF16/F32 **required** layouts are supported only under the exact reviewed declaration policy; complete observed extras cannot select execution.
- Structurally understood unused tensors are inspected and identity-bound but never staged, converted, transferred, or retained.
- CPU behavior remains F32→F32, F16→F16, and BF16→F32; CUDA policy remains explicit and capability-checked.
- Accelerator transfers use deterministic 256 MiB-preferred, 64-entry, shard-bounded batches with an allowed oversized singleton.
- Normal loading synchronizes once per nonempty transfer batch and not after handle-only Llama construction.
- Final and loading footprints contain the implemented tensor and verification-buffer bytes; sequence-cache rate is separate, and ignored extras enter neither load footprint.
- Failed preparations retain the exact admitted loading peak; verified model unload failures retain exact final ownership.
- Backend load failures may preserve bounded stage and exact tensor correlation through E0/E1 without exporting adapter inventory; cleanup identity remains separate.
- Contract-violating complete models retain explicit unverified evidence, block admission, and are never mislabeled exact.
- E0 can retry or exhaust a failed partial load or incompatible complete model without publishing it or losing generation-safe identity/accounting state.
- Cleanup polling is bounded and fair across owner classes and identities; terminal retention remains structured through process exit.
- E1 and Slint become simpler: resolved declaration and loaded execution facts are no longer collapsed into one source-scalar label.
- Persistence can represent absent declarations without storing per-tensor runtime evidence.
- Synthetic E0 evidence may observe existing public plan/receipt facts. External E1 evidence observes the public product boundary without a shadow preparation, and production APIs are not expanded for reports.
- Milkdrift remains centered on operator-defined workflows and explicit ownership; Phase 12 hardens one local endpoint and stops at that boundary.

## Review trigger

Review this decision when adding another accepted scalar set, execution dtype, quantized format, model architecture, loading algorithm, asynchronous materialization contract, source-identity mechanism, ownership-certainty class, or cleanup owner; when final/peak/cache-rate formulas change; when E1 has a demonstrated consumer for additional source-layout facts; or when another backend cannot honestly implement prepared loading and retryable sole-owner cleanup.

Device selection, default-feature, fallback, and hardware-support changes continue to trigger ADR-0019 review. Terminal process-lifetime cleanup changes trigger ADR-0006 review. Backend substitution changes trigger ADR-0010 review. A second local engine triggers ADR-0013 review rather than silently weakening this transaction.
