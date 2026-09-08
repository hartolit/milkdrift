# Phase 01: Explain the context policy and model limits

Work toward the reader outcome assigned for this phase in the [sprint README](README.md).
Read [AGENTS.md](../../../../AGENTS.md) in its required order, the
[documentation standard](../../engineering-rules.md#7-documentation), and the
[virtual-office procedure](../README.md). An explicit request to execute this phase assigns you
the scope below; record the assignment before editing. Merely reading or reviewing this prompt
does not dispatch the work.

## Assignment

Make the user's three examples understandable before using the approach elsewhere. Start with
`TaskContextPolicy` and its constructors/accessors in
`crates/blueprint/src/context.rs`, its attachment through `TaskConfig` in
`crates/blueprint/src/model/node.rs`, `CONTEXT_MANIFEST_SCHEMA_VERSION_V2` in
`crates/model/src/context.rs`, and `MAX_MODEL_OUTPUT_UNITS` plus the corresponding request
constructor/accessor in `crates/model/src/task.rs`. Also improve each package's `src/lib.rs`
introduction and add its `README.md`. Include related documentation corrections needed to explain
this path, using the scope policy; a general rewrite of either package is a later assignment.

## Read and explain

Trace policy defaults, validation, exact-source selection, and `TaskConfig::new`/`direct_inputs`.
Read runtime context discovery and selection, manifest construction and decoding, host
materialization, and model-provider request mappings as consumers. Inspect blueprint `kernel`,
model `contracts`, and runtime `causal_context` tests and the manifest fixture; read related ADRs
0011 and 0012. Preserve executable behavior and fixture bytes under the sprint's exclusions.

Explain what the policy asks for, when the runtime uses it, and how the manifest records the
actual result. Cover direct-input defaults, additional selectors, exclusions, budgets, session
choices, and required-source failure with the detail supported by code. Explain why requesting
a source does not override access or branch visibility. Show a compilable example that attaches
a policy to a task and makes at least one consequential choice explicit.

For the manifest version, describe what is stored, why byte checks and producer identities matter,
and how the reader handles unsupported versions. For output units, explain their meaning, valid
range and rejection, the difference from byte limits, and where endpoint-specific support is
checked. Correct the misleading output accessor wording after tracing its uses. Do not invent
the rationale for the numeric limit or claim that every provider supports it.

## Verify and stop

Run the blueprint kernel, model contracts, runtime causal-context suite, affected package
doctests, warning-denying rustdoc, and repository documentation contracts as described in
[workflow](../../workflow.md#choose-verification-for-the-change).
Inspect rendered API pages and README links. Preserve source behavior and canonical fixture bytes.

Hand off the named changes, a representative before/after explanation, supporting source/tests,
check results, and any unresolved claim. The reviewer should be able to explain how to create a
policy, where it is applied, and what a limit failure means without reading method bodies. Stop
after this handoff; do not start the foundation-package sweep.
