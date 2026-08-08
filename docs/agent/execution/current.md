# Current execution context

**Status date:** 2026-08-08

```text
Phase 10 complete.
Phase 11 complete for the executed CPU + Linux CUDA matrix.
Post-Phase 11 quality closure complete.
Phase 12 complete for deterministic CPU compatibility and the exact locally executed CUDA matrix.
The Phase 12 self-hosted workflow remains unrun; external mixed-checkpoint evidence is absent.
No subsequent product phase is active.
```

Phase 12 was executed through the [segmented execution guide](milkdrift-phase12-execution-guide.md), not the older monolithic prompt.

## Phase 12 handoff

- **Segment 1 — core loader/runtime:** commit `58490fe693fef7a2635956181088664cd90685e8` (`58490fe`) implemented per-tensor Safetensors inspection, a consumable prepared-load transaction, exact final and loading-peak plans, mixed-layout conversion, E0 validation, and retained partial-load cleanup.
- **Segment 2 — artifact/application:** commit `12510695aa29be6a2665dbf3777cccbb8172c2d1` (`1251069`) integrated optional configuration-declared metadata, immutable artifacts, E1 receipt truth, persistence compatibility, retained cleanup events, and compile-only thin Slint adaptation without adding frontend responsibility.
- **Segment 3 — validation/project truth:** this closure commit updates deterministic CPU/CUDA fixtures, benchmark observers, report schemas, the self-hosted workflow, and canonical documentation.

The closure tree has focused, download-free CPU passes for:

- `candle-backend --test llama_cpu`: 20 passed, including homogeneous and mixed layout policy, malformed/unsupported rejection, exact final/loading-peak planning, and pre-materialization host-budget rejection;
- `inference-runtime --test native_backend_generation`: 3 passed, including the mixed F16/F32 hosted E0 generation, release, unload, empty-accounting, shutdown, and join lifecycle;
- `inference-runtime --test fault_injection`: 32 passed, including aggregate loading-peak admission and immediate, retryable, retained, and exhausted cleanup ownership paths;
- `runtime-benchmarks`: 78 passed, including synthetic schema 3 and external schema 4 serialization and evidence-separation checks.

The canonical `cargo xtask verify` gate also passed from a previously absent Cargo target directory. Both portable-domain target matrices, locked dependency policy, and offline Markdown links passed.

## Exact current compatibility boundary

The implemented unquantized Candle Llama Safetensors policy accepts homogeneous `{F32}`, `{F16}`, and `{BF16}` layouts and mixed `{F16, F32}` or `{BF16, F32}` layouts. On CPU, F32 executes as F32, F16 as F16, and BF16 as F32. F16/BF16 mixtures, unsupported integer/unknown dtypes, quantized layouts, contradictory declarations, and layouts outside the current Llama path remain rejected before device materialization where applicable.

The exact CUDA compile chain and complete deterministic hardware matrix passed locally on 2026-08-08 on NVIDIA GeForce RTX 5070 Ti ordinal 0, driver/KMD 610.43.03, CUDA UMD/toolkit 13.3, `nvcc` 13.3.73, compute capability 12.0, and build cap 120. This establishes only the exact local fixture matrix. The accepted Phase 11 Actions baseline remains historical, and the Phase 12 GitHub self-hosted workflow has not run.

## Remaining evidence gaps

- The Phase 12 GitHub self-hosted CUDA workflow has not run; no Actions run ID or remote attestation is claimed.
- No schema-4 external product report or new Phase 12 performance measurement is accepted.
- No suitable immutable, license-reviewed external mixed-dtype Llama checkpoint was established, so external mixed-checkpoint compatibility remains unclaimed.

No immutable, license-reviewed, suitable external mixed-dtype Llama profile has been established. The pinned TinyLlama revision is homogeneous BF16 and remains only the historical product/lifecycle profile. Missing network access or credentials would be acquisition failures, not compatibility failures; no such access failure is being represented as evidence that a mixed checkpoint is incompatible.

The planned next major program returns to workflow, workspace, artifact, authority, capability, budget, and execution-endpoint foundations. Further Candle-loader expansion is not the default successor.

Canonical owners:

- [Current implementation and evidence status](../../project/implementation-status.md)
- [Current validation procedures and recorded gates](../../project/validation.md)
- [Performance evidence, schema semantics, and limitations](../../project/performance.md)
- [Active execution plan](execution-plan.md)
- [Execution history](history.md)
