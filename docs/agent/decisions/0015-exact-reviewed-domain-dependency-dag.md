# ADR-0015: Use an exact reviewed domain dependency DAG

- **Status:** Accepted
- **Date:** 2026-08-01
- **Enforcement amendment:** 2026-08-10 — Cargo-derived acyclic domain graph under explicit manifest roles

## Context

The first vertical slice prohibited every F1-to-F1 production dependency. That temporary rule prevented cycles while crate responsibilities were still being established, but a permanent blanket ban would encourage unrelated vocabulary to accumulate in `domain-contracts` merely to avoid an otherwise honest domain dependency.

A coarse layer matrix is not sufficient by itself. Permitting every downward or peer dependency allowed by a layer can introduce accidental coupling or a cycle without recording why the exact crates need one another. Ownership also became blurred when `TaskId`, a task-graph concept, lived in the shared foundation.

## Decision

The original decision replaced the blanket F1-to-F1 prohibition with an exact allowlisted DAG. Its accepted graph, shown from dependent to dependency, was and remains the current Cargo graph:

```text
tokenization    -> domain-contracts
context-planner -> domain-contracts
sampling        -> domain-contracts
task-graph      -> domain-contracts
```

The 2026-08-10 enforcement amendment preserves the durable invariant—workspace-local normal/build F0/F1 edges form an acyclic graph—but removes the second hand-maintained copy of every ordinary Cargo edge. Every package now declares an explicit domain role in manifest metadata. The validator checks the role/location contract, rejects upward facilities, derives the complete domain graph from actual Cargo declarations, and rejects cycles. A legal inward or peer edge therefore does not require editing a Rust package-name registry; a role change, external/development exception, or ownership move still requires explicit review.

`domain-contracts` currently has no workspace-local production dependency and no F1 peer edge currently exists. That inventory is Cargo truth, not an allowlist frozen into the validator.

`domain-contracts` contains only portable engine/backend contracts and stable portable vocabulary genuinely shared across independent domain or runtime boundaries. A type does not belong there merely because one crate uses it, because a future consumer is imaginable, or because placing it there avoids review of the actual dependency. Vocabulary with one coherent domain owner stays with that owner; consumers take an explicit reviewed dependency when needed.

`TaskId` is defined and exported by `task-graph`. It identifies nodes, dependencies, attempts, and state transitions owned by that domain and is not re-exported from `domain-contracts`. Runtime consumers import it from `task_graph` rather than widening the foundation.

The exact domain graph is enforced independently from folder placement. External dependencies and development-only dependencies remain subject to their separate exact review policies and are not implicit additions to this DAG.

## Rejected alternatives

- **Keep the universal F1-to-F1 ban:** this prevents cycles but pressures domain-specific concepts into the shared foundation and cannot express a justified peer edge.
- **Permit every edge allowed by the coarse layer matrix:** a category-level direction rule does not review exact coupling or prevent cycles among peers.
- **Use `domain-contracts` as a general common-types crate:** convenience sharing would erase ownership and make the foundation a dependency magnet.
- **Duplicate or re-export `TaskId` from multiple crates:** more than one canonical path would obscure task-graph ownership and permit identity types to drift.

## Consequences

- The current domain production graph has four Cargo-declared edges and one root, with no F1 peer edge.
- Future domain coupling must fit explicit manifest roles and preserve the acyclic whole graph; ordinary legal edges need no duplicate policy entry.
- Moving `TaskId` changes its public import path from `domain_contracts::TaskId` to `task_graph::TaskId` while preserving its task identity semantics.
- Shared-foundation growth must be justified by real cross-boundary ownership rather than dependency-policy convenience.
- The portability set remains the five crates named by [ADR-0007](0007-portability-targets.md); a future DAG change must preserve or explicitly review that contract.

## Review trigger

Review when the domain role vocabulary changes, a proposed edge would change the graph's acyclicity or portability, an exception is proposed, or concrete multi-owner pressure shows that vocabulary should move into or out of `domain-contracts`. Adding an ordinary correctly classified domain crate or legal acyclic Cargo edge does not by itself require a validator match-table change.
