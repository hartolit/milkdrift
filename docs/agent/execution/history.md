# Execution history

This is the milestone and evidence ledger. Entries preserve exact-tree provenance,
durable outcomes, accepted run IDs, and important gaps without repeating current
support matrices, component behavior, commands, or measurement tables.

For current truth use [implementation status](../../project/implementation-status.md),
for procedures use [validation](../../project/validation.md), and for curated
measurements use [performance evidence](../../project/performance.md).

## Foundation phases 3–9

| Milestone | Date and baseline | Durable outcome | Evidence/gap at closure |
|---|---|---|---|
| Phase 3 — generation kernel | 2026-07-23; uploaded locked source | E0 gained bounded admission, scheduling, output, cancellation, cleanup quarantine, accounting, unload, and terminal shutdown. | Local canonical verification was recorded without a durable exact Git identity. |
| Phase 4 — Candle CPU slice | 2026-07-25; `8de2ebf` | Real Candle/Llama E0 load, prefill/decode, sampling, cancellation, sequence destruction, unload, and external tiny-random smoke. | Local only; historical measurements remain in [performance](../../project/performance.md#historical-phase-4-external-smoke). |
| Phase 5 — E1 generation | 2026-07-25; `f6ac180` | Frontend-neutral direct completion, tokenizer/decode bridge, bounded text output, unload policies, and worker lifecycle. | Source boundary recorded; exact post-closure commit gate was still required. |
| Phase 6 — Slint product | 2026-07-27; `6843864` plus review closure | Thin Slint host exposed E1 direct completion with frame-batched output and explicit shutdown. | Download-free local validation passed; no manual external graphical session. |
| Pre-Phase 7 architecture | 2026-07-29; `f8b3396` | Corrective workflow became an independent capability runtime; domain/platform/adapter/runtime/app roots and E0/E1 direction were established. | Later exact-tree validation remained necessary. |
| Phase 7 — compatible chat | 2026-07-29; `2b03cfb`, review fixes `8d134e3`/`3b4541f` | Exact TinyLlama chat profile, conversation attempts, atomic context-planning units, regeneration, and Slint chat. | Original gate predated final semantic fixes; no manual external session. |
| Phase 8 — second-backend experiment | 2026-07-30; `797ba0f` working tree | GGUF parity tested E0/E1 boundaries and showed that a second engine added composition cost without an independent lifecycle. | Focused and canonical local gates passed; architecture was intentionally superseded by ADR-0013. |
| Phase 9 — Candle-only correction | 2026-07-31; `f0fe9c6`, tree `db8a9ae` | Removed GGUF/llama.cpp and Python operational tooling; retained Candle-only E1, backend-neutral E0, exact chat, and Rust/Cargo-native policy. | Local canonical, portable, dependency, link, clean-build, and one E1 Hub smoke passed; no graphical session. |
| Phase 9 lifecycle/tooling closure | 2026-08-01; input `f0fe9c6`, later checkpoint `3942a19` | Retryable E1 shutdown ownership, rejected-model cleanup, transactional startup, exact domain DAG, virtual workspace/xtask, stable Clippy policy, and terminal process-retention semantics. | Local canonical/portable/policy gates passed. External Hub and UI evidence were not rerun. |

ADRs [0008](../decisions/0008-capability-and-execution-boundaries.md),
[0013](../decisions/0013-candle-only-local-execution.md),
[0014](../decisions/0014-rust-cargo-native-operational-tooling.md), and
[0016](../decisions/0016-virtual-workspace-focused-xtask.md) preserve the
architectural conclusions from these phases.

## Phase 10 — measurement infrastructure and CPU product baseline

**Date:** 2026-08-01 to 2026-08-03

- Original infrastructure commit `62a342e` was not accepted after fresh CI exposed
  a test-harness allocation error. Corrections `148f0fe` and `f883d64` produced
  accepted code-under-test Commit A
  `efcd36e320a97d61d3f982619fee182410c514df`, tree
  `f80c5d6c746376df81d7ac8e7281ac9736e44d88`.
- Commit A established maintained sampling/E0 Criterion targets, one download-free
  synthetic lifecycle/process runner, exact allocation gates, and governed
  benchmark/model-fixture artifacts. Its local exact-tree, portable, policy, and
  link gates passed; four focused Criterion targets and the synthetic baseline were
  measured.
- External runner Commit C
  `771c0de4d72565a6302ca60f3b6bafd8c807962b`, tree
  `3d5b6ccc5ecc959de7cb370c1147f76e4cd32e3f`, then completed the fixed TinyLlama
  E1 CPU resolve/load/chat/completion/cancel/unload/shutdown workload from a clean
  tree.

Curated environments, timings, RSS, and limitations remain in
[performance evidence](../../project/performance.md#commit-a-controlled-baseline)
and [external product evidence](../../project/performance.md#external-product-evidence).
No remote exact-tree success or GPU capability was claimed at Phase 10 closure.

## Phase 11 — executed CPU and Linux CUDA

**Date:** 2026-08-04

Accepted product baseline Commit E
`411945e0fd53363f98609db21a43d757c4d9b506`, tree
`7099dcb5c9879190543d3afa5fde399a84d799df`, added mandatory/default CPU plus
explicit no-fallback CUDA ordinal 0, E1/Slint device selection, and controlled
same-workload product evidence. Support was intentionally limited to the observed
RTX 5070 Ti, compute-capability-12.0, Toolkit-13.3/build-cap-120 row. Sampling
remained host-side.

The final quality baseline
`1a62d2ed6623500e9052b4b8386ebd058984bd89`, tree
`79864da274aed94471c2fbcfedaa97c2f32f3e7a`, passed hosted
[Quality run 30942153370](https://github.com/hartolit/milkdrift/actions/runs/30942153370)
and self-hosted
[CUDA run 30942148369](https://github.com/hartolit/milkdrift/actions/runs/30942148369).
Those runs establish only that exact product/runner matrix. Exact timing and memory
results remain in [performance](../../project/performance.md#historical-phase-11-controlled-cpu-vs-cuda-product-evidence).

## Phase 12 — transaction-bound Safetensors loading

**Date:** 2026-08-08

Segments `58490fe` and `1251069` culminated in closure commit
`181a069ce81525e9c144fe8de051ced8e3c0b9d7`, tree
`310e437c0729f51fe6c0ba3dcb5fbf9f1935a80f`. The durable result is one
transaction-bound prepared load, per-tensor required-versus-observed scalar truth,
exact final/loading-peak planning, reviewed mixed F16/F32 and BF16/F32 conversion,
and explicit failed-load ownership. [ADR-0020](../decisions/0020-transactional-prepared-model-loading.md)
owns the decision.

Local CPU, portable, policy, link, and exact RTX CUDA matrices passed. After push,
self-hosted [CUDA run 31281013243](https://github.com/hartolit/milkdrift/actions/runs/31281013243)
passed on the closure commit. Hosted
[Quality run 31281013257](https://github.com/hartolit/milkdrift/actions/runs/31281013257)
passed canonical native work, then the superseded workspace-wide bench topology
exhausted disk before portable work completed; this was infrastructure history,
not a WASM product failure.

No immutable license-reviewed external mixed-layout checkpoint was accepted. The
fixed TinyLlama profile remained homogeneous BF16 lifecycle evidence only.

## Pristine ownership amendments

**Date:** 2026-08-09 to 2026-08-10

- `d4a1e4324a6793becc56147e4b2e3246189d2693`, tree `1357ef2b`, hardened
  declaration truth, bounded complete inspection, selective verified
  materialization, source identity, and required-only footprint planning.
- `b43d0f47953c5319a41340d9087b7fd8f07b3280`, tree `634ebb55`, separated byte
  ownership from cache rates and exact from unverified retained ownership.
- `1f91cba691a8099805fa31f576079e79c282c73e`, tree `00b519d2`, tightened E1
  receipt validation, retained state, `LAM1` v3 persistence, and thin-host
  projection.
- `88f2d97ce6728a3ac1f783ffb6655979247038dc` replaced package-name/exact-edge
  policy with explicit roles, a generic inward DAG, declarative benchmarks, and
  isolated CI/hardware suites. Local gates passed; CUDA and hosted runs were then
  pending.
- `eae49a6fcf61df270ddc8dea1a03910063a5bf90` completed the independent source
  review without activating another product phase.

The removed prompt bodies are indexed only for provenance in
[archive/README.md](archive/README.md).

## Foundation repair and local closure

**Date:** 2026-08-13

Commits `4578373`, `4606874`, `78ebf3d`, `56ce2bb`, and `b1f7e90` corrected mixed
declaration ambiguity, failed-load typestate/plan stability, E1 cleanup
coordination, simultaneous-lifetime sequence reservation, generic task-graph
ownership, and per-profile verification targets.

Clean local closure baseline
`b1f7e90b1ba67f1cf968d773052b5062ef8cbbb9`, tree
`fcb3ee6fa00243734abd74b64218aa0db2e340c1`, passed the complete download-free
CPU, six native component, composite, two portable, dependency-policy, and offline
link classes on the UM790 Pro. CUDA was unavailable on that host. The result proved
deterministic fixture lifecycle/accounting contracts, not physical leak freedom,
representative performance, or an external checkpoint.

## Verification infrastructure and remote repair

**Date:** 2026-08-13

Candidate `f3e2f5b` was rejected: hosted
[run 31688874924](https://github.com/hartolit/milkdrift/actions/runs/31688874924)
found an invalid fairness-test assumption, while CUDA
[run 31688874952](https://github.com/hartolit/milkdrift/actions/runs/31688874952)
stopped before compilation because the maintained offline cache lacked
`serde_yaml_ng`.

Scheduler repair `59fa35c` passed hosted
[run 31693672969](https://github.com/hartolit/milkdrift/actions/runs/31693672969);
its CUDA run repeated the cache precondition failure. Cache synchronization repair
`db8015c` passed hosted
[run 31695345591](https://github.com/hartolit/milkdrift/actions/runs/31695345591)
and reached CUDA compilation, where strict Clippy exposed hardware-only helpers in
the ordinary feature graph.

Feature-boundary repair
`6df699c3b2bb1b7ffa59f7bcf86c69d9e0654813`, tree
`c3a870cca7b7569e648787ca68c42e513d56f48d`, then passed hosted
[Quality run 31696186308](https://github.com/hartolit/milkdrift/actions/runs/31696186308)
and self-hosted
[CUDA run 31696186329](https://github.com/hartolit/milkdrift/actions/runs/31696186329),
including adapter/E0/E1 hardware and all deterministic fault cases. Later trees do
not inherit those run results.

## Pristine continuation packages 01–04

**Date:** 2026-08-13 to 2026-08-14

| Package | Commit | Durable outcome | Evidence gap at closure |
|---|---|---|---|
| Output foundation | `0a6cfd172367b66e26c2d393ba2365575a6d6d28` | Token and text output retain distinct typed APIs over one private bounded implementation. | Later exact-tree remote runs pending. |
| Runtime/application structure | `aa7363d8dd2b2838837cc2e2d52291ed432274de` | Large runtime/application state machines split by transition responsibility without compatibility wrappers. | Later exact-tree remote runs pending. |
| Artifact/accelerator pipeline | `716ae9a23ea12fc81374e4d576d3a3a61f2ae8e9`, tree `131f457d` | Honest expected-content input, bounded shard-aware transfer batches, one sync per batch, exact live-footprint planning, and local six-case RTX adapter coverage. | No speedup or external-model claim; later combined-tree CUDA pending. |
| Verification/evidence infrastructure | `ee5078dd6bb6126afd12f25785a4e5effb38761b`, tree `50ad9901` | Shared benchmark support, nonduplicated schema-6 evidence, metadata-owned hardware profiles, and consolidated workflow/resource validation. | Full local gates accepted; exact-tree hosted/CUDA pending. |

## Documentation authority maintenance

**Date:** 2026-08-14
**Input:** `ee5078dd6bb6126afd12f25785a4e5effb38761b`, tree
`50ad9901583252b474ccf48c79fa16558cd6e3e0`

The active documentation was rebuilt around one README/vision/architecture/
operation/status/evidence/component/ADR spine. Completed prompt bodies, the stale
analyzer, recovered implementation plan, and free-floating application warning
were removed; unique accepted rationale remains in the vision, project
architecture, component guides, and ADRs. Current context and this ledger were
compressed, validation was separated from results, and stable hygiene rules now
prevent the retired layout from returning.

The resulting commit/tree and post-commit validation are reported by the package
completion response rather than embedded self-referentially here. No product
support or historical run was broadened by this documentation change.
