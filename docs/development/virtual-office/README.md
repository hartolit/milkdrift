# Virtual office

Sprint directories hold temporary plans and handoffs for work that needs several assignments.
The [whiteboard](whiteboard/README.md) retains broader issues and discussions across sprints.

[Engineering rules](../engineering-rules.md#findings-beyond-the-assignment) govern assignment scope
and findings. [Workflow](../workflow.md#choose-verification-for-the-change) owns verification.
[AGENTS.md](../../../AGENTS.md) identifies the owners of lasting product facts and decisions.

## Current sprints

- [Documentation clarity](documentation-clarity/README.md): phase 01 is ready for review; no
  rewrite result has been accepted.

## Start a sprint

Review the whiteboard overview for topics relevant to the sprint's purpose. Use the
[preparation prompt](whiteboard/prepare-sprint.md) when a topic needs investigation before planning.
The board may be empty, and unrelated topics may remain open.

Give the sprint a README with its outcome, exclusions, work areas, acceptance criteria, and
current assignments. Each assignment needs an owner, a coherent responsibility, expected result,
relevant sources, checks, and a stop condition. Likely files help coordinate edits. Use ordinary
prose or a small table; derive routine details from the user's request instead of requiring a
completed form. Add phase prompts only where they make repeated assignments easier to execute.

Split work where responsibilities or real dependencies differ. A phase number or file count does
not by itself require another handoff or prevent independent work from proceeding.

## Assign, execute, and review

The coordinator maintains assignments and accepted coverage. A worker completes the assigned
outcome, then hands back the result, reviewed scope, checks, and unresolved findings. The same
agent can coordinate and execute when no delegation is needed. Start further assignments only
within the work the user has authorized.

When delegation is explicitly assigned, coordinate shared files before editing them and keep
workspace Cargo jobs with one owner. Do not create further agent tasks merely because a phase
prompt exists. Preserve other contributors' changes.

Keep one current handoff per assignment; update it rather than appending daily reports. Link to
broader findings on the whiteboard. Raw logs and generated inventories belong under ignored
`target/` or in CI artifacts. Reviews assess the result against its acceptance criteria and the
applicable engineering rules. If review stalls, state the unresolved question and seek a concrete
decision or a better-scoped assignment; do not continue an open-ended polishing loop.

## Close and remove a sprint

1. Check accepted coverage and the integrated result using the verification policy. A remaining
   acceptance gap needs a fix or an explicit scope decision; record any accepted deferral as
   unfinished work with an owner and destination.
2. Put lasting explanations and decisions in their existing owners. Preserve useful broader
   findings on the whiteboard, with enough context to survive removal of sprint notes. Update
   affected overview rows and replace links to temporary files.
3. Remove the completed sprint directory and its entry here. Keep this README, the whiteboard,
   and other active sprints. Run documentation link checks after removal.
4. Report the result, evidence, and accepted follow-up work. Git retains committed history; do not
   paste the sprint log into product documents or create another sprint automatically.

Use the same steps for an abandoned sprint, identifying what remains unfinished.
