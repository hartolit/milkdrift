# Current implementation status

**Status date:** 2026-08-02

```text
Phase 10 repository infrastructure and synthetic acceptance: complete.
External real-product baseline: outstanding.
Phase 11: not active.
```

This page owns current product support, unsupported behavior, lifecycle guarantees, and the existence of benchmark infrastructure. It does not own command logs or timing intervals. Use [performance evidence](performance.md) for methodology/results, [validation](validation.md) for procedures, [execution history](../agent/execution/history.md) for chronology, and the [execution plan](../agent/execution/execution-plan.md) for future work.

## Supported product

| Capability | Current support |
|---|---|
| Local engine | Candle only |
| Artifact source and format | Immutable Hugging Face Hub revision with Safetensors |
| Device | CPU only |
| Resident models | One selected/resident model in E1 |
| Direct completion | Supported for every successfully loaded compatible model |
| Built-in chat | Exact `TinyLlama/TinyLlama-1.1B-Chat-v1.0` profile at commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`, with `</s>` token ID 2 |
| Frontend | Slint desktop through the E1 façade |
| Persistence | redb-backed preferences and model catalogue; conversation history remains in memory |

`ModelSelection` is normalized repository/revision input. Resolution pins an immutable Hub commit, and E1 derives engine, source, format, scalar, device, tokenizer vocabulary, and provenance evidence. Callers cannot assemble unsupported engine/source/format/device cross-products.

The current composition is:

```text
Slint or another native frontend
        -> application-runtime (E1)
             -> one bounded Hub worker
             -> Hugging Face tokenizer/decoder
             -> redb persistence
             -> one Candle hosted E0 worker/thread
                  -> inference-runtime (E0)
                       -> Candle + Safetensors + CPU
```

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
| `runtime-benchmarks` Criterion target | Hosted public-E0 checked-prefill and incremental-decode submission-to-event measurements | Component-like hosted boundary, not raw Candle kernels or E1 latency |
| Stable report support | Versioned JSON with Git/toolchain/host/workload/fixture identity, typed lifecycle/accounting, and sampled Linux process memory | Generated output remains under root `target`; tokens, text, secrets, and broad environment dumps are excluded |

`benchmarks/runtime` is the sole root benchmark package. It is non-publishable, has no build script or incoming dependency, uses the root lockfile/target, and consumes only reviewed public production APIs. Curated methodology and the exact Commit A results are canonical in [performance evidence](performance.md).

`runtime-benchmarks` is currently synthetic-only. External product evidence remains outstanding; [performance evidence](performance.md#external-product-evidence) owns the complete status and limitations.

## Unsupported and deferred behavior

- GGUF and other quantized model formats are unsupported.
- GPU execution, device selection, and device-memory evidence are unsupported.
- Hosted-provider, peer, remote/browser transport, and multi-model residency are not implemented.
- Chat compatibility is not generalized beyond the exact reviewed TinyLlama profile.
- Conversation persistence and arbitrary branch trees are not implemented.
- Slint does not expose a generation-settings panel.
- Strict allocation freedom is not claimed for Candle or Hugging Face tokenization/decoding.
- Synthetic fixture timings do not establish language quality, representative vocabulary/context scale, production steady-state throughput, or product-model performance.
- Process RSS is sampled process-wide host memory, not ownership proof, allocator-event counting, native-resource attribution, or device memory.

## Acceptance state

The accepted code-under-test is Commit A `efcd36e320a97d61d3f982619fee182410c514df`, tree `f80c5d6c746376df81d7ac8e7281ac9736e44d88`. It has clean local exact-tree repository acceptance and controlled download-free measurements recorded in [history](../agent/execution/history.md#phase-10--repository-infrastructure-and-synthetic-acceptance) and [performance evidence](performance.md). Local evidence is not a claim that a remote GitHub CI run passed.

External product evidence remains outstanding. The historical E1 Hub smoke proves an older pinned correctness path only; it is not current product-performance evidence. A future baseline must actually run an exact model/revision through a narrow authorized opt-in path before Phase 11 is activated.

## Historical context

The [recovered implementation plan](implementation-plan.md) is non-authoritative historical source material. Phase 8’s dual-product experiment and earlier phase claims remain historical; current support follows accepted ADRs and this page.
