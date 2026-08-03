# Current execution context

**Status date:** 2026-08-03
**External code-under-test (Commit C):** `771c0de4d72565a6302ca60f3b6bafd8c807962b`
**Commit C tree:** `3d5b6ccc5ecc959de7cb370c1147f76e4cd32e3f`

```text
Phase 10: complete.
External CPU baseline: complete.
Phase 11: active; lower-layer CUDA/E0 foundation implemented, not complete.
```

Commit C was clean before and after the authorized exact-model CPU run. The authoritative runner, command, and cache policy are in [`benchmarks/runtime`](../../../benchmarks/runtime/README.md) and [validation](../../project/validation.md#external-cpu-product-baseline). Exact results and limitations are in [performance evidence](../../project/performance.md#external-product-evidence); concise chronology is in [execution history](history.md).

CPU remains the mandatory default and the only E1/frontend-selected product device. The lower-layer foundation now adds explicit non-default Linux CUDA in `candle-backend`, verified actual-device and footprint contracts in E0, device-aware scalar/memory/logit handling, and ignored opt-in hardware tests. CUDA ordinal 0 executed the committed fixture on an NVIDIA GeForce RTX 5070 Ti with compute capability 12.0 using driver 610.43.03 and CUDA Toolkit 13.3; direct adapter execution and hosted E0 prefill/decode, synchronization, unload, and zero post-unload accounting passed locally.

## Next session ownership

- add frontend-neutral discovery and explicit CPU/CUDA selection to E1 without changing CPU defaults;
- persist and expose only stable adapter-owned device facts, never Candle or `cudarc` types;
- add Slint presentation and user selection without moving lifecycle or token scheduling into the frontend;
- preserve explicit failure after CUDA selection; do not add automatic CPU fallback;
- complete product-level CUDA evidence and measurements before marking Phase 11 complete.

## Canonical links

- [Execution plan](execution-plan.md)
- [Implementation status](../../project/implementation-status.md)
- [Performance evidence](../../project/performance.md)
- [Validation procedures](../../project/validation.md)
- [Execution history](history.md)
