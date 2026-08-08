# ADR-0020: Use transaction-bound prepared model loading

- **Status:** Accepted
- **Date:** 2026-08-08
- **Phase:** 12
- **Implementation:** `58490fe693fef7a2635956181088664cd90685e8` and `12510695aa29be6a2665dbf3777cccbb8172c2d1`
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
- E1 and Slint should not become a second Safetensors loader merely to display the change.

The correction must strengthen one local Candle endpoint without changing Milkdrift's workflow-first identity, introducing another engine, weakening explicit CUDA selection, or moving format details into portable workflow/domain boundaries.

## Decision

### Separate four scalar facts

The architecture uses four distinct meanings:

1. **Configuration-declared optional scalar** is recognized immutable `dtype` or legacy `torch_dtype` metadata. It is `Option<ScalarType>`, may be absent, and is evidence of producer intent only.
2. **Observed tensor scalar set** is a fixed-size, allocation-free `ScalarTypeSet` built from every selected Safetensors tensor header. It crosses the adapter boundary only as compact format-neutral descriptor evidence.
3. **Inferred primary scalar** is an adapter-private result derived from the exact observed set. It is not an E0, E1, persistence, or frontend policy axis.
4. **Execution scalar** is selected by exact device-aware preparation, materialized for backend execution tensors, reported by the loaded backend model, and verified by E0.

The optional declaration never substitutes for observed headers. The observed set never substitutes for execution. Detailed tensor names, offsets, shard paths, shapes, source dtypes, and payload digests remain adapter-private.

### Accept only the reviewed initial layouts

The Candle Llama/Safetensors adapter accepts exactly:

| Observed set | Inferred primary | Permitted recognized declaration |
|---|---|---|
| `{F32}` | F32 | `None` or F32 |
| `{F16}` | F16 | `None` or F16 |
| `{F16,F32}` | F16 | `None` or F16 |
| `{BF16}` | BF16 | `None` or BF16 |
| `{BF16,F32}` | BF16 | `None` or BF16 |

Every other set is rejected. In particular, F16+BF16 is not accepted, with or without F32. Empty, FP8, integer, boolean, unknown, and quantized tensor categories are unsupported. A present recognized declaration must equal the inferred primary; a contradictory or unsupported present declaration is unsupported format rather than evidence that every byte is corrupt.

The execution policy is:

| Inferred primary | CPU | Supported CUDA policy |
|---|---|---|
| F32 | F32 | F32 |
| F16 | F16 | F16 |
| BF16 | F32 | BF16 only when the selected device reports support |

Every accepted tensor is independently converted to the selected execution dtype when required. Vocabulary logits still cross to E0 as host F32.

### Replace `plan_load`/`load` with one prepared transaction

`ModelLoader` has an associated `Prepared: PreparedLoad` and two load operations:

- `prepare_load(&mut self, source, configuration) -> Result<Prepared, LoadError>` creates one exact source/configuration/device-bound preparation and exposes its `LoadPlan` through `PreparedLoad::plan()`;
- `load_prepared(&mut self, prepared) -> Result<Model, FailedLoad<Prepared>>` consumes that exact preparation without replanning.

An unmaterialized preparation is ordinary-drop-safe. This permits E0 to reject an invalid plan or insufficient aggregate peak without invoking backend cleanup.

After materialization begins, failure returns `FailedLoad<Prepared>` containing both the primary `LoadError` and the sole cleanup owner. `PreparedLoad::cleanup(&mut self)` is explicit and retryable: failure leaves the owner valid and complete; success authorizes drop as fully released.

### Bind accepted weight facts to retained files and digests

Candle preparation completes all selected shard-header inspection before device initialization. It sorts paths, bounds shard/header processing, opens and retains every weight file, validates all tensor metadata and required Llama shapes, and records each tensor's exact range and SHA-256 payload digest.

Materialization reads from the retained open files rather than reopening paths. It rechecks each retained file length and each payload digest. Therefore deleting or replacing a path cannot redirect an accepted preparation, while same-inode payload mutation is detected before the changed tensor is accepted. The parsed configuration, exact load configuration, selected device, inspected shards, and plan remain in the opaque preparation.

### Distinguish exact final ownership from the loading peak

`LoadPlan::expected_footprint` is the exact final post-load required execution-tensor ownership plus cache bytes per token. `LoadPlan::loading_peak_footprint` is the exact component-wise deterministic tensor peak for the chosen loading algorithm. Neither is physical RSS/VRAM or allocator/driver accounting.

For tensor `i`, let:

- `S_i` be exact source payload bytes;
- `E_i` be exact execution bytes;
- `A_i = S_i + alignment_i - 1` be aligned staging allocation;
- `P_i` be execution bytes already retained before tensor `i`;
- `R` be required-Llama execution bytes;
- `M` be execution bytes for every selected tensor, including supported extras;
- `C` be exact cache bytes per token at execution width.

The CPU final footprint owns `R` host weight bytes and no working bytes. Its host loading peak is:

```text
Hcpu = max(
    M,
    max_i(P_i + A_i + S_i),
    max_i(P_i + S_i + E_i) for casts
)
```

The CPU loading footprint records `R` as host weight and `Hcpu - R` as host working headroom.

The CUDA final footprint owns `R` device weight bytes and no working bytes. Its host staging peak is:

```text
Hcuda = max_i(A_i + S_i, E_i, S_i + E_i for casts)
```

The CUDA loading footprint records `Hcuda` host working bytes, `R` device weight bytes, and `M - R` device working headroom for selected extras. Both phases carry the same `C`. Every operation uses checked arithmetic.

The quantities exclude headers/configuration, file/digest metadata, allocator bookkeeping/fragmentation, driver/context allocations, process RSS, and whole-device observations.

### Admit the peak at E0 before materialization

E0 validates that the preparation accepted its exact `LoadConfiguration`, the descriptor is coherent and has a nonempty observed set, checked final/peak totals do not overflow, every peak ownership component contains the corresponding final component, and cache bytes per token agree.

Given a pre-load reservation `R0`, E0 independently admits both `R0 + loading_peak` and `R0 + final` against its fixed aggregate budget. It reserves `R0 + loading_peak` before calling `load_prepared`.

On success, E0 verifies handle, complete descriptor, actual device, actual execution scalar, final accounted footprint, and lifecycle transition. Only then does it commit a model slot, replace peak reservation with final reservation, and publish a receipt.

On materialization failure, E0 immediately attempts `PreparedLoad::cleanup`:

- success restores `R0` and returns the original load failure;
- failure retains `PendingModelOwner::FailedLoad(prepared)`, the model identity, and the full loading peak;
- the cleanup resource is `FailedLoad { model_id }`;
- primary model-load and cleanup failure classes remain separate;
- the initial failure is attempt one, bounded retry is shared with existing cleanup policy, and exhausted ownership remains quarantined/accounted.

A complete model that fails post-load E0 verification follows the existing explicit unload/quarantine path and conservatively retains the loading peak if unload preparation fails.

This extends ADR-0010's E0 verification boundary. It does not trust the adapter because preparation exists.

### Preserve explicit terminal cleanup policy

Phase 12 introduces no adapter-local hidden `mem::forget`. Failed preparations remain reachable through E0 while retry is possible or exhausted ownership is observable.

ADR-0006 remains authoritative for terminal shutdown: if the finite explicit cleanup budget is exhausted while the complete E0 runtime still owns native resources, the worker may use the named `RetainUntilProcessExit` disposition and forget the complete runtime only after publishing structured terminal failure. Process termination remains the reclamation boundary. This is distinct from ordinary prepared-load retry.

### Keep E1 and Slint narrow

E1 receives lower descriptor/receipt facts but does not reproduce Candle policy.

- `ResolvedModel` exposes immutable identity and optional configuration-declared metadata.
- Public E1 `LoadedModel` exposes the E0-verified execution scalar and actual execution device, but no declaration, observed set, inferred primary, or per-tensor inventory.
- E1 checks declaration agreement across artifact/admission/descriptor evidence, a nonempty observed set within its compact F32/F16/BF16 vocabulary, receipt identity/capabilities/device, and final reserved footprint.
- E1 does not infer primary, choose per-tensor conversion, compare declaration with execution, or fall back.
- Retained lower model ownership is reported as `ModelCleanupPending { exhausted, failure }`; ordinary owner-free failure remains `ModelLoadFailed`.
- Slint may display the optional declaration in a resolved summary and execution scalar/device in a loaded summary. It gains no tensor table, conversion control, or backend responsibility.

This preserves ADR-0013: Candle remains the sole local engine, E0 remains generic/backend-neutral at portable contracts, E1 remains non-generic/private concrete composition, and token-sensitive work remains statically dispatched.

### Persist declaration only

New `LAM1` writes use version 2 and store optional configuration-declared scalar metadata. Exact version 1 reads remain supported and decode their mandatory scalar as a present declaration. Observed sets, inferred primary, execution scalar/device, per-tensor inventory, footprints, and cache paths are not persisted.

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
- Mixed F16/F32 and BF16/F32 layouts are supported only under the exact reviewed set/declaration policy.
- CPU behavior remains F32→F32, F16→F16, and BF16→F32; CUDA policy remains explicit and capability-checked.
- Final ownership is no longer inflated by transient loading headroom; failed cleanup retains the conservative loading peak.
- E0 can retry or exhaust a failed partial load without publishing a model or losing identity/accounting.
- E1 and Slint become simpler: resolved declaration and loaded execution facts are no longer collapsed into one source-scalar label.
- Persistence can represent absent declarations without storing per-tensor runtime evidence.
- Benchmark/evidence observers may copy public plan/receipt facts inward, but production APIs are not expanded for reports.
- Milkdrift remains centered on operator-defined workflows and explicit ownership; Phase 12 hardens one local endpoint and stops at that boundary.

## Review trigger

Review this decision when adding another accepted scalar set, execution dtype, quantized format, model architecture, loading algorithm, asynchronous materialization contract, source-identity mechanism, or cleanup owner; when final/peak formulas change; when E1 has a demonstrated consumer for additional source-layout facts; or when another backend cannot honestly implement prepared loading.

Device selection, default-feature, fallback, and hardware-support changes continue to trigger ADR-0019 review. Terminal process-lifetime cleanup changes trigger ADR-0006 review. Backend substitution changes trigger ADR-0010 review. A second local engine triggers ADR-0013 review rather than silently weakening this transaction.
