# Current implementation status

**Status date:** 2026-08-04

```text
Phase 10: complete.
External CPU product baseline: complete.
Phase 11 complete for the executed CPU + Linux CUDA matrix.
No subsequent phase is active.
```

This page owns current product support, unsupported behavior, lifecycle guarantees, and the existence of benchmark infrastructure. It does not own command logs or timing intervals. Use [performance evidence](performance.md) for methodology/results, [validation](validation.md) for procedures, [execution history](../agent/execution/history.md) for chronology, and the [execution plan](../agent/execution/execution-plan.md) for future work.

## Supported product

| Capability | Current support |
|---|---|
| Local engine | Candle only |
| Artifact source and format | Immutable Hugging Face Hub revision with Safetensors |
| Device | Mandatory default CPU; explicit CUDA ordinal 0 only on the executed Linux x86_64 NVIDIA GeForce RTX 5070 Ti matrix |
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
                       -> Candle + Safetensors + mandatory CPU or explicitly selected, feature-gated CUDA ordinal 0
```

### Phase 11 implemented boundary

Phase 11 is complete only for the following executed support matrix:

| Execution target | Support and evidence boundary |
|---|---|
| CPU | Mandatory in every build, the default feature graph and fresh-install selection, and the shared-CI path. CPU tests and the final CPU compile/test/Clippy gates were executed successfully. |
| CUDA ordinal 0 | Local exact-feature compilation and executed hardware evidence support only the Linux x86_64 matrix: NVIDIA GeForce RTX 5070 Ti, driver 610.43.03, CUDA toolkit 13.3, compute capability 12.0, and build target 120. |
| Other NVIDIA/CUDA or GPU targets | Unsupported and unclaimed. The exact executed row does not establish generic NVIDIA compatibility. |
| Metal | Not implemented. |

The implemented boundary is:

- the product feature graph is exactly `desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`;
- the benchmark feature graph is exactly `runtime-benchmarks/cuda -> application-runtime/cuda`;
- the direct E0 test edge `inference-runtime/cuda -> candle-backend/cuda` remains development-only;
- no default feature graph reaches CUDA, there is no generic `gpu` alias, and `cudnn`, `flash-attn`, and `nccl` are not enabled;
- E1 owns `ApplicationDevice::{Cpu, Cuda { ordinal: u32 }}`, bounded discovery, structured availability diagnostics, persisted explicit selection, selection lifecycle, and explicit accelerator-memory policy without exposing Candle or `cudarc` types;
- resolution remains device-independent, while load admission passes the exact selected `ExecutionDevice`; an unavailable persisted CUDA selection fails explicitly after re-probing and never falls back to CPU;
- E0 verifies the actual loaded device and bounded footprint before publishing a receipt, and that actual device identity reaches E1 and Slint;
- Slint uses stable Rust identity/index mapping for its compact device selector and distinguishes selected, artifact-only resolved, and actual loaded-device summaries;
- sampling remains host-side over F32 logits after CUDA transfer; GPU-side sampling is not supported.

The E1 memory policy is `AcceleratorMemoryPolicy::{Automatic, Limit { bytes: NonZeroU64 }}`. Because E0's aggregate device budget is fixed at startup, Automatic admission uses the least reported physical total across every CUDA row in the bounded startup catalogue; an unavailable row or missing capacity contributes zero and fails closed, while a limit applies the lower user cap. Load re-probes block without fallback when that fixed nonzero budget no longer fits the selected device's latest physical total, requiring restart before CUDA load. Existing CPU host budgeting is unchanged, and selected-device Candle planning checks current available VRAM before partial residency. One resident model remains the product limit.

Clean Commit E `411945e0fd53363f98609db21a43d757c4d9b506`, tree `7099dcb5c9879190543d3afa5fde399a84d799df`, supplied the Phase 11 closure evidence:

- the exact supported TinyLlama primary workload passed on CPU and CUDA, including compatible chat, controlled completion, cancellation, release, unload, and bounded shutdown;
- three complete CUDA lifecycle cycles were stable;
- the direct E0 CUDA snapshot test proved zero model, request, workspace, and cleanup accounting after lifecycle cleanup;
- the CUDA adapter tests and E1 CUDA tests passed;
- schema-2 chat timing is now recorded in the external evidence reports;
- the final CPU and CUDA compile, test, and Clippy gates passed;
- raw reports remained beneath ignored root `target/`.

The user accepted the manual Slint run: CPU and CUDA both worked, CUDA output was visibly near instant, and no interaction issue was observed. No screenshots were recorded or claimed. GitHub Actions acceptance remains a separate post-push fact and is not claimed until an observed run is recorded.

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
| `runtime-benchmarks` external runner | Sole explicit-network public-E1 path for the exact supported TinyLlama revision, with mandatory `cpu` or `cuda:0` selection; compatible chat, controlled direct completion, cancellation, release, unload, shutdown, timing, process RSS, and CUDA device observations | Executed CPU and exact Linux CUDA product evidence only; not model quality, general serving capacity, another model/scalar/device, generic NVIDIA compatibility, or a threshold |
| `runtime-benchmarks` Criterion target | Hosted public-E0 checked-prefill and incremental-decode submission-to-event measurements | Component-like hosted boundary, not raw Candle kernels or E1 latency |
| Stable report support | Versioned synthetic JSON owns direct E0 accounting; CPU/CUDA external JSON owns Git/toolchain/host/workload/model/device identity, qualified E1 lifecycle and load-contract evidence, sampled Linux process memory, and bounded CUDA driver observations | Generated output remains under root `target`; tokens, text, secrets, and broad environment dumps are excluded |

`benchmarks/runtime` is the sole root benchmark package. It is non-publishable, has no build script or incoming dependency, uses the root lockfile/target, and consumes only reviewed public production APIs. Its normal baseline remains download-free; the separate external CPU/CUDA binary requires explicit network authorization, explicit device selection, and a canonical explicit cache. Curated methodology and exact synthetic/external results are canonical in [performance evidence](performance.md).

Current exact external product evidence exists for the supported TinyLlama revision on CPU and on CUDA ordinal 0 for the exact executed Linux matrix. CPU remains mandatory and the fresh-install default; CUDA selection is explicit and persisted, and unavailable CUDA fails without fallback. This does not broaden support to another CUDA ordinal, NVIDIA device, operating system, model, format, or engine.

## Unsupported and deferred behavior

- Metal is unsupported.
- CUDA support is limited to the exact executed Linux x86_64 ordinal-0 matrix above. There is no generic NVIDIA compatibility claim, generic `gpu` alias, automatic CPU fallback, cuDNN (`cudnn`), flash-attention (`flash-attn`), multi-GPU, or `nccl` support.
- GGUF and other quantized model formats are unsupported.
- GPU-side sampling is unsupported; sampling remains host-side over transferred F32 logits.
- Another local engine, hosted-provider execution, peer execution, and remote/browser transport are not implemented.
- Multi-model residency is unsupported; one selected/resident model remains the product limit.
- Chat compatibility is not generalized beyond the exact reviewed TinyLlama profile.
- Conversation persistence and arbitrary branch trees are not implemented.
- Slint does not expose a generation-settings panel.
- Strict allocation freedom is not claimed for Candle or Hugging Face tokenization/decoding.
- Synthetic fixture timings do not establish language quality, representative vocabulary/context scale, production steady-state throughput, or product-model performance.
- Process RSS and whole-device CUDA observations are not ownership proof, allocator-event counting, native-resource attribution, or process-attributed device memory.

## Acceptance state

Synthetic acceptance remains attributable to Commit A `efcd36e320a97d61d3f982619fee182410c514df`, tree `f80c5d6c746376df81d7ac8e7281ac9736e44d88`. The Phase 10 external CPU baseline executed on clean Commit C `771c0de4d72565a6302ca60f3b6bafd8c807962b`, tree `3d5b6ccc5ecc959de7cb370c1147f76e4cd32e3f`. Phase 11 CPU + Linux CUDA acceptance executed on clean Commit E `411945e0fd53363f98609db21a43d757c4d9b506`, tree `7099dcb5c9879190543d3afa5fde399a84d799df`. Exact local evidence is recorded in [history](../agent/execution/history.md) and [performance evidence](performance.md).

Phase 11 is complete only for the executed CPU and exact Linux x86_64 CUDA ordinal-0 matrix stated above. No subsequent phase is active. GitHub Actions acceptance remains a separate post-push fact and is not claimed until an observed run is recorded.

## Historical context

The [recovered implementation plan](implementation-plan.md) is non-authoritative historical source material. Phase 8’s dual-product experiment and earlier phase claims remain historical; current support follows accepted ADRs and this page.
