# Current execution context

**Status date:** 2026-08-02
**Code-under-test (Commit A):** `efcd36e320a97d61d3f982619fee182410c514df`
**Commit A tree:** `f80c5d6c746376df81d7ac8e7281ac9736e44d88`
**Repository status:** Phase 10 repository infrastructure and synthetic acceptance complete
**External-product status:** baseline outstanding; no current product-performance claim
**Next numbered phase:** Phase 11 is not active

Commit A was clean before and after its dedicated-target validation and measurements. It contains the isolated deterministic domain allocation gate, one-shot sampling-matrix coverage, and the simplified synthetic-only runtime benchmark package. The exact local acceptance summary is in [execution history](history.md#phase-10--repository-infrastructure-and-synthetic-acceptance), and exact methodology/results are in [performance evidence](../../project/performance.md).

The follow-on evidence commit (Commit B) changes documentation only. Commit A therefore remains the executable tree measured; Commit B’s identity and post-commit local gate belong in the closure report rather than in a self-referential tracked file.

## Immediate handoff

- Canonical documentation ownership has been consolidated; timing intervals appear only in `docs/project/performance.md`.
- Raw synthetic JSON and Criterion output remain ignored beneath root `target/`.
- External product evidence remains outstanding; no network-dependent product run was authorized or performed. See [performance evidence](../../project/performance.md#external-product-evidence).
- Before Phase 11: execute the exact-model/revision external baseline, reconcile any finding, and re-establish a clean accepted CPU tree as defined by the [execution plan](execution-plan.md#phase-11--gpu-execution).

## Canonical links

- [Execution plan](execution-plan.md)
- [Implementation status](../../project/implementation-status.md)
- [Performance evidence](../../project/performance.md)
- [Validation procedures](../../project/validation.md)
- [Execution history](history.md)
