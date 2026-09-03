# Codebase audit

This audit applies `docs/development/engineering-rules.md` to the current production source,
workspace manifests, tests, repository contracts, and canonical architecture/product documents.
It is an actionable engineering review, not a second owner for product status or roadmap facts.

## Priority summary

| Priority | Finding | Main rule at risk |
| --- | --- | --- |
| Medium | Cohesion review is deferred until the 2,000-line backstop | Organize by responsibility; review large files |

## Findings

### 1. Medium — The cohesion guard starts much later than the engineering rule

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

## Suggested remediation

Apply targeted cohesion refactors and strengthen the repository guard.

The remaining item is an independently shippable cleanup slice and should not be bundled into a
large architectural rewrite.
