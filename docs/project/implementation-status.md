# Current implementation status

**Status date:** 2026-08-14

Milkdrift's implemented product is the local-inference foundation and optional E1
reference application kit. No later product phase is active. The next direction is
workflow/workspace/authority, but it is not a ratified implementation program.

This page is the sole current product support and evidence matrix. Procedures live
in [validation](validation.md), measurements in [performance](performance.md),
current behavior in component guides, and older chronology in
[execution history](../agent/execution/history.md).

## Product boundary

```text
native host (Slint is the reference)
  -> application-runtime (optional E1 services)
      -> immutable Hugging Face resolution + tokenizer + redb
      -> inference-runtime (E0)
          -> candle-backend
              -> unquantized Llama Safetensors
              -> CPU or explicit supported CUDA ordinal 0
```

General workflows and deployments, durable context workspaces, plugins,
provider/peer execution, browser transport, multi-model E1 residency, generalized
chat, conversation persistence, and a visual control center are not implemented
product paths.

## Support matrix

| Capability | Current support and boundary |
|---|---|
| Local engine | Candle only. E0 remains backend-independent at its portable contract. |
| Artifact source | Hugging Face revision resolved to an immutable commit. Provider evidence distinguishes exact LFS identity, verified Git blob, and project-established origin; Candle independently verifies expected length/SHA-256 or establishes a retained-file local baseline. |
| Format and architecture | Unquantized Safetensors through the reviewed Llama compatibility path. Complete selected structure is inspected; only required Llama tensors are materialized. |
| Required scalar layouts | `{F32}`, `{F16}`, `{F16,F32}`, `{BF16}`, and `{BF16,F32}`. Mixed F16/F32 and BF16/F32 require the matching recognized producer declaration. Genuine required F16+BF16 and required unsupported categories fail before device initialization. Understood unused extras remain observed but do not affect execution or footprint. |
| CPU | Mandatory in every build and the default. F32 and F16 execute directly; reviewed BF16-source layouts execute as F32. |
| CUDA | Non-default, explicit ordinal 0, no fallback. Product support is limited to the executed Linux x86_64 RTX 5070 Ti row: compute capability 12.0, Toolkit/compiler 13.3.73, build cap 120. This is not generic NVIDIA support. |
| Other devices | Metal, AMD/ROCm, cuDNN, flash attention, NCCL, multi-GPU, generic `gpu`, and automatic CPU fallback are unsupported. |
| Quantization and other models | GGUF, quantized loading, non-Llama architectures, arbitrary required mixtures, and GPU-side sampling are unsupported. |
| Residency | One selected/resident model in E1. E0 owns model/sequence resources exclusively. |
| Completion and chat | Direct completion for every loaded compatible model. Built-in chat only for `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable revision `fe8a4ea1ffedaf415f4da2f062534de366a451e6`. |
| Persistence | redb settings and model catalogue. `LAS1` writes v2/reads v1; `LAM1` writes v3 and reads exact v1/v2 without automatic rewrite. Conversation history is memory-only. |
| Frontend | Thin Slint reference host. The current state path is `milkdrift/state.redb`; one legacy `llm-app/state.redb` is moved only when no current database exists. |
| Orchestration foundation | `task-graph` owns generic allocation-free graph mechanics; `corrective-workflow` owns a bounded corrective definition/executor and reference template. Neither is the general workflow/workspace runtime. |

The exact scalar/materialization algorithm is owned by the
[Candle guide](candle-backend.md). Load, sequence, cleanup, and shutdown ownership
are described in [operation](operation.md), [inference runtime](inference-runtime.md),
and [lifecycle](lifecycle.md).

## Current implementation facts

- Configuration declaration, complete observed scalar set, required scalar policy,
  execution scalar, artifact provenance, expected content, final footprint,
  loading peak, sequence reservation, and retained ownership certainty are
  separate facts.
- Candle performs bounded complete inspection, sequential whole-shard
  verification, required-only materialization, and shard-aware accelerator
  transfer batches. Final and loading footprints count required ownership only.
- E0 reserves the loading peak before materialization, publishes only a completely
  verified model, and retains failed or contradictory owners through explicit
  bounded cleanup.
- Sequence reservation separates all-layer persistent state from one block's
  simultaneous transient peak and outer model state.
- E0 publishes a newly created sequence only after its exact identity, capacity,
  immutable plan, `Empty` state, and zero position match the admitted contract.
- E1 has one cleanup coordinator and exposes durable exact, unverified, or unknown
  retained state without treating disconnect or zero exact bytes as release.
- Token and text output keep distinct typed public APIs over one private bounded
  storage implementation. Runtime and application state machines are split by
  transition responsibility rather than driven by frontends. Accepted command and
  maintenance events remain ordered ahead of the correlated shutdown event even
  while the public event queue is full.
- Verification plans, maintained benchmarks, and CUDA hardware suites are declared
  in package metadata and consumed by both local tooling and CI. Destructive CI
  resource helpers resolve physical roots, reject root/checkout containment, and
  accept only children whose physical parent is the validated runner temporary
  directory.

## Accepted evidence

Evidence applies only to the named tree and scope.

| Evidence class | Exact baseline and accepted result | Current-tree consequence |
|---|---|---|
| Latest full local repository acceptance | `ee5078dd6bb6126afd12f25785a4e5effb38761b`, tree `50ad9901583252b474ccf48c79fa16558cd6e3e0`: focused benchmark/tooling tests, canonical composite, both portable plans, dependency policy, and offline links passed locally with isolated targets. | This is the latest complete locally accepted incoming baseline. It includes the verification/evidence consolidation but no current-tree CUDA execution. |
| Foundation local closure | `b1f7e90b1ba67f1cf968d773052b5062ef8cbbb9`, tree `fcb3ee6fa00243734abd74b64218aa0db2e340c1`: complete download-free CPU, native component, composite, portable, policy, and offline-link matrix passed. | Deterministic CPU lifecycle/accounting foundation accepted for that tree. |
| Latest accepted hosted Quality | [run 31696186308](https://github.com/hartolit/milkdrift/actions/runs/31696186308) passed every required job on `6df699c3b2bb1b7ffa59f7bcf86c69d9e0654813`, tree `c3a870cca7b7569e648787ca68c42e513d56f48d`. | Proves only that older exact tree. Later source/tooling changes still require their own remote run. |
| Latest accepted self-hosted CUDA | [run 31696186329](https://github.com/hartolit/milkdrift/actions/runs/31696186329) passed compile, strict Clippy, adapter/E0/E1 hardware, 47 deterministic fault cases, and cleanup on the exact RTX row for `6df699c`. | Establishes the supported hardware row, not later-tree CUDA acceptance or generic NVIDIA support. |
| Later local accelerator loading | `716ae9a23ea12fc81374e4d576d3a3a61f2ae8e9`, tree `131f457dd32b6e31886769980637cda49f72fd8a`: the focused default matrix and six local RTX adapter cases passed for bounded transfer batching and verified artifact loading. | Correctness evidence for that exact accelerator tree; no speedup or external-model result is inferred. |
| External product performance | Historical controlled CPU/CUDA evidence is tied to Commit E `411945e0fd53363f98609db21a43d757c4d9b506`, tree `7099dcb5c9879190543d3afa5fde399a84d799df`. | Curated measurements remain valid only for that environment/workload; no schema-6 current-tree product report is accepted. |

Documentation-authority commit `acdd2ed066808661f6e0f7336dedf84513016850`, tree
`56008a2d76b96205bb810597464603cd3a5cafcb`, changed documentation and stable
hygiene policy without broadening support. The independent source-closure
candidate changes E0 contract enforcement, CI resource containment, regression
coverage, and active documentation. Its resulting commit/tree and final local
checks are reported externally because a tracked document cannot name the commit
that contains itself.

## Open evidence and unsupported claims

- Hosted Quality and self-hosted CUDA have not run on the source-closure candidate.
  Earlier runs are not promoted to current-tree evidence.
- No immutable, license-reviewed external mixed-layout checkpoint and no external
  schema-6 CPU/CUDA product report is accepted. Deterministic project fixtures own
  current mixed-layout correctness.
- Synthetic fixtures prove compatibility and lifecycle behavior, not language
  quality, representative scale, physical leak freedom, production throughput, or
  external-checkpoint compatibility.
- Process RSS and whole-device memory are observations, not deterministic owner
  attribution. Exact E0 accounting does not claim immediate allocator, driver, or
  operating-system reclamation.
