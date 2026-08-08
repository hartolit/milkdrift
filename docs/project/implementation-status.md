# Current implementation status

**Status date:** 2026-08-08

```text
Phase 10 complete.
Phase 11 complete for its historically executed CPU + Linux CUDA matrix.
Post-Phase 11 quality closure complete.
Phase 12 complete for deterministic CPU compatibility and the exact locally
executed Linux CUDA matrix.
Phase 12 canonical clean-target, portability, dependency, link, CUDA compile,
and local hardware gates passed.
The Phase 12 GitHub self-hosted workflow remains unrun, and external
mixed-checkpoint evidence remains absent.
No subsequent product phase is active.
```

This page is the sole product-level support matrix and validation-state owner. It separates implementation, focused working-tree validation, clean-tree acceptance, compile-only coverage, and hardware execution. It does not turn earlier evidence into evidence for later code. Repeatable procedures belong in [validation](validation.md), measurements in [performance evidence](performance.md), and historical chronology in [execution history](../agent/execution/history.md).

Milkdrift remains workflow-first: the local Candle endpoint is one lifecycle-safe execution target for future operator-defined workflows, not the project identity. The current `application-runtime` and Slint desktop are a reference application kit and thin host, not the future general workflow API or control plane.

## Phase 12 implementation baseline

Phase 12 production work is split across:

- commit `58490fe693fef7a2635956181088664cd90685e8`, which introduced exact prepared loading, per-tensor Safetensors inspection and conversion, conversion-aware final/loading-peak accounting, and retained partial-load cleanup through E0;
- commit `12510695aa29be6a2665dbf3777cccbb8172c2d1`, which integrated optional configuration metadata, E1 receipt semantics, persistence versioning, cleanup events, and thin Slint presentation;
- this closure segment, which broadens deterministic mixed-layout adapter/E0 coverage, updates the inward benchmark observer and guarded CUDA workflow, and reconciles canonical evidence owners.

On 2026-08-08, the Phase 12 closure tree passed the focused download-free CPU suites, the canonical gate from a previously absent Cargo target directory, both portable-domain target matrices, locked dependency policy, offline Markdown links, the exact CUDA compile chain, and the exact local deterministic hardware matrix. The hardware run was local rather than GitHub Actions. No external mixed-dtype checkpoint has been accepted.

## Supported product

| Capability | Current implementation and evidence boundary |
|---|---|
| Project identity | Operator-defined workflows, explicit context/authority/resource ownership, and replaceable execution targets; the current local inference application is the implemented foundation. |
| Local engine | Candle only. |
| Artifact source | Hugging Face Hub revision resolved to an immutable commit. |
| Model format and architecture | Unquantized Safetensors through the current Llama compatibility path. |
| Scalar layouts | Exactly `{F32}`, `{F16}`, `{F16,F32}`, `{BF16}`, and `{BF16,F32}` under the declaration rules below. |
| CPU | Mandatory in every build and the fresh-install/default selection. Phase 12 focused suites and the canonical clean-target gate passed locally on the closure tree. |
| CUDA | Non-default explicit ordinal 0 remains the only product CUDA identity. The exact Phase 12 compile chain and deterministic fixture hardware matrix passed locally on the executed RTX 5070 Ti row. The Phase 12 GitHub self-hosted workflow has not run; no generic NVIDIA or external mixed-checkpoint claim is made. |
| Device failure | Explicit failure with no automatic CPU fallback. |
| Resident models | One selected/resident model in E1. |
| Direct completion | Available for every successfully loaded compatible model. |
| Built-in chat | Exact `TinyLlama/TinyLlama-1.1B-Chat-v1.0` profile at commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`, with `</s>` token ID 2. Loading another compatible model does not imply chat support. |
| Frontend | Thin Slint desktop through the E1 façade. |
| Persistence | redb-backed preferences and model catalogue; conversation history remains in memory. |

The complete current composition remains:

```text
Slint or another native host
        -> application-runtime (E1 reference application kit)
             -> one bounded Hub worker
             -> Hugging Face tokenizer/decoder
             -> redb persistence
             -> one Candle hosted E0 worker/thread
                  -> inference-runtime (E0)
                       -> Candle + unquantized Llama Safetensors
                       -> mandatory/default CPU
                          or explicit feature-gated CUDA ordinal 0
```

## Scalar meanings

Phase 12 uses four separate scalar facts:

1. **Configuration-declared optional scalar** is recognized `dtype` or legacy `torch_dtype` metadata from immutable model configuration. It is producer-intent evidence only. `None` means no recognized declaration was retained; it does not make the artifact unloadable.
2. **Observed tensor scalar set** is the fixed-size `ScalarTypeSet` built from every selected Safetensors tensor header across every shard. It records categories, not tensor names, counts, or homogeneity claims.
3. **Inferred primary scalar** is an adapter-private compatibility result derived from the exact observed set. It is F32 for `{F32}`, F16 for `{F16}` or `{F16,F32}`, and BF16 for `{BF16}` or `{BF16,F32}`. When a recognized declaration is present, it must equal this inferred primary; an unsupported or contradictory present declaration is rejected.
4. **Execution scalar** is selected during exact device-aware preparation and then verified from the loaded backend model by E0. It may differ from both individual serialized tensor dtypes and the configuration declaration.

These facts must not be collapsed into a singular “source scalar.” Detailed tensor names, shapes, offsets, per-tensor dtypes, and payload digests remain private to `candle-backend`.

## Exact Phase 12 scalar-layout policy

| Observed tensor scalar set | Inferred primary | Permitted recognized declaration | CPU execution | Supported CUDA planner policy |
|---|---|---|---|---|
| `{F32}` | F32 | `None` or F32 | F32 | F32 |
| `{F16}` | F16 | `None` or F16 | F16 | F16 |
| `{F16,F32}` | F16 | `None` or F16 | F16 | F16 |
| `{BF16}` | BF16 | `None` or BF16 | F32 | BF16 when the selected device reports support; otherwise planning fails |
| `{BF16,F32}` | BF16 | `None` or BF16 | F32 | BF16 when the selected device reports support; otherwise planning fails |

The adapter rejects every other observed set, including any F16+BF16 combination, an empty set, FP8, integer, boolean, unknown, or quantized tensor types, and any non-`None` declaration that is unsupported or contradicts the inferred primary. GGUF and other quantized formats remain unsupported.

Final vocabulary logits are returned to E0 as caller-owned host F32 for every accepted execution mapping. Sampling remains host-side.

## Prepared loading and memory truth

`ModelLoader::prepare_load` creates one opaque, source/configuration/device-bound `PreparedLoad` and exposes its exact `LoadPlan`. `ModelLoader::load_prepared` consumes that same preparation without replanning. Before device initialization, Candle validates all selected shard headers, tensor dtypes, duplicate names, shapes, offsets, bounds, required Llama tensors, and checked byte arithmetic.

The Candle preparation retains open shard file handles, inspected lengths, tensor ranges, and per-tensor SHA-256 payload digests. Materialization reads from those retained handles rather than reopening paths, rechecks file length, and verifies every payload digest. Deleting or replacing a path therefore cannot redirect an accepted preparation; mutation through the retained inode is detected before the changed tensor is accepted.

Two deterministic footprint phases are distinct:

- **Final footprint** (`LoadPlan::expected_footprint`) is exact post-load required execution-tensor ownership plus cache bytes per token. It is the quantity transferred to the loaded model, verified by E0, and published as the successful receipt/snapshot reserved footprint.
- **Loading-peak footprint** (`LoadPlan::loading_peak_footprint`) is the exact component-wise deterministic peak of the selected per-tensor loading algorithm, including aligned source staging, source tensors, cast tensors, host-to-device transfer staging, and temporary ownership of supported extra tensors. It is admission-phase headroom, not post-load residency, process RSS, allocator/driver overhead, or whole-device memory.

The exact formulas are canonical in [Candle backend](candle-backend.md). E0 admits both the peak and final quantities against its aggregate budget, reserves the peak before materialization, and commits only the final quantity after complete model verification.

## Load failure and cleanup ownership

A failed materialization returns `FailedLoad<PreparedLoad>`: the primary `LoadError` and the sole owner of every completed or pending tensor/device resource. E0 immediately attempts `PreparedLoad::cleanup`.

- Cleanup success restores the pre-load reservation and returns the original load failure.
- Cleanup failure moves the preparation into E0 pending-model cleanup as `CleanupResource::FailedLoad`, retains the complete loading-peak footprint, and reports primary model-load failure separately from failed-load cleanup failure.
- The initial failed cleanup is attempt one. `poll_cleanup` performs at most one additional retained operation, uses the configured finite total-attempt budget (three by default), releases ownership/accounting exactly once on success, and leaves exhausted ownership quarantined and accounted.
- A complete loaded model that fails E0 post-load verification follows the same no-publication rule and retains the conservative loading-peak quantity if explicit unload preparation fails.

E1 publishes no resident `LoadedModel` for any such failure. `ApplicationEvent::ModelCleanupPending { exhausted, failure }` distinguishes retained cleanup from an ordinary owner-free `ModelLoadFailed`; application activity remains unloading and device selection remains locked until a private E0 snapshot proves zero aggregate ownership, disconnection/confirmed worker stop resolves ownership, or exhaustion remains explicit.

These semantics extend rather than replace [ADR-0006](../agent/decisions/0006-explicit-bounded-shutdown.md): terminal shutdown exhaustion still uses `RetainUntilProcessExit`, and process termination remains the final reclamation boundary for deliberately retained native ownership.

## E1, persistence, and Slint boundary

`ResolvedModel` remains device-independent and exposes immutable identity, tokenizer/compatibility facts, and the optional configuration-declared scalar. It does not expose an observed tensor set, inferred primary, execution scalar, or actual device.

Public E1 `LoadedModel` exposes the receipt-verified execution scalar and execution device, but no configuration/source scalar or observed tensor inventory. E1 validates the optional declaration across resolution/admission/descriptor evidence and accepts only a nonempty observed set within the compact F32/F16/BF16 application vocabulary. It does not compare declaration to individual tensors, infer the primary scalar, choose conversions, or reproduce Candle's device policy.

`LAM1` catalogue writes are version 2 and store the optional configuration declaration only. Exact version 1 records remain readable, with their mandatory scalar decoded in memory as `Some(...)`. Observed layouts, inferred primary, execution scalar/device, cache paths, and per-tensor details are not persisted. `LAS1` settings remain version 2 writes/version 1 reads under the existing device and accelerator-memory policy.

Slint remains a thin reference host. Resolved summaries may show the optional configuration declaration; loaded summaries show only verified execution scalar and execution device. Labels are presentation, never parsed identity, and the frontend owns no model-loading or fallback policy.

## Validation and evidence state

| Evidence class | Phase 12 state on 2026-08-08 |
|---|---|
| Production implementation | Present in commits `58490fe`, `1251069`, and this closure commit. |
| Focused/download-free CPU tests | Passed: 20 adapter, 3 hosted native-E0, 32 E0 fault-injection, and 78 benchmark tests. |
| Local Phase 12 CPU environment | Linux 7.1.5-arch1-2 x86_64, AMD Ryzen 9 5950X (16 cores/32 threads), Rust 1.96.1, and Cargo 1.96.1. |
| Canonical clean-target `cargo xtask verify` gate | Passed locally from a previously absent Cargo target directory on 2026-08-08. |
| Portable domain checks | `wasm32-unknown-unknown` and `thumbv7em-none-eabihf` passed for all five domain crates. |
| Dependency and link policy | Locked cargo-deny policy passed; offline Lychee checked 276 links with 0 errors. |
| Phase 12 CUDA compile graph | Passed with `CUDA_COMPUTE_CAP=120`. Compilation remains distinct from hardware evidence. |
| Local Phase 12 CUDA hardware execution | Passed on NVIDIA GeForce RTX 5070 Ti ordinal 0, driver/KMD 610.43.03, CUDA UMD/toolkit 13.3, `nvcc` 13.3.73, compute capability 12.0, and build cap 120. |
| Phase 12 GitHub self-hosted workflow | Not run; no remote workflow provenance is claimed. |
| External mixed-dtype checkpoint | Absent. No immutable, license-reviewed external mixed checkpoint has been accepted or executed. |
| Benchmark/evidence additions | Present as an inward observer of public production APIs; no new performance measurement or external product report is claimed. |

### Historical Phase 11 evidence

The accepted pre-Phase 12 hardware-executed source baseline remains commit `1a62d2ed6623500e9052b4b8386ebd058984bd89`, tree `79864da274aed94471c2fbcfedaa97c2f32f3e7a`.

- shared-CPU [quality run 30942153370](https://github.com/hartolit/milkdrift/actions/runs/30942153370) passed on that baseline;
- self-hosted [CUDA hardware run 30942148369](https://github.com/hartolit/milkdrift/actions/runs/30942148369) passed on that baseline for Linux x86_64, NVIDIA GeForce RTX 5070 Ti, driver 610.43.03, CUDA Toolkit 13.3, compute capability 12.0, ordinal 0, and `CUDA_COMPUTE_CAP=120`.

That evidence remains historical proof of the Phase 11 device path only. It is not evidence that the Phase 12 per-tensor implementation, mixed layouts, new peak accounting, or retained prepared-load cleanup executed on CUDA hardware.

Historical external TinyLlama CPU/CUDA evidence remains attributed to clean Commit E `411945e0fd53363f98609db21a43d757c4d9b506`, tree `7099dcb5c9879190543d3afa5fde399a84d799df`. TinyLlama at the pinned revision is homogeneous BF16, not an external mixed-dtype checkpoint, and cannot close the Phase 12 external evidence gap.

## Evidence infrastructure

`runtime-benchmarks` remains the sole non-production cross-crate observer. No production, tooling, test, or application package depends on it. The closure tree updates its synthetic and external schemas to distinguish optional configuration declaration, observed scalar set, planned execution scalar/device, exact final footprint, loading peak, actual receipt facts where public, process RSS, and whole-device CUDA observations. It uses an unmaterialized observer `prepare_load`, copies the public plan, and drops the preparation without turning benchmark needs into a new production API.

Historical reports retain their original schema meanings and measurements. No Phase 12 performance result or external checkpoint/product report is inferred merely because observer or workflow definitions changed. The separately executed local CUDA compile and hardware-correctness results are recorded above.

## Unsupported behavior

- Arbitrary mixed-dtype layouts, any F16+BF16 set, FP8/integer/boolean/unknown tensors, quantized weights, GGUF, and non-Llama model architectures are unsupported.
- CUDA outside the exact locally executed Phase 12 RTX 5070 Ti matrix, generic NVIDIA compatibility, generic `gpu`, automatic CPU fallback, cuDNN, flash attention, multi-GPU, NCCL, and GPU-side sampling are unsupported.
- Metal is not implemented.
- Another local engine, hosted-provider execution, peer execution, general workflow execution, plugin execution, and remote/browser transport are not current product paths.
- Multi-model E1 residency is unsupported.
- Chat compatibility is not generalized beyond the exact reviewed TinyLlama profile.
- Conversation persistence, arbitrary branch trees, and a Slint generation-settings panel are not implemented.
- Strict allocation freedom is not claimed for Candle or Hugging Face tokenization/decoding.
- Synthetic fixtures establish deterministic compatibility and lifecycle behavior, not language quality, representative scale, production throughput, or external-checkpoint compatibility.

## Current closure boundary

Phase 12 is complete for the deterministic compatibility claim and the exact locally executed CPU/CUDA matrix. The GitHub self-hosted Phase 12 workflow remains unexecuted remote evidence, and external mixed-checkpoint evidence remains explicitly absent rather than being replaced with homogeneous TinyLlama or a fabricated claim.

The next major architectural direction returns to workflow/workspace/authority foundations. Phase 12 strengthens one local endpoint and does not authorize indefinite expansion of the Candle loader or promotion of the current application kit into the workflow core.
