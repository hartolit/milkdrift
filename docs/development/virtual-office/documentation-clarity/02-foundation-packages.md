# Phase 02: Explain the foundation packages

Work toward the reader outcome assigned for this phase in the [sprint README](README.md).
Read [AGENTS.md](../../../../AGENTS.md) in its required order, the
[documentation standard](../../engineering-rules.md#7-documentation), and the
[virtual-office procedure](../README.md). Use the reviewed phase 01 example and the assigned
reader outcome. Choose routine assignment details from the user's request and sprint coverage.

## Assignment

Work on one assigned portion of `contracts`, `capability`, `blueprint`, `workspace`, `authority`,
or `model` under `crates/`. Read its manifest, public exports, relevant private implementation,
real consumers, and tests. Use phase 01 as a quality example, not a paragraph template. Do not
redo accepted explanations unless review reveals a concrete problem.

Add or complete the package README and crate introduction. Document public types, constructors,
traits, variants, fields, and functions needed for that reader outcome. Review ordinary comments and
test comments in scope too; explain non-obvious choices and remove redundant narration. Keep
short comments where the owning type already supplies sufficient context.

Explain the relevant reader questions for the assigned package:

- Contracts: which repeated validation/encoding operation it supplies and what the caller still checks.
- Capability: how a requirement differs from an advertised capability and the selection used by an attempt.
- Blueprint: how a caller constructs a valid definition or revision, and where execution begins later.
- Workspace: how values and artifact references relate to branches, bytes, producers, and retention.
- Authority: who asks for an operation, which grant is evaluated, what a decision means, and why a label cannot grant access.
- Model: how task requests, responses, and context manifests are used by runtime and adapters.

Follow actual consumers to explain relationships. Describe units, defaults, ordering, special
values, and refusal behavior where they matter. Include a supported use and observable result
without recreating runtime or provider setup in a contract package's README.

## Verify and stop

Run the checks selected by [workflow](../../workflow.md#choose-verification-for-the-change),
including relevant owner tests needed to substantiate explanations. Inspect rendered introductions
and examples; check that prose changes preserve names,
validation, fixture bytes, visibility, and contract versions.

Hand off exact files/symbols reviewed, areas retained as already clear, examples and evidence,
unresolved findings, and the remaining portion of this package. The coordinator updates coverage;
do not claim a whole package complete from a sample. Stop after the assigned portion. Separate
assignments cover remaining files and packages; do not begin phase 03 automatically.
