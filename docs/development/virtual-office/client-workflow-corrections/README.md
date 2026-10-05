# Client workflow corrections

This finite assignment completes the eight client-workflow findings and three verification
questions described in the supplied `00-first-execution-prompt.md`, against baseline
`9ec53ab0c2433efef52174f7a9b9d6e37ed07b1b`. The daemon and domain owners retain semantics and
authorization; clients use their public operations. No GUI, new provider or infrastructure
qualification campaign is authorized by this assignment.

One agent owns the coordinated work and the [current handoff](handoff.md). Follow the
[implementation](../../practices/implementation.md) and
[documentation](../../practices/documentation.md) practices and the
[workflow](../../workflow.md). The ordered checkpoints are artifact disclosure (F3), unavailable
model editing (F1), upload identities (F2), URL identities (F5), file lifecycle and draft trust
(F4/V3), complete size admission and aggregate reads (F6/V1), event framing (F7), error meaning
(F8), and upload preparation recovery (V2). Each checkpoint includes all applicable consumers,
regressions, focused compilation/lints and a coherent local commit.

Assignment section 10 owns the one full-system acceptance stage after these checkpoints:
required tool probes and strict gate, workspace tests and discovery, API/fixture review,
actual daemon/CLI and independent-client scenarios, and the combined recovery/permission/export
journey. Earlier focused checks do not establish full-system acceptance. Use the initial commit
as the changed-content secret-scan base. Preserve logs under ignored
`target/client-workflow-corrections/` and package reviewable evidence at closeout.

Finish when F1–F8 and V1–V3 each have an explicit supported disposition, all required final checks
pass on the finished executable source, and canonical documentation explains the behavior and
limits. Remove this temporary directory only after preserving evidence and updating its links.
