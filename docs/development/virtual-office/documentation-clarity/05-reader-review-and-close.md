# Phase 05: Review the reader experience and close

Use the [sprint plan](README.md), [documentation standard](../../engineering-rules.md#7-documentation),
and repository reading order. Review the integrated result of phases 01–04 when their coverage is
ready; their authors need not obtain separate acceptance of every package first. Use a reviewer
who did not author the material for independent acceptance, and identify any self-review honestly.
This prompt does not automatically create another agent.

## Read before inspecting implementation

Start from the entry documentation and follow these connected reader tasks:

- Describe a workflow, make a meaningful context choice, and explain how the request relates to
  what an attempt actually receives.
- Submit authorized work, follow its execution and stored outcome, then reason about a lost reply,
  interruption, or prospective change.
- Follow a capability requirement into a local, model, or peer adapter; explain who owns execution,
  cancellation, and the remaining uncertainty.
- Use a maintained operator recipe, inspect the result, and find recovery and qualification guidance.

Judge whether the docs teach the purpose and relationships and help the reader choose what to do.
A complete list of constructor checks does not answer that question. Note where the reader has
to invent a connection, jump to implementation, or wade through repetitive detail.

Check accuracy against source, consumers, tests, and the relevant canonical owner afterward.
Inspect the earlier contracts, context, and persistence edits as part of this review; neither their
test results nor their status in the former plan establish readability. Sample simple constants,
getters, important types/traits, errors, and private/test comments as well as the main walkthroughs.

Ask whether a comment belongs at all, whether detail has the right home, and whether an example or
diagram explains something better than prose. Check arrows, labels, and omitted steps against the
actual behavior. Do not demand diagrams, extra paragraphs, or wholesale rewrites of already clear
material. State each finding as a concrete reader problem or incorrect claim with a finite fix.

## Check coverage and resolve findings

Compare the current Cargo workspace and maintained document areas with the sprint table.
Every package needs a useful README and appropriate rustdoc entry. Account for all owning modules,
including material inspected and retained; scenario sampling supplements that coverage and cannot
replace it. Identify genuine gaps without demanding a per-symbol checklist.

Resolve ordinary editorial findings within the authorized review and coordinate shared edits.
Return substantial gaps to the owning assignment for correction, then recheck what changed.
Separate unresolved implementation questions from documentation defects using the findings policy.
No invented issue or whiteboard quota is part of acceptance. An explicitly accepted deferral must
remain identified as unfinished with an owner.

## Verify, digest, and remove

Review the integrated diff for excluded executable/API/data changes and unsupported qualification
claims. Apply the [verification policy](../../workflow.md#choose-verification-for-the-change) to
the actual changes, grouping relevant checks and reusing earlier results only when their inputs
remain unchanged. Record what ran against which tree, any limits, and rendered-document inspection.

After coverage, reader review, and verification pass, put useful lasting explanations in their
existing owners. Update status or roadmap only when a substantiated fact or accepted product task
changed. Preserve unresolved whiteboard topics and replace their temporary links. Follow
[close and remove a sprint](../README.md#close-and-remove-a-sprint) to delete the sprint notes and
fix links. Report the result and any accepted follow-up without copying the execution log into
permanent docs or starting another sprint.
