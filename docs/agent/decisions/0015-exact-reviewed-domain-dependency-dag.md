# ADR-0015: Use an exact reviewed domain dependency DAG

- **Status:** Accepted
- **Date:** 2026-08-01

## Context

The first vertical slice prohibited every F1-to-F1 production dependency. That temporary rule prevented cycles while crate responsibilities were still being established, but a permanent blanket ban would encourage unrelated vocabulary to accumulate in `domain-contracts` merely to avoid an otherwise honest domain dependency.

A coarse layer matrix is not sufficient by itself. Permitting every downward or peer dependency allowed by a layer can introduce accidental coupling or a cycle without recording why the exact crates need one another. Ownership also became blurred when `TaskId`, a task-graph concept, lived in the shared foundation.

## Decision

Replace the blanket F1-to-F1 prohibition with an exact allowlisted DAG for workspace-local production dependencies wholly inside `crates/domain`. Arrows below point from the dependent crate to its dependency. The complete approved graph is:

```text
tokenization    -> domain-contracts
context-planner -> domain-contracts
sampling        -> domain-contracts
task-graph      -> domain-contracts
```

These four edges are normal production dependencies. `domain-contracts` has no workspace-local production dependency. No domain peer edge and no domain build dependency is currently approved. A coarse layer rule may permit an F1 peer dependency in principle, but the exact-edge registry must reject it until its source, target, dependency kind, and rationale are reviewed. The registry itself must remain acyclic; adding an edge that creates a cycle is invalid.

`domain-contracts` contains only portable engine/backend contracts and stable portable vocabulary genuinely shared across independent domain or runtime boundaries. A type does not belong there merely because one crate uses it, because a future consumer is imaginable, or because placing it there avoids review of the actual dependency. Vocabulary with one coherent domain owner stays with that owner; consumers take an explicit reviewed dependency when needed.

`TaskId` is defined and exported by `task-graph`. It identifies nodes, dependencies, attempts, and state transitions owned by that domain and is not re-exported from `domain-contracts`. Runtime consumers import it from `task_graph` rather than widening the foundation.

The exact domain graph is enforced independently from folder placement. External dependencies and development-only dependencies remain subject to their separate exact review policies and are not implicit additions to this DAG.

## Rejected alternatives

- **Keep the universal F1-to-F1 ban:** this prevents cycles but pressures domain-specific concepts into the shared foundation and cannot express a justified peer edge.
- **Permit every edge allowed by the coarse layer matrix:** a category-level direction rule does not review exact coupling or prevent cycles among peers.
- **Use `domain-contracts` as a general common-types crate:** convenience sharing would erase ownership and make the foundation a dependency magnet.
- **Duplicate or re-export `TaskId` from multiple crates:** more than one canonical path would obscure task-graph ownership and permit identity types to drift.

## Consequences

- The current domain production graph has four explicit edges and one root, with no approved F1 peer edge.
- Future domain coupling requires an exact rationale and an acyclic whole-graph review instead of inheriting permission from a layer name.
- Moving `TaskId` changes its public import path from `domain_contracts::TaskId` to `task_graph::TaskId` while preserving its task identity semantics.
- Shared-foundation growth must be justified by real cross-boundary ownership rather than dependency-policy convenience.
- The portability set remains the five crates named by [ADR-0007](0007-portability-targets.md); a future DAG change must preserve or explicitly review that contract.

## Review trigger

Review when a new domain crate is introduced, an exact domain production edge must be added or removed, a proposed edge would change the graph's acyclicity, or concrete multi-owner pressure shows that vocabulary should move into or out of `domain-contracts`.
