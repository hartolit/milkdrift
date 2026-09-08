# Phase 02: Execute, control, and recover work

Execute the whole phase described in the [sprint plan](README.md), using the
[documentation standard](../../engineering-rules.md#7-documentation) and the repository's required
reading order. The plan owns scope exclusions, coordination, verification, and handoff rules.

## Reader outcome and ownership

A contributor should be able to follow authorized work from a command through scheduling and
execution, explain what is saved, and understand what can happen after interruption or a change
of plan. Own all documentation in `crates/authority`, `crates/control`, `crates/runtime`,
`crates/persistence`, and `adapters/redb-store`, including their introductions, APIs, and meaningful
private/test comments. Reassess the earlier persistence commit work within this whole area.

Start with a concrete command and follow its authority decision, planned events, storage commit,
and resulting work. Then follow an attempt into external execution and its observations back
into durable history. Explain the responsibilities that prevent a command replay from silently
becoming another external action. Read capability-host and daemon consumers at the handoffs;
coordinate related corrections with their documentation owners.

Use interruption to explain the design: what a caller can recover after losing a reply, which
records establish an external outcome, and why uncertain work may need a decision. Show a small
sequence or state sketch where it makes the difference between committed work, failed delivery,
and an unknown external result easier to see. Do not flatten those distinctions into an automatic
retry promise.

Connect grants, control proposals, prospective revisions, and reconciliation to what they permit
next. Distinguish library functionality from the controller lifecycle's production availability.
Explain structured execution and context selection as part of running work, with links to their
definitions, rather than leaving them as an unrelated vocabulary list.

## Complete the area

Cover the rest of the responsibility as well as the command example: scheduling, structured work,
context, observations, projections, recovery, accounts, storage reads/discovery, revisions,
workspace state, artifacts, snapshots, application/peer storage ports, clocks, retention, and
administration. Organize the work internally by related operations; these are not separate tasks
requiring permission or review after each one. Read all owning modules and inspect consequential
private/test comments, retaining what already teaches the code well.

For persistence traits, callers need to understand the operation and implementers need their
actual obligations. Put those obligations with the trait or method that owns them. Rework the
receipt constants and introductions so the purpose of retained command records is clear without
copying each validator into several comments. Explain redb setup through its current ports and
link operator procedures to their maintained home.

Use existing source, failure/restart tests, and ADRs to verify claims. Recheck relevant
[context-policy findings](../whiteboard/issues/context-policy-enforcement.md) without folding an
implementation fix into the documentation assignment. Choose checks under the verification policy;
a passing storage test does not establish filesystem power-loss behavior or exactly-once external
effects.

Read the resulting explanations across the five packages, correct mismatches and repetition, and
hand off the complete area with evidence and any real gaps. Do not stop at the next persistence
port or defer all of runtime because the command walkthrough is finished.
