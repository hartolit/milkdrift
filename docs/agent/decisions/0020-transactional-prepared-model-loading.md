# ADR-0020: Use transaction-bound prepared model loading

- **Status:** Accepted
- **Date:** 2026-08-08
- **Amended:** 2026-08-10 for artifact loading, retained-ownership certainty, and the application boundary
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

There are two identity paths:

- `VerifiedImmutable` carries exact length and whole-file SHA-256 from a source whose content-addressing and immutability contract is proven. Only this authority skips a pre-admission baseline pass.
- `ProjectEstablished` and `Unverified` are mutable-source fallbacks. Candle hashes its retained file sequentially before admission; a supplied project digest must match, while an unverified source establishes a fresh baseline.

A Hugging Face LFS SHA-256 paired with exact file size at the resolved commit is the current provider-verified authority. Cache names, symlink targets, ETags, Git object IDs, inode/mtime facts, and provider conventions are not cryptographic proof. When Hub LFS identity is unavailable, the Hub adapter establishes a local whole-file digest, marks it project-established, and Candle revalidates it before admission.

Materialization reads every retained shard sequentially from byte zero. It verifies the retained header before payload processing, hashes ignored ranges through one fixed 64 KiB buffer, stages only required ranges, and compares exact EOF/length and the accepted whole-shard digest before model construction/publication. There are no per-tensor seeks, payload digests, mmap, unsafe code, or whole-model host retention. Deleting or replacing the path cannot redirect the retained file; same-inode mutation, truncation, and extension fail before publication. The parsed configuration, exact load configuration, selected device, inspected shards, and plan remain in the opaque preparation.

### Bound inspection metadata independently

One private production limit set rejects hostile growth before device initialization: at most 256 shards; 8 MiB per header and 64 MiB aggregate headers; 16,384 tensors; 512 bytes per name and 8 MiB aggregate names; rank 8; dimension extent 1,048,576 and 131,072 aggregate dimensions; 1,024 metadata entries with 256-byte keys, 4 KiB values, and 4 MiB aggregate metadata strings; and 64 MiB final owned inspection inventory. Configuration bytes are capped at 1 MiB. Custom deserializers enforce these limits while traversing JSON, fixed-rank shape storage avoids per-shape vectors, and checked reservations turn allocation failures into deterministic adapter failures. These resources are separately bounded rather than folded into tensor `MemoryFootprint`. Required-name/load-map metadata is also bounded by the tensor/name ceilings: one name clone is transient per required tensor, and failure-safe model construction temporarily duplicates one map's names and shallow tensor handles under the same bounds. Platform-dependent map bucket overhead remains outside exact tensor accounting but cannot exceed the bounded entry count.

### Distinguish exact final ownership from the loading peak

`LoadPlan::final_footprint` is exact final required execution-tensor byte ownership. `LoadPlan::loading_peak_footprint` is the exact component-wise deterministic byte peak for selective materialization. `MemoryFootprint` contains only host/device weight and host/device working bytes. `ModelDescriptor::sequence_cache_bytes_per_token` is a separate planning rate used to derive concrete `SequencePlan` ownership; a rate is not current ownership. None of these quantities is physical RSS/VRAM or allocator/driver accounting.

For required tensor `i`, let:

- `S_i` be exact source payload bytes;
- `E_i` be exact execution bytes;
- `A_i = S_i + alignment_i - 1` be the aligned staging allocation bound;
- `P_i` be required execution bytes already retained before tensor `i`;
- `R = sum(E_i)` be all required Llama execution bytes.

Unused tensors enter no formula. The CPU final footprint owns `R` host weight bytes. Its host loading peak is:

```text
Hcpu = max(
    R,
    max_i(P_i + A_i + S_i),
    max_cast_i(P_i + S_i + E_i)
)
```

The CPU final footprint records `R` host weight bytes. The CPU loading footprint records `R` host weight bytes and `Hcpu - R` host working bytes.

The CUDA final footprint owns `R` device weight bytes. Its host staging peak is:

```text
Hcuda = max(
    max_i(A_i + S_i),
    max_i(E_i),
    max_cast_i(S_i + E_i)
)
```

The CUDA final footprint records `R` device weight bytes. The CUDA loading footprint records `Hcuda` host working bytes, `R` device weight bytes, and zero device working bytes. The descriptor separately reports the exact sequence-cache byte rate at execution width. Every operation uses checked arithmetic. The fixed 64 KiB verification buffer and bounded config/header/inspection metadata are governed by separate structural ceilings rather than `MemoryFootprint`; allocator bookkeeping/fragmentation, driver/context allocation, process RSS, and whole-device observations remain outside deterministic tensor accounting.

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

### Keep E1 and Slint narrow

E1 receives lower descriptor/receipt facts but does not reproduce Candle policy.

- `ResolvedModel` exposes immutable identity and optional configuration-declared metadata.
- Public E1 `LoadedModel` exposes the E0-verified execution scalar and actual execution device, but no declaration, observed set, required primary, or per-tensor inventory.
- E1 checks declaration agreement across artifact/admission/descriptor evidence, a nonempty complete observed set, receipt identity/capabilities/device, and final reserved footprint. Integer or `Other` bits from unused tensors are truthful evidence rather than an E1 compatibility policy.
- E1 does not infer required primary, choose per-tensor conversion, compare declaration with execution, or fall back.
- Retained lower model ownership is reported as `ModelCleanupPending { cleanup: ApplicationRetainedModel }`, preserving resource, ownership certainty, cleanup disposition, and independent primary/cleanup failures; ordinary owner-free failure remains `ModelLoadFailed`.
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
- Final and loading footprints contain concrete bytes only; sequence-cache rate is separate, and ignored extras enter neither load footprint.
- Failed preparations retain the exact admitted loading peak; verified model unload failures retain exact final ownership.
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
