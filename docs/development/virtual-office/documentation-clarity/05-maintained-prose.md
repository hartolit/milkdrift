# Phase 05: Make the maintained prose readable

Work toward the reader outcome assigned for this phase in the [sprint README](README.md).
Read [AGENTS.md](../../../../AGENTS.md) in its required order, the
[documentation standard](../../engineering-rules.md#7-documentation), and the
[virtual-office procedure](../README.md). Use the reviewed phase 01 example and a coherent reader
outcome, such as explaining an operator procedure across its guide, reference, and example.

## Assignment

Use the document coverage rows in the sprint README to cover root entry points, product and
architecture documents, development guides, operator guides, references, decisions, and example
explanations. Check remaining comments outside Rust sources, including workflow configuration
comments. Do not change workflow behavior, example data, schema files, or historical evidence.

For each assigned document, identify its audience and the question it owns. Follow its claims
into source, current consumers, tests, and the relevant canonical document before editing. Rewrite
compressed lists of properties into connected explanations: introduce the operation, describe
the responsible component and result, then explain the constraint and its consequence.

Keep necessary domain names consistent and explain them in context. Add a small example or
sequence when it helps a reader understand causality, authority, replay, branch visibility, or
unknown outcomes. Cut repeated facts and decorative terminology; do not shorten away meaning.
Retain clear existing sections instead of imposing one writing template on every document.

Respect each document's job:

- Vision describes intended experience; it must not imply every proposed capability exists today.
- Architecture explains responsibilities, dependencies, and invariants using readable examples.
- Status retains current version cells, qualified evidence, limitations, and required headings.
- Roadmap retains ordered unfinished product work without a documentation-sprint diary.
- Guides explain prerequisites, actions, expected results, and meaningful failures.
- References preserve exact identifiers, versions, field names, and supported/refused behavior.
- ADRs explain the decision and tradeoff at the time. Clarify wording without inventing a new
  decision or rewriting history to describe the latest implementation as the original choice.

Keep durable explanations in their existing owners. Update inbound links when headings change;
coordinate supporting edits under the scope policy. Do not grow a glossary in every
document or replace all technical terms with vague synonyms.

## Verify and stop

Run repository documentation contracts, existing CLI/example checks where applicable, and any
affected doctests using [workflow](../../workflow.md#choose-verification-for-the-change).
Inspect Markdown rendering for structure, tables, links, and code fences.
Do not weaken a contract assertion to allow an editorial change.

Hand off reviewed documents/sections, representative improvements, the source of any corrected
claim, verification results, unresolved disagreements, and the next portion to assign. Stop
after this portion. Do not treat a terminology search with no matches as proof of readability
or begin final review without an explicit assignment.
