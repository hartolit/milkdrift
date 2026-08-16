# Current implementation status

**Status date:** 2026-08-16

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
- Backend load failures preserve one bounded portable lifecycle stage and, when a
  single tensor is authoritative, a checked ordinal/fingerprint coordinate through
  E0 cleanup and E1 presentation. Paths, tensor names, vendor errors, adapter
  inventory, and cleanup failures do not replace the primary diagnostic.
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

## Accepted historical evidence

Evidence applies only to the named historical commit, tree, command, and scope.
Tracked documentation makes no post-push CI assertion about the checked-out
commit. Current-checkout remote acceptance is determined externally by the
exact-SHA procedure in [validation](validation.md#exact-current-checkout-remote-acceptance).

| Evidence class | Exact historical baseline and accepted result | Scope |
|---|---|---|
| Documented full local repository acceptance | Commit `ee5078dd6bb6126afd12f25785a4e5effb38761b`, tree `50ad9901583252b474ccf48c79fa16558cd6e3e0`: focused benchmark/tooling tests, canonical composite, both portable plans, dependency policy, and offline links passed locally with isolated targets. | Complete local incoming baseline for the continuation closure. |
| Foundation local closure | Commit `b1f7e90b1ba67f1cf968d773052b5062ef8cbbb9`, tree `fcb3ee6fa00243734abd74b64218aa0db2e340c1`: the complete download-free CPU, native-component, composite, portable, policy, and offline-link matrix passed. | Deterministic CPU lifecycle/accounting evidence for that exact tree. |
| Hosted Quality baseline | [Run 31835967580](https://github.com/hartolit/milkdrift/actions/runs/31835967580) passed every required hosted job for exact commit `3ac08a14a89f9d8ab4b50520e6336ee7f583aba4`, tree `23143bc78392c24f4c9c0345e168d7d56a92816f`. | Historical acceptance of continuation packages 01–07 on that tree only. |
| Self-hosted CUDA baseline | [Run 31835967556](https://github.com/hartolit/milkdrift/actions/runs/31835967556) passed compile, strict Clippy, six Candle adapter cases, one E0 lifecycle case, 49 serial fault-injection cases, two E1 cases, and cleanup for exact commit `3ac08a14a89f9d8ab4b50520e6336ee7f583aba4`, tree `23143bc78392c24f4c9c0345e168d7d56a92816f`. | Historical execution on the maintained RTX 5070 Ti row; not generic NVIDIA or AMD support. |
| Local accelerator loading | Commit `716ae9a23ea12fc81374e4d576d3a3a61f2ae8e9`, tree `131f457dd32b6e31886769980637cda49f72fd8a`: the focused default matrix and six local RTX adapter cases passed for bounded transfer batching and verified artifact loading. | Correctness evidence for that exact accelerator tree; no speedup or external-model result is inferred. |
| External product performance | Commit `411945e0fd53363f98609db21a43d757c4d9b506`, tree `7099dcb5c9879190543d3afa5fde399a84d799df`: controlled CPU/CUDA product evidence was recorded under its historical schema and environment. | Curated measurements remain scoped to that environment/workload; no external schema-6 product report is accepted. |

Documentation-authority commit `acdd2ed066808661f6e0f7336dedf84513016850`, tree
`56008a2d76b96205bb810597464603cd3a5cafcb`, changed documentation and stable
hygiene policy without broadening support. The later `3ac08a14...` baseline adds
independently reviewed source closure and exact-tree hosted/CUDA historical
evidence without activating a successor product phase. Subsequent maintenance
commits do not inherit those runs.

## Open evidence and unsupported claims

- No immutable, license-reviewed external mixed-layout checkpoint and no external
  schema-6 CPU/CUDA product report is accepted. Deterministic project fixtures own
  current mixed-layout correctness.
- Synthetic fixtures prove compatibility and lifecycle behavior, not language
  quality, representative scale, physical leak freedom, production throughput, or
  external-checkpoint compatibility.
- Process RSS and whole-device memory are observations, not deterministic owner
  attribution. Exact E0 accounting does not claim immediate allocator, driver, or
  operating-system reclamation.
