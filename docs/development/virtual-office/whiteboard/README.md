# Office whiteboard

Keep broader implementation issues and improvement discussions here while they need investigation.
Topics may remain through many sprints. Use the
[scope and findings policy](../../engineering-rules.md#findings-beyond-the-assignment) to decide
what belongs here; [the office procedure](../README.md) handles sprint coordination and cleanup.

## Overview

| Topic | Kind | State | Next action or assigned sprint/task | Last evaluation |
| --- | --- | --- | --- | --- |
| [Context policy enforcement](issues/context-policy-enforcement.md) | issue | open | Investigate session intent, required evidence after selection stops, and omission-reason precedence bypassing metadata redaction; executable fixes are outside the documentation sprint. | 2026-09-08 |
| [Prompt-sequence import version label](issues/prompt-sequence-version-label.md) | issue | open | Correct the generated revision reason in a focused executable change, reviewing its effect on revision identity and exact-byte evidence. | 2026-09-08 |
| [Process reporting cleanup](issues/process-reporting-cleanup.md) | issue | open | Ensure child termination and I/O joining on every post-spawn reporting failure, with bounded lifecycle regression evidence. Executable fixes are outside the documentation sprint. | 2026-09-08 |

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
