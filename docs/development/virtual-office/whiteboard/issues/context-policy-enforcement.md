# Context policy declarations and runtime enforcement

Source inspection at `b679811` found gaps in session agreement, required evidence, and omission
metadata. The corrected behavior and retained-history rules are now owned by
[ADR 0031](../../../../decisions/0031-context-enforcement-and-retained-evidence.md).
The assessment below preserves the evidence that motivated this work.

## Assessment at preparation

`ContextSessionPolicy` participates in the blueprint policy digest, and prompt-sequence
compilation sets it. Runtime dispatch, however, does not consult `TaskContextPolicy::session` or
compare it with the supplied model request. Model-provider negotiation consults
`ModelTaskRequest::session` instead, accepting only `Fresh` in the current mappings. A blueprint
continuation declaration alone therefore neither arranges nor enforces continuation.

Sequence compilation currently creates `process.execute` tasks and carries session intent in
their stage data as well as context policy. That path does not supply a `ModelTaskRequest`.
Enforcing agreement for model invocations therefore needs to preserve the separate process-stage
contract rather than requiring every capability to use a model session shape.

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
architecture and status distinguish the required semantics from these current limits.

2026-09-08 — Alder-20260908-c (agent pseudonym), documentation-clarity phase 03: rechecked the
runtime dispatch/selection boundary and model-provider's manifest loading and `negotiate` path
at `52cf340` with documentation-only edits. The provider checks `ModelTaskRequest::session`
and accepts only `Fresh`; it does not compare blueprint session intent or redo selection checks.
Host data access verifies selected references and uses supplied read authority without evaluating
the actor's grant anew. Package/API explanations preserve these distinctions. No executable fix
or new combined-case regression test was added.

2026-09-09 — Ash-20260909-a (agent pseudonym), context/import sprint preparation: rechecked
selection, eligibility, omission construction, dispatch, candidate construction, and session
contracts at `b679811`. All three gaps remain in source. Dispatch's retry branch reads and rebinds
the prior manifest without invoking the selector, so regression evidence must cover reuse as well
as fresh selection. Existing excluded-exact-source tests also preserve the complete
required/exact/fail-closed conjunction; fixing stopped selection must not widen unrelated refusals.
The sequence compiler's process-stage data and the model request's session variants are distinct
contracts to trace during repair. This preparation adds no executable fix or failure-injection
evidence; the conclusions are from source and test inspection.

2026-09-10 — Reed-20260910-a (agent pseudonym), context/import phase 01: implemented the
corrections at the selector, retained-manifest checks, and effect-claim boundary. Production tests
cover saved omission redaction, legacy retry/reopen refusal, both model request forms, and old
category-free snapshots. The [sprint handoff](../../context-and-import-corrections/README.md)
records the checked tree, evidence, and compatibility review points.
