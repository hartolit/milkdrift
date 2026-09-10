# Independently review external interoperability qualification

You are the independent reviewer for the [sprint](README.md). Assess the execution candidate,
report, and proposed qualification against the sprint's acceptance criteria. You must not have
implemented this candidate or produced its report. A fresh review is separate from the scenario's
own verifier/reviewer processes and does not make the report cryptographically attested.

Read [AGENTS.md](../../../../AGENTS.md) in order, the
[implementation](../../practices/implementation.md) and
[documentation](../../practices/documentation.md) practices, and the
[workflow](../../workflow.md). Read the execution handoff, current diff, source and tests named in
the [execution prompt](01-execute.md), the
[external evidence guide](../../../guides/external-evidence.md), and
[ADR 0027](../../../decisions/0027-controller-final-entry-reservations.md).

## Review responsibility

Decide whether the supplied evidence establishes the assigned real process/model interoperability
boundary for the exact candidate. Review actual data and implementation rather than accepting
the author's summary or report flags. Coordinate Cargo jobs before running checks. Do not change
executable code during acceptance review; return demonstrated defects to its owner so the revised
candidate and affected evidence can be reviewed together.

1. Match the clean source commit/tree, build provenance, daemon binary, platform, and resource
   identities to the report. Check that required fixes are in the tested candidate. Separate
   later documentation edits from executable changes; do not qualify untested executable work.
2. Inspect the real resource declarations and bounded version evidence without exposing private
   values. Confirm the coding process is a real agent using the declared prompt and repository,
   and the model is a real supported endpoint. The generated verifier/reviewer helpers are
   legitimate scenario machinery; they cannot substitute for either real external resource.
3. Check the strict consumer schema and owning report validator, then trace the claimed process
   facts through attempts, verification/review, artifacts, restart, and prospective remediation.
   Confirm distinct invocations and no duplicated entered work. Understand the intentionally
   withheld verification success artifact as the guide's orchestration fault injection.
4. Inspect the model's selected and omitted evidence, frozen manifest, Fresh session, exact
   profile provenance, final response and usage, stream observations where required by the
   harness, output artifacts, and restart facts. A workflow success alone does not establish
   that these response requirements were met.
5. Verify report redaction and digest references using the retained private evidence. Do not
   reproduce credentials, raw prompts/outputs, or repository contents in the review. If the
   evidence required to assess a claim is unavailable, name the unverified criterion and leave
   acceptance pending rather than assuming the report attests itself.
6. Review every integration correction for complete ownership and adoption, appropriate refusal
   paths, bounded behavior, regression evidence, and clear explanations. Check that the
   applicable full gate/focused evidence covers the final executable candidate. Reuse unchanged
   successful checks with provenance; rerun only for a changed result or concrete uncertainty.
7. Read the proposed canonical docs as an operator. Confirm they explain how to reproduce the
   supported use and state only the tested qualification. Ensure the scope freeze and separate
   controller activation gate remain intact, without a new platform, quality, or durability claim.

## Result and stop condition

Produce one current review handoff with the examined candidate/report identities, scope reviewed,
checks actually run or reused, and one disposition: accepted, corrections required, or blocked
on named evidence. Each finding needs a source or evidence reference, its consequence for an
acceptance criterion, and the correction or observation that would resolve it. Do not create
unrelated cleanup work or demand extra external runs merely to repeat unchanged evidence.

Use the [findings policy](../../workflow.md#findings-beyond-the-assignment) for issues outside this
responsibility. Apply the [verification policy](../../workflow.md#choose-verification-for-the-change)
to any review-document edits and run `git diff --check`. Return the disposition to the coordinator;
they own shared canonical-doc integration and sprint removal.

Stop after a supported disposition. If corrections arrive, assess the changed scope and affected
checks without reopening accepted unchanged work. Do not approve on the promise of a future real
run, call a blocked report qualifying, or authorize production controller activation.
