# Phase 03: Explain how work executes and survives interruption

Work toward the reader outcome assigned for this phase in the [sprint README](README.md).
Read [AGENTS.md](../../../../AGENTS.md) in its required order, the
[documentation standard](../../engineering-rules.md#7-documentation), and the
[virtual-office procedure](../README.md). Use the reviewed phase 01 example and the assigned
reader outcome; coordinate related explanations without waiting for unrelated packages.

## Assignment

Work in the assigned portion of `crates/persistence`, `crates/runtime`, `crates/capability-host`,
`crates/control`, or `crates/prompt-sequence`. Add or complete its README and crate introduction,
then review public documentation and consequential private/test comments needed for that outcome.
Read adjacent owners and tests before describing their interaction; coordinate supporting edits
under the scope policy.

Use a concrete operation to explain the package's role:

- Persistence: what a caller asks a storage implementation to commit, what must commit together,
  and what evidence remains available after a failure or replay.
- Runtime: how an accepted command makes work eligible, an attempt starts, observations arrive,
  and recovery determines what may happen next.
- Capability host: how a generation is registered, selected and claimed, how the adapter runs,
  and who owns cancellation, workers, and shutdown.
- Control: how a proposal is checked and applied through existing authority/runtime operations;
  distinguish library controller behavior from production availability.
- Prompt sequence: how input becomes a revision, which existing runtime operations execute it,
  and how a failed stage can lead to a later proposed change.

Explain important ordering with the event or failure it prevents. Describe when a call has saved
a fact, when external work may already have happened, and which outcomes remain unknown. Give
trait implementers explicit obligations and callers useful errors and recovery guidance. Do not
replace uncertainty with a claim of automatic retry safety. Name ownership and shutdown outcomes
instead of listing lifecycle adjectives.

Large areas such as runtime context, scheduling, projection, recovery, and structured execution
need separate assignments. Explain one path fully and link to related owners; do not duplicate
the architecture document or turn the README into an index of every symbol.

## Verify and stop

Use [workflow](../../workflow.md#choose-verification-for-the-change) to select checks, including
relevant durable/restart or failure tests needed to support the explanation. A prose change does
not justify new durability or interoperability claims.

Hand off reviewed scope, the supported operation and failure path a reader can now follow,
source/test evidence, check results, unresolved findings, and the next portion to assign. Stop
there. Do not refactor a difficult implementation to make it easier to describe or start phase 04.
