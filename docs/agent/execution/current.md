# Current execution context

**Status date:** 2026-08-04
**Code-under-test (Commit E):** `411945e0fd53363f98609db21a43d757c4d9b506`
**Commit E tree:** `7099dcb5c9879190543d3afa5fde399a84d799df`

```text
Phase 10: complete.
External CPU baseline: complete.
Phase 11 complete for the executed CPU + Linux CUDA matrix.
No subsequent phase is active.
```

Commit E was clean for the executed acceptance matrix. Raw reports remained beneath ignored root `target/`. The authoritative runner, commands, and cache policy are in [`benchmarks/runtime`](../../../benchmarks/runtime/README.md) and [validation](../../project/validation.md#phase-11-controlled-cpu-and-cuda-product-evidence). Exact result tables and limitations are in [performance evidence](../../project/performance.md#external-product-evidence); concise chronology is in [execution history](history.md). GitHub Actions acceptance remains a separate post-push fact and is not claimed until an observed run is recorded.

CPU remains mandatory, is the default-build and fresh-install selection, remains the shared-CI path, and passed the executed CPU tests and final CPU compile/test/Clippy gates. Explicit CUDA ordinal 0 is supported only on the executed Linux x86_64 matrix: NVIDIA GeForce RTX 5070 Ti, driver 610.43.03, CUDA toolkit 13.3, compute capability 12.0, and build target 120. This is not a generic NVIDIA compatibility claim.

The product feature graph is exactly `desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`; the benchmark graph is `runtime-benchmarks/cuda -> application-runtime/cuda`; and the direct E0 test edge `inference-runtime/cuda -> candle-backend/cuda` remains development-only. No default graph reaches CUDA. User device selection is explicit and persisted; unavailable CUDA fails without fallback. E0 verifies the actual loaded device, and that identity reaches E1 and Slint. Sampling remains host-side over F32 logits after CUDA transfer.

The exact supported TinyLlama primary workload passed on CPU and CUDA, including cancellation, release, unload, and shutdown. Three complete CUDA lifecycle cycles were stable; a direct E0 CUDA snapshot test proved zero model/request/workspace/cleanup accounting; adapter and E1 CUDA tests passed; schema-2 chat timing is now recorded; and the final CPU and CUDA compile/test/Clippy gates passed. The user accepted the manual Slint run: CPU and CUDA worked, CUDA output was visibly near instant, and no interaction issue was observed. No screenshots were recorded or claimed.

Metal, `cudnn`, flash attention, GGUF/quantized formats, GPU-side sampling, multi-GPU, `nccl`, another local engine, hosted execution, and peer execution remain unsupported or deferred. One selected/resident model remains the product limit.

## Next session ownership

There is no next-session implementation work and no active product phase. Future execution tracks remain inactive until a separate reviewed activation decision. The only unresolved acceptance fact is external to the local closure record: GitHub Actions acceptance must be observed after push before it can be claimed.

## Canonical links

- [Execution plan](execution-plan.md)
- [Implementation status](../../project/implementation-status.md)
- [Performance evidence](../../project/performance.md)
- [Validation procedures](../../project/validation.md)
- [Execution history](history.md)
