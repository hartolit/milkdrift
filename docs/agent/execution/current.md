# Current execution context

**Status date:** 2026-08-15
**State:** continuation packages 01–07 closed; no product phase active

## Immediate boundary

The accepted continuation source boundary is
`3ac08a14a89f9d8ab4b50520e6336ee7f583aba4`, tree
`23143bc78392c24f4c9c0345e168d7d56a92816f`. Packages 01–07 are closed there by
[Quality run 31835967580](https://github.com/hartolit/milkdrift/actions/runs/31835967580)
and [CUDA run 31835967556](https://github.com/hartolit/milkdrift/actions/runs/31835967556).

The repository is parked at that accepted local CPU/CUDA foundation. No source or
test maintenance candidate supersedes it, and workflow/workspace/authority
remains an unratified product direction.

## Unresolved acceptance

- No reviewed external mixed-layout checkpoint or current schema-6 external
  product run exists.
- Deterministic project fixtures remain the current mixed-layout correctness
  evidence. AMD support is absent.
- Any later source/test candidate requires new exact-tree hosted Quality and CUDA
  acceptance before it replaces the boundary above.

## Canonical owners

- [Project architecture](../../project/architecture.md)
- [End-to-end operation](../../project/operation.md)
- [Implementation status and evidence](../../project/implementation-status.md)
- [Validation procedure](../../project/validation.md)
- [Performance evidence](../../project/performance.md)
- [Execution plan](execution-plan.md)
- [Milestone history](history.md)

## Environment handoff

The usual UM790 Pro host supports CPU, portability, policy, and documentation
work. It has no NVIDIA device, `nvidia-smi`, or `nvcc`; absence of local CUDA
execution is an environment limitation, not a product failure. Use isolated
targets, `CARGO_INCREMENTAL=0`, and one heavy Cargo process at a time.
