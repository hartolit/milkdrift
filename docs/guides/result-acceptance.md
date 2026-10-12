# Verify task results before proceeding

A completed external call does not establish that its answer is correct or that repository checks
passed. Give consequential verification to a configured verifier task with explicitly selected inputs
and an output contract. Ordinary task definitions declare named outputs with required presence,
optional exact media type and a byte ceiling. A nominally successful capability that omits or violates
one of those outputs leaves the task failed. The original terminal evidence remains retained.

A producer-to-consumer `task_output` binding waits for the required immutable output and freezes its
identity at the consumer's start. `settled_outcome` deliberately makes a failure available to review.
Neither a model's prose nor a displayed success badge grants authority to edit, adopt or release work.
The actor must submit the corresponding typed change through the shared daemon path.

For a failed required obligation, add an explicitly linked corrective task. After it completes,
`adopt_correction` checks that its outputs satisfy the original declaration. Adoption preserves the
failed task and can redirect only explicitly named, version-guarded unstarted consumers. A workflow
becomes complete only after its finish request and required obligations are satisfied. Uncertainty
remains distinct from failure and cannot be relabelled as a harmless retry.

The [external evidence harness](external-evidence.md) exercises a concrete version of this pattern:
a verifier retains a failed check but omits its required verification attestation; independent review
runs from that settled outcome; a new coding task and verifier produce a successful correction;
the operator's explicit adoption satisfies the original obligation without rewriting it.

The old `workflow.accept_result` built-in and graph branches were retired under
[ADR 0051](../decisions/0051-daemon-task-workflows.md). Current definitions and change shapes belong
to the [task API](../reference/control-api.md). Inspect selected task state, reason, frozen inputs
and result permissions before treating output as usable. Download complete artifacts when a text
preview is truncated; a preview digest covers complete retained text, not only displayed characters.
