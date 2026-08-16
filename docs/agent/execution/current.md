# Current execution context

**Status date:** 2026-08-16
**State:** canonical present-scope consolidation complete locally; exact-tree acceptance external

## Immediate handoff

The workspace now contains only the implemented product/runtime spine plus its
benchmark and policy tooling. Every retained package declares a single
responsibility, runtime reachability is derived from Cargo's resolved production
graph, the inactive task graph and corrective workflow are removed, and the
documented lint contract matches the enforced gate. No product successor is
active; workflow/workspace/authority remains unratified direction.

Exact current-commit remote acceptance is an external property evaluated after
push. Tracked execution state neither predicts that result nor stores a run ID to
make itself current. Use the exact-SHA procedure in
[validation](../../project/validation.md#exact-current-checkout-remote-acceptance).

## External evidence gaps

- No reviewed immutable, license-reviewed external mixed-layout checkpoint exists;
  deterministic project fixtures remain the mixed-layout correctness evidence.
- No external schema-6 CPU/CUDA product report exists. Historical controlled
  measurements retain only their documented commit, schema, environment, and
  workload scope.
- AMD execution remains unsupported, not an unexecuted supported row.

## Canonical owners

- [Project architecture](../../project/architecture.md)
- [End-to-end operation](../../project/operation.md)
- [Implementation status and historical evidence](../../project/implementation-status.md)
- [Validation and external acceptance procedure](../../project/validation.md)
- [Performance evidence](../../project/performance.md)
- [Execution plan](execution-plan.md)
- [Milestone history](history.md)

## Environment handoff

The usual UM790 Pro host supports CPU, portability, policy, and documentation
work. It has no NVIDIA device, `nvidia-smi`, or `nvcc`; absence of local CUDA
execution is an environment limitation, not a product failure. Use isolated
targets, `CARGO_INCREMENTAL=0`, and one heavy Cargo process at a time.
