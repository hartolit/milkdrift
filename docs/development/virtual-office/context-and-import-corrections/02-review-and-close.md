# Review the corrected operation, explain it, and close the sprint

Execute assignment 02 of [this sprint](README.md) against the implementation checkpoint. Read
[AGENTS.md](../../../../AGENTS.md) in its required order, the
[documentation practice](../../practices/documentation.md), the
[implementation practice](../../practices/implementation.md) for the correctness review, and the
[workflow](../../workflow.md). Read the current handoff and diff; a phase-completion label is not
evidence that its acceptance criteria were met.

## Review the complete behavior

Follow the corrected operation from task and sequence declarations through candidate discovery,
selection, saved manifests, retry/restart, invocation construction, and the relevant consumers.
Assess the [sprint acceptance criteria](README.md#acceptance) against actual tests and call paths.
Pay particular attention to combinations where one early decision can hide a later obligation:
stopped selection with required evidence, policy exclusion with protected metadata, and retained
requests with session validation.

Check that the evidence covers persisted/readable results and external-entry refusal, not only
helpers or selected enum variants. Check that the import reason correction accounts for new
revision identity without altering historical bytes. Confirm that model-session checks do not
break the separate process-stage contract or imply unsupported continuation features.

Correct remaining defects within the sprint's accepted responsibility, including their callers
and tests. This assignment includes those implementation corrections; it is not limited to prose.
Use the [findings policy](../../workflow.md#findings-beyond-the-assignment) for work that actually
requires a broader decision. Do not defer an ordinary acceptance gap merely to produce a whiteboard
entry, and do not broaden the sprint into a new context or session architecture.

## Explain the result to its readers

Read the maintained explanations as a newcomer before checking them against source. A reader
should be able to follow which prior evidence a task requests, why the runtime includes, omits,
or refuses it, what the invoked capability receives, and what remains frozen on retry. Explain
where session intent is enforced and how that differs from support for a continuation protocol.
For sequence imports, make the version and identity consequences understandable at the owning
compiler or import explanation.

Review the affected introductions and API docs across blueprint, runtime, model, model-provider,
and prompt-sequence as one related explanation. Follow the changed relationships into the
[architecture](../../../architecture.md#context-and-artifacts),
[ADR 0011](../../../decisions/0011-causal-context-manifests.md),
[sequence reference](../../../reference/prompt-sequence-v2.md), and relevant operator examples or
guides. Keep detail at its existing owner and link to it; do not copy an end-to-end manual into
every package or comment. Let the documentation practice guide examples, diagrams, and subtraction.

Remove stale descriptions of fixed gaps only where executed evidence supports the stronger claim.
Keep any remaining limitations, including historical-data and capability-specific behavior,
explicit in their maintained owners. Update [status](../../../product/status.md) with the resulting
behavior and qualified evidence. The roadmap's external interoperability and controller activation
milestones remain open; this sprint does not qualify them.

## Verify, reconcile, and remove temporary work

Apply the [verification policy](../../workflow.md#choose-verification-for-the-change) to the final
changes. For prose-only changes, run documentation contracts and inspect formatting. Rust comments
and examples also require formatting, affected doctests, and warning-denying rustdoc. Executable
corrections require the full gate for the corrected tree. Reuse successful phase-01 evidence where
it remains applicable; do not rerun an unchanged full gate merely because this is another phase.

Once acceptance is established, record the disposition of both whiteboard issues with supporting
evidence, retain durable facts in their existing source/docs owners, and remove the resolved topic
files and overview rows. If a topic cannot be closed, keep its precise remaining gap and an explicit
accepted disposition; do not declare the sprint complete while an acceptance gap remains unresolved.

Follow the [office close procedure](../README.md#close-and-remove-a-sprint): remove this sprint
directory and its office entry, repair references to temporary files, and run documentation
contracts after cleanup. Keep raw evidence under ignored `target/` paths or CI artifacts; Git
retains the committed history. Finish with the corrected behavior, checks actually run and their
limits, and any accepted follow-up. Stop without opening another sprint or starting roadmap work.
