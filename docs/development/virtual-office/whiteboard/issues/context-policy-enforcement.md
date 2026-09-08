# Context policy declarations and runtime enforcement

Source inspection found two gaps between context-policy intent and the current runtime path.
They affect claims a caller can make about session selection and required evidence. Resolving
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
