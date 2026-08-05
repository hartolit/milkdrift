# Current implementation status

**Status date:** 2026-08-05

```text
Phase 10 complete.
Phase 11 complete for the executed CPU + Linux CUDA matrix.
Post-Phase 11 quality closure complete.
No subsequent product phase is active.
```

This page is the sole product-level support matrix and validation-state owner. It does not own command logs or timing tables. Use [validation](validation.md) for repeatable procedures, [performance evidence](performance.md) for exact measurements and their limits, [execution history](../agent/execution/history.md) for chronology, and the [execution plan](../agent/execution/execution-plan.md) for the completed program and inactive future tracks.

## Accepted hardware-executed baseline

The accepted hardware-executed source baseline is commit `1a62d2ed6623500e9052b4b8386ebd058984bd89`, tree `79864da274aed94471c2fbcfedaa97c2f32f3e7a`. It contains the final source scalar and execution scalar APIs, E1 modularization, schema-3 evidence refactor, and the CUDA job whose execution behavior remains current. Subsequent unnumbered maintenance through the current tree reconciles documentation and broadens the workflow's source path coverage without changing the job body or supported runtime behavior.

Two successful GitHub Actions runs were observed on that exact commit:

- normal shared-CPU [quality run 30942153370](https://github.com/hartolit/milkdrift/actions/runs/30942153370), including the canonical gate, portable-domain checks, dependency policy, and offline local-link validation;
- self-hosted [CUDA hardware run 30942148369](https://github.com/hartolit/milkdrift/actions/runs/30942148369), including the exact CUDA feature graph and download-free adapter, E0, and E1 hardware tests.

The intervening post-Phase 11 documentation closure changed no executable source, manifest, lockfile, fixture, or workflow. Its local closure gates and resulting identity belong in the final closure report rather than a self-referential tracked hash.

## Supported product

| Capability | Current support |
|---|---|
| Local engine | Candle only |
| Artifact source | Hugging Face Hub revision resolved to an immutable commit |
| Model format and path | Safetensors; current unquantized Llama path |
| CPU | Mandatory in every build, default feature graph, and fresh-install selection |
| CUDA | Explicit non-default ordinal 0 only on the executed Linux x86_64 NVIDIA GeForce RTX 5070 Ti matrix below |
| Device failure | Explicit failure with no automatic CPU fallback |
| Resident models | One selected/resident model in E1 |
| Direct completion | Supported for every successfully loaded compatible model |
| Built-in chat | Exact `TinyLlama/TinyLlama-1.1B-Chat-v1.0` profile at commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`, with `</s>` token ID 2 |
| Frontend | Slint desktop through the E1 façade |
| Persistence | redb-backed preferences and model catalogue; conversation history remains in memory |

`ModelSelection` contains only normalized repository/revision input. Resolution pins an immutable Hub commit and reports source evidence without selecting a device. E1 holds `ApplicationDevice` selection separately. A resolved model exposes a configuration-declared source scalar; a loaded model exposes that source metadata after loader compatibility validation, the receipt-verified execution scalar, and the actual device.

The current TinyLlama scalar boundary is:

```text
BF16 configuration-declared source scalar on CPU
    -> F32 execution

BF16 configuration-declared source scalar on supported CUDA
    -> BF16 execution
```

A loaded CPU model with a BF16 configuration-declared source scalar is therefore not reported simply as “BF16.”

The current composition is:

```text
Slint or another native frontend
        -> application-runtime (E1)
             -> one bounded Hub worker
             -> Hugging Face tokenizer/decoder
             -> redb persistence
             -> one Candle hosted E0 worker/thread
                  -> inference-runtime (E0)
                       -> Candle + Safetensors
                       -> mandatory/default CPU
                          or explicit feature-gated CUDA ordinal 0
```

### Phase 11 implemented boundary

| Execution target | Support and evidence boundary |
|---|---|
| CPU | Mandatory and default. The shared Ubuntu 24.04 quality workflow executes the default CPU graph without a CUDA toolkit or driver. |
| CUDA ordinal 0 | Supported only on Linux x86_64 with NVIDIA GeForce RTX 5070 Ti, driver 610.43.03, CUDA Toolkit 13.3, compute capability 12.0, and `CUDA_COMPUTE_CAP=120`. |
| Other NVIDIA/CUDA or GPU targets | Unsupported and unclaimed. The exact executed row does not establish generic NVIDIA compatibility. |
| Metal | Not implemented. |

The implementation and evidence boundary is:

- product features are exactly `desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`;
- benchmark features are exactly `runtime-benchmarks/cuda -> application-runtime/cuda`;
- `inference-runtime/cuda -> candle-backend/cuda` is a development-only fixture edge;
- no default graph reaches CUDA, and there is no generic `gpu` alias;
- E1 owns bounded discovery, explicit persisted selection, structured availability, and accelerator-memory policy without exposing Candle or `cudarc` types;
- unavailable selected CUDA fails without fallback;
- E0 verifies actual device, execution scalar, and adapter accounted footprint before publishing its reserved footprint; that actual device reaches E1 and Slint;
- CUDA-enabled binaries retain explicit CPU execution;
- sampling remains host-side over F32 logits after a safe CUDA-to-host transfer.

## Memory terminology

- **Accounted footprint** is the adapter’s accepted planning/accounting quantity reported by `LoadedModel::accounted_footprint()`; it is not physical residency.
- **Reserved footprint** is E0’s deterministic admission and ownership quantity in load receipts and snapshots, including retained cleanup ownership.
- **Process RSS** is a sampled whole-process OS observation that includes unrelated mappings, allocator retention, workers, and driver state.
- **Whole-device CUDA observation** is a sampled driver total/free/used value for the complete device, not process-attributed memory.

Accounted and reserved footprints establish their named deterministic contracts. Process RSS and whole-device CUDA observations establish only sampled environment state. None of these sampled observations proves a leak or non-leak, and immediate OS/allocator/driver reclamation is not inferred from E0 ownership release.

## Validation and evidence state

Normal CPU CI and self-hosted CUDA hardware CI passed on the accepted hardware-executed source baseline recorded above. The CUDA workflow is download-free and does not run TinyLlama, Hugging Face resolution, Criterion, elapsed-time thresholds, or Slint interaction. Its trigger trust boundary is canonical in [validation](validation.md#self-hosted-cuda-hardware-correctness-gate).

Manual external product evidence remains attributed to clean Commit E `411945e0fd53363f98609db21a43d757c4d9b506`, tree `7099dcb5c9879190543d3afa5fde399a84d799df`. The same exact TinyLlama workload executed on CPU and supported CUDA, including compatible chat, controlled completion, cancellation, release, unload, and bounded shutdown. Three complete CUDA lifecycle cycles ran, and the direct E0 CUDA fixture established zero model, request, workspace, and cleanup accounting after unload. The user also accepted manual Slint CPU and CUDA behavior; no screenshot or automated graphical assertion is claimed.

The post-Phase 11 schema-3 regression remains attributed to commit `7dd7a72565cfb976bf123ed664296e9332af0e70`, tree `766682d96b89a3e6fb4b0d14282e44e318244a56`. It changes the evidence schema and observation schedule without replacing Commit E’s historical timing values. Exact measurements and limitations remain only in [performance evidence](performance.md#external-product-evidence).

## Runtime and lifecycle guarantees

`application-runtime` is the public frontend-neutral E1 façade. E0 exclusively owns loaded model resources, sequences, request admission, generation workspaces, token scheduling and sampling, cancellation boundaries, output backpressure, cleanup quarantine, reserved-footprint accounting, unload, and terminal shutdown.

Current guarantees include:

- startup rollback retains and boundedly reaps partially created worker ownership;
- incompatible-model cleanup remains privately owned and accounted through success, proven disconnection, or observable bounded exhaustion;
- shutdown distinguishes running, stopping, cleanly stopped, retryable failure, and terminal failure;
- unresolved join handles remain owned after timeout so later shutdown can retry;
- E0 cleanup exhaustion is terminal and retains the runtime allocation until process exit rather than invoking unverified implicit backend destruction;
- successful request release restores model-only reserved footprint, and successful unload restores empty E0 accounting.

## Evidence infrastructure

`runtime-benchmarks` remains the sole cross-crate measurement observer. Its normal synthetic runner is download-free; its external runner requires explicit network authorization, an exact immutable TinyLlama revision, and mandatory `cpu` or `cuda:0` selection. Schema 3 records separate source/execution scalars, requested/selected/actual device identities, an independent accounted footprint, process RSS, and qualified whole-device CUDA observations. Exact methodology and results remain canonical in [performance evidence](performance.md); repeatable commands remain in [validation](validation.md) and [`benchmarks/runtime/README.md`](../../benchmarks/runtime/README.md).

## Unsupported behavior

- CUDA outside the exact matrix above, generic NVIDIA compatibility, generic `gpu`, automatic CPU fallback, cuDNN, flash attention, multi-GPU, and NCCL are unsupported.
- Metal and GPU-side sampling are unsupported.
- GGUF and other quantized formats are unsupported.
- Mixed-dtype Safetensors repositories are not generally supported. The current Candle Llama loader requires every tensor in every shard to match the source scalar declared by model configuration, so an otherwise compatible repository containing another floating-point tensor dtype may fail with `UnsupportedFormat`.
- Another local engine, hosted-provider execution, peer execution, and remote/browser transport are not implemented.
- Multi-model residency is unsupported.
- Chat compatibility is not generalized beyond the exact reviewed TinyLlama profile.
- Conversation persistence, arbitrary branch trees, and a Slint generation-settings panel are not implemented.
- Strict allocation freedom is not claimed for Candle or Hugging Face tokenization/decoding.
- Synthetic fixture timings do not establish language quality, representative scale, production throughput, or product-model performance.

## Historical context

Synthetic Phase 10 acceptance remains attributable to Commit A `efcd36e320a97d61d3f982619fee182410c514df`, tree `f80c5d6c746376df81d7ac8e7281ac9736e44d88`. The historical Phase 10 CPU product baseline remains attributable to Commit C `771c0de4d72565a6302ca60f3b6bafd8c807962b`, tree `3d5b6ccc5ecc959de7cb370c1147f76e4cd32e3f`. Those records are not rewritten as current-tree measurements.

The [recovered implementation plan](implementation-plan.md) remains non-authoritative historical source material. Phase 8’s former dual-product experiment and earlier support claims remain historical; current support follows accepted ADRs and this page.
