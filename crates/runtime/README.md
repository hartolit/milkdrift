# Runtime crates

Stateful resource owners and reusable orchestration belong here. Runtime crates
consume domain contracts/algorithms plus reviewed platform and adapter services;
lower layers never import runtimes.

Current roles:

- E0 `inference-runtime` owns local model resources, request admission, token-step
  scheduling, cancellation boundaries, cleanup, and unload;
- capability engines own independently stateful reusable behavior with a reason to
  change separate from E1; `corrective-workflow` is the first such engine and is a
  bounded data-defined reference capability rather than the general workflow runtime;
- E1 `application-runtime` is the frontend-neutral application coordinator.

Dependency direction is E1 → capability engines/E0 and capability engine →
E0/platform/adapters/domain where required. Capability engines and E0 never depend
on E1.

Runtime roles are explicit: placing another crate under `crates/runtime` does not
make it a capability engine. Production runtime-to-platform/adapter and
runtime-to-runtime edges also require an exact reviewed composition entry. New
runtime crates still require a coherent lifecycle, ownership, replacement, test,
or reuse boundary.
