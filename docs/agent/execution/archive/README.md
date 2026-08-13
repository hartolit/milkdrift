# Completed execution provenance

Completed prompt bodies are intentionally absent from the active tree. Their exact
text remains available through Git history; this index records only the boundary
and durable outcome needed to find the result.

| Historical package | Execution or comparison boundary | Durable outcome |
|---|---|---|
| Phase 12 implementation audit | audited `181a069`, compared with `a28008a` | [ADR-0020](../../decisions/0020-transactional-prepared-model-loading.md) and [Phase 12 history](../history.md#phase-12--transaction-bound-safetensors-loading) |
| Pristine 01 — artifact loading | `d4a1e43` | [Candle backend](../../../project/candle-backend.md) and [ADR-0020](../../decisions/0020-transactional-prepared-model-loading.md) |
| Pristine 02 — runtime ownership | `b43d0f4` | [Inference runtime](../../../project/inference-runtime.md) and [model lifecycle](../../../project/lifecycle.md) |
| Pristine 03 — application boundary | `1f91cba` | [Application runtime](../../../project/application-runtime.md) |
| Pristine 04 — infrastructure truth | `88f2d97` | [Dependency policy](../../../project/dependency-policy.md), [validation](../../../project/validation.md), and [history](../history.md#verification-infrastructure-and-remote-repair) |
| Pristine 05 — independent closure | `eae49a6` | [Foundation-repair history](../history.md#foundation-repair-and-local-closure) |
| Pristine execution order | completed by `eae49a6` | [Execution plan](../execution-plan.md) |

These entries are provenance, not required reading or active instructions.
