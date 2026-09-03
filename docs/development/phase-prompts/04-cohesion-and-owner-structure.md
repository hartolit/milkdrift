# Pass 4 — Cohesion enforcement and private owner structure

Use this prompt with `00-shared-execution-contract.md`.

Run this pass after the repository’s existing controller-admission implementation and independent closure prompts have been applied, or after an explicit decision to leave that work gated. Reinspect the resulting source rather than relying on old line numbers.

## Objective

Apply the cohesion rule from `engineering-rules.md` to the current production hotspots. Replace oversized multi-phase functions and dispatchers with clear private owner structure, then strengthen repository enforcement so future agents cannot silently grow files until the 2,000-line emergency backstop.

This is a structural pass. Preserve semantics, durable bytes, wire contracts, command results, error classifications, and lifecycle ownership unless a duplicated rule discovered during the refactor must be consolidated to preserve existing behavior.

## 1. Re-run the diagnostic inventory

Before editing:

- inventory production Rust files at or above roughly 1,000 lines;
- run diagnostic `clippy::too_many_lines` and `clippy::cognitive_complexity` checks without turning raw metric output into policy;
- inspect current diffs from preceding passes;
- identify functions that own multiple separable phases versus exhaustive reducers that remain cohesive.

The 2026-09-01 audit named these first targets. Review their current successors even if paths or line numbers changed:

- the CLI command dispatcher;
- runtime causal-context source discovery;
- daemon external command dispatch/planning;
- daemon current/historical attempt reconstruction;
- redb administrative integrity phase driving.

Complete all still-applicable targets in this pass. Do not merely add exceptions around them.

## 2. Refactor by responsibility, not file size

### CLI dispatcher

Create a small root composition/argument owner and private command-family modules. Keep shared concerns—credential loading, request envelope construction, confirmation, bounded document input, output, error mapping—owned once. Command modules must call the same `ControlClient` and canonical document libraries; they must not acquire storage or domain-state ownership.

Do not add new CLI behavior in this pass beyond changes required to preserve existing behavior under the split. Pass 5 owns command expansion.

### Runtime context discovery

Retain runtime as the one context-discovery owner. Introduce one private state object or similarly cohesive mechanism, then separate:

- projection seeding;
- bounded durable journal folding;
- event/candidate classification;
- explicit-source completion;
- branch/join/subworkflow exposure;
- authority/sensitivity/budget validation;
- final deterministic ordering and omissions.

Do not create a generic context plugin system, duplicate candidate representations, change selection semantics, or materialize unselected content. Preserve canonical manifest/digest output and restart behavior exactly.

### Daemon command planning

Separate external protocol adaptation, common envelope validation, command-family planning, authority/resource fact construction, owner-queue execution, and public result mapping. One command must still traverse one normal authorized path. Do not create per-route business rules or another operation/result enum mirroring the typed owner queue.

### Attempt reconstruction

Separate current projection lookup, bounded historical journal reconstruction, context/provenance attachment, authority filtering, and public read mapping. Preserve one canonical attempt meaning for current and historical reads; do not introduce a faster but semantically different path.

### Redb administrative driver

Separate each integrity phase behind private functions/state with explicit inputs and typed results. Keep one administrative owner and one transaction/refusal policy. Do not distribute integrity policy into table modules merely to shorten the driver.

## 3. Remove boilerplate before moving it

Before splitting any target, search for repeated validation, dispatch, error mapping, event folding, cursor handling, or phase bookkeeping. Consolidate shared meaning with the smallest private abstraction. Do not copy the same switch into several child modules.

Parent modules should read as ownership maps, not forwarding façades. Avoid:

- `include!`;
- `mod.rs`;
- wildcard re-exports;
- `use super::*` in production;
- `part1`, `helpers`, `common`, or numbered modules;
- one-method public traits created solely to move code;
- arbitrary line-based splits.

Use `owner.rs` with named `owner/child.rs` modules following repository policy.

## 4. Strengthen cohesion enforcement

Replace the current “only fail at 2,000 lines” posture with a reviewed-exception mechanism aligned with the roughly 1,000-line review rule.

Required behavior:

1. New or expanded production files crossing the review threshold fail repository contracts unless explicitly reviewed.
2. Exceptions are few, exact, visible, and carry a meaningful rationale and bounded ceiling. A global wildcard or permanent blanket allowance is forbidden.
3. Long cohesive functions that must remain intact use local `#[expect(..., reason = "...")]` or an equally visible source-local rationale rather than global lint suppression.
4. Existing exhaustive reducers are not split solely to satisfy a metric; their exception must state the cohesive invariant they preserve.
5. The 2,000-line hard backstop remains or becomes stricter.
6. Test/evidence code is reviewed separately so generated or exhaustive fixtures do not weaken production policy.
7. Repository-contract tests cover the enforcement mechanism itself, including missing, stale, duplicate, over-broad, and exceeded exceptions.

Do not add a large policy framework. A small explicit rule in the existing repository-contract owner is sufficient.

## 5. Prove semantic equivalence

For each refactored target, add or retain focused tests that compare externally observable behavior before and after the split:

- CLI command parsing, request payload, JSON output, errors, and exit codes;
- context manifest bytes/digest, selected sources, omissions, ordering, and failure classifications;
- daemon command idempotency, authority, receipts, and public result mapping;
- current versus historical attempt reads and bounded scans;
- redb integrity detection, ordering, rollback, and refusal.

No durable fixture or protocol version should change in a pure cohesion pass. Any unexpected byte change must be investigated rather than accepted.

## 6. Evidence

Run all focused suites for the five owners, `repository_contracts`, public API inventory for any touched library root, the operational evidence smoke lane, and the full local gate.

Run relevant mutation shards if logic was consolidated while splitting. A move-only claim does not excuse missing mutation coverage when conditionals changed.

## Scope exclusions

Do not expand the CLI command set, change context policy, implement new provider behavior, alter controller admission, introduce UI, or use this pass as a broad package rewrite.

## Acceptance criteria

The pass is complete only when:

- every still-applicable named hotspot has a clear private phase/command-family structure;
- no duplicate rule was spread into child modules;
- the public surface and durable/wire behavior remain stable;
- repository contracts enforce review near 1,000 lines with exact exceptions;
- focused, repository, operational-smoke, mutation-as-applicable, and full gates pass.
