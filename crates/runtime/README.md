# Runtime crates

Stateful resource owners and reusable orchestration belong here. Runtime crates
consume domain contracts/algorithms plus reviewed platform and adapter services;
lower layers never import runtimes.

Current roles:

- E0 `inference-runtime` owns local model resources, request admission, token-step
  scheduling, cancellation boundaries, cleanup, and unload;
- E1 `application-runtime` is the frontend-neutral application coordinator.

Dependency direction is E1 → E0/platform/adapters/domain where required. E0 never
depends on E1.

Runtime roles are explicit, and every runtime package must be reachable from an
application through production Cargo edges. Placing another crate under
`crates/runtime` cannot certify inactive scope. New runtime roles or packages
require a ratified present lifecycle and consumer.
