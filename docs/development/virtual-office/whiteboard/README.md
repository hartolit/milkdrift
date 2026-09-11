# Office whiteboard

Keep broader implementation issues and improvement discussions here while they need investigation.
Topics may remain through many sprints. Use the
[scope and findings policy](../../workflow.md#findings-beyond-the-assignment) to decide
what belongs here; [the office procedure](../README.md) handles sprint coordination and cleanup.

## Overview

| Topic | State | Next action | Assignment | Last evaluated |
| --- | --- | --- | --- | --- |
| [Configurable generation and thinking policy](discussions/model-generation-policy.md) | open | Decide the task/profile ownership and supported wire mappings for thinking mode, effort, and budgets; distinguish deployment limits from necessary hard ceilings. | Unassigned | 2026-09-11 |
| [External model configuration provenance](issues/external-model-configuration-provenance.md) | open | Define how evidence captures or verifies server-side thinking/context settings and run controlled thinking-on/off cases with recorded budgets. | Unassigned | 2026-09-11 |

This overview alone owns planning state, next actions, assignment links, and last-evaluation dates.
Topic files hold evidence, current technical assessment, and dated contributions. Keep execution
progress in the assigned sprint/task rather than duplicating it here. The coordinator owns shared
overview edits when several contributors work at once.

Use `open` for a topic awaiting a decision or investigation, `parked` when it needs a later
trigger, and `assigned` when a real sprint/task has accepted work. For a parked topic, put the
reason or review trigger in its next-action cell. Proposed work remains open until assigned.

## Record and develop a topic

Search for an existing topic before creating a descriptively named file in
[issues](issues/README.md) or [discussions](discussions/README.md). An issue describes a suspected
or established problem; a discussion can explore an improvement without claiming the code is wrong.

A short entry is enough: explain the problem or question, available evidence, and why it needs
attention beyond the current assignment. Put its next useful step in the overview. The templates
are optional prompts; omit empty sections and add detail as it becomes useful.

Separate observations, assumptions, and proposals. Link relevant source, consumers, tests, or
product rules, recording the code version examined when it matters. For example, a claim that a
crate has too many responsibilities needs concrete examples of difficult changes or mixed
ownership. A label such as “god crate” does not establish the problem. Large logs and generated
reports belong under ignored `target/` or in CI artifacts; retain enough explanation here to
understand the finding without them.

## Challenge purpose before choosing a solution

Every new or substantially revised topic must include questions of purpose that invite a reasoned
challenge. Explain whose problem it addresses, which Milkdrift outcome matters, and why the current
design may be insufficient. A recorded finding or preferred proposal is not a decision to implement
it. Do not infer agreement from earlier agents' confidence or from the topic's presence here.

Make the questions specific to the topic rather than copying a checklist. Useful challenges ask:

- What supported workflow or operator need would improve, and what observation establishes that need?
- Could the current design, a smaller correction, or a change in usage meet it well enough?
- Does the proposal belong in Milkdrift, in an external capability, or with its operator? What
  complexity, authority, compatibility, or maintenance cost would Milkdrift take on?
- What evidence would favor the proposal, and what result would make us reject, narrow, or defer it?

Develop the strongest case for retaining the current design as well as changing it. Contributors
should answer or sharpen the unresolved questions with evidence, counterexamples, and consequences
for the project. Repeating agreement does not advance a topic; disagreement needs reasons too.
Friction should expose assumptions and improve the decision, not manufacture objections or force
every proposal into implementation. Reviewers may conclude that no change is warranted.

## Contributions and agent pseudonyms

Sign substantive agent contributions with a date, a made-up session label such as
`Cedar-20260908-a`, and a task reference when available. Use one label per actual agent session
and identify it as an agent pseudonym. No personality profile or identity registry is needed;
different labels do not establish independent review, authority, or consensus.

Add evidence, an alternative, a counterexample, or reasoning that advances the question.
Distinguish experiments proposed from those actually run. Keep the current assessment concise
and preserve relevant disagreement in the dated contributions. Do not invent contributors,
backdate statements, or reproduce chat transcripts.

## Evaluate and carry forward

During sprint preparation, review the overview and investigate relevant topics using the
[preparation prompt](prepare-sprint.md). Recheck older observations against current code. Consider
smaller corrections and keeping the current design alongside broader alternatives. Update the
assessment when evidence changes; update planning fields only in the overview. Do not mark
unexamined topics as revalidated or force every discussion to a conclusion.

Unresolved topics stay at their existing paths. At sprint close, update affected assignment links
and next actions; unfinished assigned topics return to open or parked. A sprint ending does not
resolve its topics, and age alone is not a reason to discard them.

Close a topic when resolved, superseded, merged, or rejected with a reason. Record the disposition
and its supporting reference in the reviewed change, put lasting facts or decisions in their
existing owners, then remove the topic and overview row and fix inbound links. Git retains
committed history. Keep the board and its templates when removing sprint files.
