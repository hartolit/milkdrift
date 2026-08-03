# Current implementation status

**Status date:** 2026-08-03

```text
Phase 10: complete.
External CPU product baseline: complete.
Phase 11: active; E1/Slint explicit CUDA selection implemented, external product evidence outstanding.
```

This page owns current product support, unsupported behavior, lifecycle guarantees, and the existence of benchmark infrastructure. It does not own command logs or timing intervals. Use [performance evidence](performance.md) for methodology/results, [validation](validation.md) for procedures, [execution history](../agent/execution/history.md) for chronology, and the [execution plan](../agent/execution/execution-plan.md) for future work.

## Supported product

| Capability | Current support |
|---|---|
| Local engine | Candle only |
| Artifact source and format | Immutable Hugging Face Hub revision with Safetensors |
| Device | CPU by default; explicit ordinal CUDA when built through the reviewed non-default feature chain |
| Resident models | One selected/resident model in E1 |
| Direct completion | Supported for every successfully loaded compatible model |
| Built-in chat | Exact `TinyLlama/TinyLlama-1.1B-Chat-v1.0` profile at commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`, with `</s>` token ID 2 |
| Frontend | Slint desktop through the E1 façade |
| Persistence | redb-backed preferences and model catalogue; conversation history remains in memory |

`ModelSelection` remains normalized repository/revision input. Resolution pins an immutable Hub commit and reports artifacts, source, format, scalar, tokenizer, identity, and compatibility without selecting a device. E1 holds selected `ApplicationDevice` state separately, and `LoadedModel` reports only the actual device verified from E0's receipt. Callers cannot assemble unsupported engine/source/format/device cross-products.

The current composition is:

```text
Slint or another native frontend
        -> application-runtime (E1)
             -> one bounded Hub worker
             -> Hugging Face tokenizer/decoder
             -> redb persistence
             -> one Candle hosted E0 worker/thread
                  -> inference-runtime (E0)
                       -> Candle + Safetensors + selected CPU or feature-gated CUDA
```

### Phase 11 implemented boundary

The lower Candle/E0 CUDA path and the E1/Slint selection follow-up are implemented:

- CPU device ID 0 remains mandatory, in every build, and the fresh-install selection;
- the exact opt-in path is `desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`; the separate `inference-runtime/cuda -> candle-backend/cuda` edge is development-only;
- no default feature graph reaches CUDA, there is no generic `gpu` alias, and `cudnn`, `flash-attn`, and `nccl` are not enabled;
- E1 owns `ApplicationDevice::{Cpu, Cuda { ordinal: u32 }}`, bounded discovery, structured availability diagnostics, persistence, selection lifecycle, and explicit accelerator-memory policy without exposing Candle or `cudarc` types;
- resolution remains device-independent, while load admission passes the exact selected `ExecutionDevice` and publishes only a receipt-verified actual device;
- persisted unavailable CUDA remains selected and visible, load fails explicitly after re-probing, and no CPU fallback occurs;
- Slint uses stable Rust identity/index mapping for its compact device selector and distinguishes selected, artifact-only resolved, and actual loaded-device summaries;
- E0 still verifies the actual loaded device and bounded footprint before publishing a receipt or resident slot.

The E1 memory policy is `AcceleratorMemoryPolicy::{Automatic, Limit { bytes: NonZeroU64 }}`. Because E0's aggregate device budget is fixed at startup, Automatic admission uses the least reported physical total across every CUDA row in the bounded startup catalogue; an unavailable row or missing capacity contributes zero and fails closed, while a limit applies the lower user cap. Load re-probes block without fallback when that fixed nonzero budget no longer fits the selected device's latest physical total, requiring restart before CUDA load. Existing CPU host budgeting is unchanged, and selected-device Candle planning checks current available VRAM before partial residency. One resident model remains the product limit.

Previously recorded lower-layer fixture execution remains accepted lower-layer evidence. During this implementation work, the focused CUDA feature matrix and release desktop build passed locally, and the ignored E1 fixture discovered CUDA 0 on the RTX 5070 Ti, selected it without loading, then loaded/unloaded the committed fixture with matching selected and actual device identity. These results are development evidence on the current uncommitted tree, not clean committed-tree acceptance. The desktop process launched against an isolated fresh database, but the locked graphical session prevented visual selector interaction. External/product-model CUDA evidence and measurements remain outstanding, so Phase 11 is not complete.

## Runtime and lifecycle guarantees

`application-runtime` is the public frontend-neutral, non-generic E1 façade. E0 exclusively owns loaded model resources, sequences, request admission, generation workspaces, token scheduling and sampling, cancellation boundaries, output backpressure, cleanup quarantine, accounting, unload, and terminal shutdown.

Current lifecycle guarantees include:

- startup rollback retains and boundedly reaps partially created worker ownership;
- incompatible-model cleanup remains privately owned and accounted through success, proven disconnection, or observable bounded exhaustion;
- shutdown distinguishes running, stopping, cleanly stopped, retryable failure, and terminal failure;
- unresolved join handles remain owned after timeout so later shutdown can retry;
- E0 cleanup exhaustion is terminal and retains the runtime allocation until process exit rather than invoking unverified implicit backend destruction;
- E1 preserves terminal E0 cleanup failure independently of handle state;
- endpoint disconnection alone does not prove clean shutdown;
- successful request release restores model-only accounting, and successful unload restores empty accounting.

These guarantees are exercised by deterministic tests; exact acceptance provenance is in [execution history](../agent/execution/history.md).

## Benchmark and evidence infrastructure

Benchmark infrastructure exists without changing production support or public APIs:

| Surface | Current capability | Evidence boundary |
|---|---|---|
| `sampling` Criterion target | Public sampler matrix with separate `sample_only` and `restore_and_sample` boundaries, eight policy/history cases at three vocabulary sizes, and three stop-matching cases | Component regression only; the ordinary matrix test executes every case once for correctness |
| `domain-contracts` allocation target | Harness-free executable with isolated prefill/decode allocator regions | Deterministic project-allocation gate, not native/device allocation attribution |
| `runtime-benchmarks` synthetic runner | Bounded download-free hosted-E0 lifecycle/output/accounting/RSS cycles plus fresh E1 start/shutdown cycles | Synthetic integration evidence, not product-model performance or quality |
| `runtime-benchmarks` external runner | Sole explicit-network public-E1 path for the exact supported TinyLlama revision; compatible chat, controlled direct completion, release, unload, shutdown, timing, and process RSS | One current local CPU product baseline, not model quality, general serving capacity, another model/scalar/device, or a threshold |
| `runtime-benchmarks` Criterion target | Hosted public-E0 checked-prefill and incremental-decode submission-to-event measurements | Component-like hosted boundary, not raw Candle kernels or E1 latency |
| Stable report support | Separate versioned synthetic and external JSON with Git/toolchain/host/workload/model identity, typed lifecycle/accounting, and sampled Linux process memory | Generated output remains under root `target`; tokens, text, secrets, and broad environment dumps are excluded |

`benchmarks/runtime` is the sole root benchmark package. It is non-publishable, has no build script or incoming dependency, uses the root lockfile/target, and consumes only reviewed public production APIs. Its normal baseline remains download-free; the separate external binary requires explicit network authorization and a canonical explicit cache. Curated methodology and exact synthetic/external results are canonical in [performance evidence](performance.md).

A current exact external CPU product baseline exists for the supported TinyLlama revision. CPU remains the fresh-install default, while E1 and Slint now support explicit CUDA selection in CUDA-enabled builds. Lower-layer CUDA fixture evidence exists, but no external CUDA product baseline or product-model CUDA measurements exist.

## Unsupported and deferred behavior

- GGUF and other quantized model formats are unsupported.
- External/product-model CUDA evidence and measurements remain outstanding. Metal, `cudnn`, `flash-attn`, `nccl`, and multi-GPU execution are unsupported; there is no generic `gpu` feature alias or automatic CPU fallback.
- Hosted-provider, peer, remote/browser transport, and multi-model residency are not implemented.
- Chat compatibility is not generalized beyond the exact reviewed TinyLlama profile.
- Conversation persistence and arbitrary branch trees are not implemented.
- Slint does not expose a generation-settings panel.
- Strict allocation freedom is not claimed for Candle or Hugging Face tokenization/decoding.
- Synthetic fixture timings do not establish language quality, representative vocabulary/context scale, production steady-state throughput, or product-model performance.
- Process RSS is sampled process-wide host memory, not ownership proof, allocator-event counting, native-resource attribution, or device memory.

## Acceptance state

Synthetic acceptance remains attributable to Commit A `efcd36e320a97d61d3f982619fee182410c514df`, tree `f80c5d6c746376df81d7ac8e7281ac9736e44d88`. The external CPU baseline executed on clean Commit C `771c0de4d72565a6302ca60f3b6bafd8c807962b`, tree `3d5b6ccc5ecc959de7cb370c1147f76e4cd32e3f`. Exact local evidence is recorded in [history](../agent/execution/history.md) and [performance evidence](performance.md). Local evidence is not a claim that a remote GitHub Actions run passed.

Phase 10 is complete. Phase 11 is active: lower Candle/E0 CUDA execution and explicit E1/Slint CPU/CUDA selection are implemented. Focused local CUDA compilation and the ignored E1 committed-fixture smoke passed on the development tree; clean committed-tree product-model evidence and measurements remain outstanding. Phase 11 is not complete.

## Historical context

The [recovered implementation plan](implementation-plan.md) is non-authoritative historical source material. Phase 8’s dual-product experiment and earlier phase claims remain historical; current support follows accepted ADRs and this page.
