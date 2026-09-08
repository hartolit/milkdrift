# Phase 06: Review the reader experience and close the sprint

Work toward the reader outcome assigned for this phase in the [sprint README](README.md).
Read [AGENTS.md](../../../../AGENTS.md) in its required order, the
[documentation standard](../../engineering-rules.md#7-documentation), and the
[virtual-office procedure](../README.md). Coverage from phases 01–05 must be accepted. Assign a
reviewer who did not author the portion being reviewed, with named paths and reader scenarios. The coordinator
owns integration and deletion; a review assignment does not authorize an unbounded rewrite.

## Review assignments

First compare current workspace members and maintained documents with the coverage table. Check
that each package has a README and useful crate entry, and that the reviewed portions account for
all source areas, not just selected public types. Record gaps as specific assignments. Generated
inventories remain under ignored `target/`, never in permanent docs.

Review one named cross-package scenario at a time, without starting from the implementation:

1. Build a task with a context policy; explain defaults, selected/omitted inputs, and a refused choice.
2. Follow a command through authority, runtime, a capability, and saved observations; explain an
   interrupted attempt and why a repeated request may replay or conflict.
3. Use an operator recipe to understand setup, result inspection, artifacts, and restart; separate
   software behavior from platform or external-provider qualifications.

Check the explanation against source and tests afterward. Across the scenarios, sample simple
helpers, nontrivial types, traits, constants, errors, and private lifecycle comments. Reader
sampling supplements the completed coverage review; it cannot replace it. Flag vague language,
incorrect relationships, missing use/failure explanations, duplication, and unsupported claims.

Give each finding a location, reader problem, evidence, and a finite correction. The coordinator
assigns and rechecks fixes using the scope policy and the office review procedure.

## Integration and removal

Once review passes, the coordinator:

1. Checks the integrated diff for changes to behavior, public API, dependencies, fixtures, wire
   formats, or qualification claims. Unexpected changes require investigation before acceptance.
2. Verifies the integrated result under the
   [verification policy](../../workflow.md#choose-verification-for-the-change), including affected
   doctests, warning-denying rustdoc, repository documentation contracts, and rendered package/API
   documentation. Records the checked tree, outcomes, limitations, and raw log references. Reuses
   earlier checks only where their inputs and supporting assumptions remain unchanged.
3. Checks coverage and any accepted deferrals against the sprint's completion criteria.
4. Incorporates useful lasting explanations and decisions into their existing owners. Updates
   product status/roadmap only if an established current fact or accepted product task changed.
5. Follows [close and remove a sprint](../README.md#close-and-remove-a-sprint) for digestion,
   whiteboard carryover, deletion, and link checks.

Stop with a concise report of the maintained documentation, checks, and any explicitly accepted
follow-up. Do not preserve a completion diary, create another sprint automatically, or expand
into the product roadmap's unrelated implementation work.
