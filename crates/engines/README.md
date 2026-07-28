# Engine crates

Stateful resource owners and reusable orchestration belong here. Engines consume
features and adapters; lower layers never import engines.

Current roles:

- E0 `inference-runtime` owns local model resources, request admission, token-step
  scheduling, cancellation boundaries, cleanup, and unload;
- capability engines own independently stateful reusable behavior with a reason to
  change separate from E1; `corrective-workflow` is the first such engine;
- E1 `application-runtime` is the frontend-neutral application coordinator.

Dependency direction is E1 → capability engines/E0 and capability engine →
E0/adapters/features where required. Capability engines and E0 never depend on
E1. New engines still require a coherent lifecycle, ownership, replacement, test,
or reuse boundary; this role is not permission to create crates for every feature.
