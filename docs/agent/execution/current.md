# Current execution context

**Status date:** 2026-08-03
**External code-under-test (Commit C):** `771c0de4d72565a6302ca60f3b6bafd8c807962b`
**Commit C tree:** `3d5b6ccc5ecc959de7cb370c1147f76e4cd32e3f`

```text
Phase 10: complete.
External CPU baseline: complete.
Phase 11: active; E1/Slint explicit CUDA selection implemented, external product evidence outstanding.
```

Commit C was clean before and after the authorized exact-model CPU run. The authoritative runner, command, and cache policy are in [`benchmarks/runtime`](../../../benchmarks/runtime/README.md) and [validation](../../project/validation.md#external-cpu-product-baseline). Exact results and limitations are in [performance evidence](../../project/performance.md#external-product-evidence); concise chronology is in [execution history](history.md).

CPU remains mandatory and is the fresh-install/default-build selection. The lower Candle/E0 CUDA path now feeds explicit E1 and Slint CPU/CUDA selection through `desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`; the existing `inference-runtime/cuda -> candle-backend/cuda` forwarding edge remains development-only. No default graph reaches CUDA, and there is no generic `gpu`, `cudnn`, `flash-attn`, or `nccl` feature path.

E1 now owns bounded discovery, structured availability diagnostics, persisted selected-device state, explicit accelerator-memory policy, load-time re-probing, exact selected-device admission, and receipt verification. Slint presents the stable E1 identities without parsing labels or falling back. On the current uncommitted development tree, the focused CUDA feature matrix and release desktop build passed, and the ignored E1 fixture discovered `CUDA 0 — NVIDIA GeForce RTX 5070 Ti` with compute capability 12.0, proved selection alone remained idle/unloaded, then loaded and unloaded the committed fixture with matching selected/actual identity. The desktop process launched against an isolated fresh database, but the Plasma session was locked, so visual selector interaction was not verified. External/product-model CUDA evidence and measurements remain outstanding.

## Next session ownership

- repeat acceptance from a clean committed tree and visually exercise the Slint selector on an unlocked CUDA desktop;
- preserve the CPU fresh-install default and explicit no-fallback behavior while completing that UI evidence;
- complete external/product-model CUDA evidence and measurements before marking Phase 11 complete;
- keep model resolution device-independent and preserve E1/E0 ownership, bounded memory admission, and exact selected-versus-actual device verification.

## Canonical links

- [Execution plan](execution-plan.md)
- [Implementation status](../../project/implementation-status.md)
- [Performance evidence](../../project/performance.md)
- [Validation procedures](../../project/validation.md)
- [Execution history](history.md)
