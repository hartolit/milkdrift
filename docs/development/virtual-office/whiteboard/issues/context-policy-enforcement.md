# Context policy declarations and runtime enforcement

Source inspection found gaps between context-policy intent and the current runtime path.
They affect claims about session selection, required evidence, and omission metadata. Resolving
them requires executable changes and regression tests, which are excluded from the documentation
sprint.

## Current technical assessment

`ContextSessionPolicy` participates in the blueprint policy digest, and prompt-sequence
compilation sets it. Runtime dispatch, however, does not consult `TaskContextPolicy::session` or
compare it with the supplied model request. Model-provider negotiation consults
`ModelTaskRequest::session` instead, accepting only `Fresh` in the current mappings. A blueprint
continuation declaration alone therefore neither arranges nor enforces continuation.

With `StopAtFirstOverflow`, the selector sets `stopped` after an optional candidate exceeds a
budget. For a later eligible candidate, `consider` takes the early omission branch before the
required/availability/authority/budget checks. That candidate is recorded as `SelectionStopped`
even if it is required and `fail_closed` is true. This conflicts with the general required-loss
rule in [ADR 0011](../../../../decisions/0011-causal-context-manifests.md); it must not be silently
promoted to the intended contract. `OmitOversized` continues the required-candidate checks.

Omission redaction also depends on the selected reason. The `omission` helper in
[context.rs](../../../../../crates/runtime/src/context.rs) clears source and sizes only for
`BranchIsolated` and `AuthorityDenied`. Once stopped, `consider` substitutes `SelectionStopped`,
even for a later optional branch-isolated candidate. Separately, `eligibility` checks category
exclusion before scope visibility, allowing `ExcludedCategory` to take precedence. Candidates
that are not selected can therefore retain protected reference metadata in the manifest.

This is a source-derived disclosure path, not an executed end-to-end exploit. Production
[candidate construction](../../../../../crates/runtime/src/context/source/candidate.rs) retains
source references even when workspace authority is denied; artifact facts can also carry the
referenced size. Selection does not load those omitted bytes, but metadata redaction needs to
be independent of the omission reason. The existing optional-overflow and branch tests do not
combine those cases. A follow-up should cover stopped/excluded candidates with denied authority
and hidden scopes, including their saved manifest and adapter/read consumers.

Supporting source and existing tests:

- [Policy defaults, session declaration, and digest](../../../../../crates/blueprint/src/context.rs).
- [Prompt-sequence policy construction](../../../../../crates/prompt-sequence/src/compiler.rs),
  `causal_policy` and `coding_node`.
- [Runtime dispatch](../../../../../crates/runtime/src/engine/dispatch.rs), `invocation_request`;
  [provider negotiation](../../../../../adapters/model-provider/src/adapter.rs), `negotiate`.
- [Selection](../../../../../crates/runtime/src/context/selection.rs), `SelectionState::consider`.
- [Causal-context tests](../../../../../crates/runtime/tests/causal_context.rs),
  `per_item_manifest_and_truncation_boundaries_are_exact` tests stopping with optional items;
  `exact_budget_boundary_and_required_fail_closed_are_stable` tests active required checks.
  Neither establishes required-item refusal after selection has stopped.

## Contributions

2026-09-08 — Cedar-20260908-a (agent pseudonym),
original context documentation assignment (handoff retained in Git at `e93d749`): traced these paths
at base commit `875b564` with documentation edits only. These are source-derived findings;
no new executable regression test was added. The API explanations now disclose the limitations
and show `Fresh` with `OmitOversized`. A follow-up should decide the intended enforcement at the
existing owners and test a contradictory session request and an optional overflow followed by
required evidence. This finding does not authorize another feature or architectural sweep.

2026-09-08 — Rowan-20260908-b (agent pseudonym), documentation-clarity phase 01: rechecked
dispatch, selection, sequence compilation, and model-provider negotiation at `3fb9c68` with
documentation-only edits. Both gaps remain in the current paths. The revised package and API
explanations retain the distinction between declarations and enforcement, and the example uses
`Fresh` with `OmitOversized`. The final source review additionally traced the omission-reason
precedence described above and qualified the redaction claims at the policy/manifest APIs.
No executable fix, new regression test, or end-to-end disclosure experiment was introduced.

2026-09-08 — Codex, documentation-clarity phase 02: rechecked runtime dispatch, candidate
construction, selection, omission construction, selected-content materialization, and provider
session negotiation at `b479d54` with documentation-only changes. All three gaps above remain.
Runtime's package and API explanations now distinguish the selection actually recorded from
policy intent; the materialization helper verifies selected facts but performs no grant evaluation.
Existing causal-context tests cover active required checks and optional stopping separately.
No new regression test or end-to-end disclosure experiment was added. Supporting corrections in
architecture and status distinguish the required semantics from these current limits. Phase 03
should preserve them when explaining host/provider consumption; phase 04 should retain these
qualifications in its shared-guide review.
