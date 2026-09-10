# Coordinate the external interoperability sprint

You are the coordinator for the [sprint](README.md). Carry its assigned outcome through execution,
independent review, integration, and closure. Preparing this prompt alone does not start that work.

Read [AGENTS.md](../../../../AGENTS.md) in its required order, the sprint README,
[workflow](../../workflow.md), [office procedure](../README.md), and the
[implementation](../../practices/implementation.md) and
[documentation](../../practices/documentation.md) practices. Use the
[external evidence guide](../../../guides/external-evidence.md) and
[ADR 0027](../../../decisions/0027-controller-final-entry-reservations.md) to distinguish this
qualification from later production controller activation.

## Responsibility

Own assignment state, shared-file coordination, the source candidate being qualified, accepted
coverage, and final documentation. Use the [execution prompt](01-execute.md) for the complete
implementation/evidence responsibility and the [review prompt](02-review.md) for independent
acceptance. You may also execute the first assignment; do not describe your own review as
independent. Assign delegation only when authorized, and do not create agent tasks just because
prompts exist. If a separate reviewer has not been assigned, hand off the concrete candidate for
that review and keep acceptance pending.

At startup inspect Git status, current source and evidence, and the whiteboard overview. Recheck
the preparation baseline against the actual checkout. Set real owners and dependencies in the
sprint README. Coordinate shared canonical docs and Cargo jobs before work overlaps.

## Carry the sprint through

1. Resolve the execution assignment's missing operator inputs without repeating existing
   authorization or asking for secret values. Keep useful preparation moving while an input is
   pending. If resources remain unavailable, preserve the exact blocker and resume point rather
   than filling the sprint with unrelated cleanup.
2. Agree the candidate before the qualifying run. The harness rejects a dirty checkout. Arrange
   a clean checkout of the reviewed candidate and build its harness and daemon there; preserve
   other contributors' work. Do not quietly select an older commit or discard changes to satisfy
   cleanliness. Required fixes must be included in the tested candidate.
3. Review the execution handoff for the report location, exact source and binary provenance,
   checks, remaining findings, and proposed canonical-doc updates. Keep the report and sensitive
   session data outside tracked documentation. Arrange a separate review using the prepared
   prompt, with sufficient private evidence access to assess its claims.
4. Return concrete acceptance defects to the execution owner. Reverify checks affected by fixes;
   rerun real qualification when executable code or relevant profiles change. Do not treat an
   earlier report as evidence for a different executable candidate. Pure final documentation
   changes need the documentation checks and a clear link to the actual tested source.
5. Integrate accepted explanations into their existing owners. Update status with the tested
   identity, platform, resource identities safe to disclose, result, and limits; update evidence
   instructions only if the procedure changed. Remove the first roadmap item only after its
   qualification is accepted. Preserve the separate controller activation prerequisites.

## Verification, handoff, and closure

Apply the [verification policy](../../workflow.md#choose-verification-for-the-change) to the
integrated diff, including others' changes being integrated. Reuse unchanged successful checks
with their provenance; rerun for changed code, failed checks, or unresolved concerns. Run
`git diff --check` and the documentation contracts after final edits and sprint removal.

Keep one current coordinator handoff only if needed, recording accepted assignments, unresolved
acceptance criteria, evidence locations, and the next action. Do not append a progress diary or
copy product status into the sprint.

Stop successfully when every sprint acceptance criterion is met, the reviewed evidence has an
agreed retained destination, canonical documentation is accurate, and office cleanup is verified.
Follow [close and remove](../README.md#close-and-remove-a-sprint); do not start another sprint
automatically. If blocked, report the concrete unmet criterion and required input without calling
the sprint qualified or installing the controller lifecycle.
