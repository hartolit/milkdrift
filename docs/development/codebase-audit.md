# Codebase audit

This audit applies `docs/development/engineering-rules.md` to the current production source,
workspace manifests, tests, repository contracts, and canonical architecture/product documents.
It is an actionable engineering review, not a second owner for product status or roadmap facts.

## Priority summary

| Priority | Finding | Main rule at risk |
| --- | --- | --- |
| Medium | `CapabilityAdapter` has several production implementations but no reusable conformance suite | Prove and enforce the rule |
| Medium | Cohesion review is deferred until the 2,000-line backstop | Organize by responsibility; review large files |

## Findings

### 1. Medium — The open adapter interface has no shared conformance suite

`CapabilityAdapter` is an open production interface with lifecycle, exact execution,
cancellation, health, and authority semantics (`crates/capability-host/src/adapter.rs:280-319`). It
has four distinct production implementations:

- `LocalProcessAdapter` (`adapters/local-process/src/process.rs:625`);
- `ModelEndpointAdapter` (`adapters/model-provider/src/adapter.rs:953`);
- `RemoteCapabilityAdapter` (`adapters/peer-http/src/remote.rs:319`);
- `WorkflowControlAdapter` (`crates/control/src/adapter.rs:138`).

The capability-host tests exercise fake adapters and each concrete adapter has focused tests, but
there is no reusable suite that every production implementation runs. Lifecycle defaults are
no-ops, and concrete implementations currently differ in start/drain/shutdown state handling. That
may be valid where semantics genuinely differ, but the shared contract is not independently proved
and those differences are not named by the interface.

Recommended correction:

1. Define the minimum common lifecycle/execution invariants precisely: exact invocation identity,
   observation sequence/terminal shape, cancellation correlation, supplied health timestamp,
   post-drain admission behavior, and shutdown ownership.
2. Add a reusable factory-driven conformance suite under capability-host test support and run it
   against all four production adapters, with mechanism-specific fixtures layered on top.
3. Replace default lifecycle methods with required methods unless a no-resource lifecycle is an
   explicit, tested semantic variant.

### 2. Medium — The cohesion guard starts much later than the engineering rule

The engineering rules require a cohesion review as production files approach roughly 1,000 lines.
The repository contract in `tools/evidence/tests/repository_contracts.rs:432-452` enforces only a
hard `< 2,000` line limit. There are currently 23 production files at or above 1,000 lines. A
diagnostic Clippy pass over workspace library/binary targets with `too_many_lines` and
`cognitive_complexity` enabled reports 127 long functions across 85 files and 12 high-complexity
functions.

The counts are diagnostic, not a request to split exhaustive reducers mechanically. The clearest
first review targets are responsibilities that already have natural phases or command families:

- `apps/cli/src/main.rs:377` — a 438-line command dispatcher, cognitive complexity 62;
- `crates/runtime/src/context/source/discovery.rs:19` — a 472-line function combining projection
  seeding, history reconstruction, event classification, explicit-source resolution, branch/join
  exposure, and final policy validation;
- `apps/daemon/src/host/commands.rs:16` — a 347-line external command dispatcher;
- `apps/daemon/src/host/attempts.rs:190` — a 239-line historical reconstruction path, cognitive
  complexity 34;
- `adapters/redb-store/src/admin/service.rs:120` — a 266-line integrity phase driver, cognitive
  complexity 34.

Recommended correction:

1. Replace the single backstop with a reviewed-exception mechanism: fail new/expanded production
   files above the review threshold, and require a local `#[expect]` rationale for cohesive long
   functions that should remain intact.
2. Refactor the listed dispatchers into private command-family or phase functions. For context
   discovery, introduce one private state object and separate projection seeding, bounded journal
   folding, explicit-source completion, and final validation without changing the owning boundary.
3. Do not split the exhaustive projection event reducers solely to satisfy a metric; review them
   for duplicated transition mechanics first, as required by the engineering rules.

## Suggested remediation order

1. Establish adapter conformance tests.
2. Apply targeted cohesion refactors and strengthen the repository guard.

The remaining items are independently shippable cleanup slices and should not be bundled into one
large architectural rewrite.
