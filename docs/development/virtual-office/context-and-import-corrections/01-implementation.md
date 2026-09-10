# Repair context enforcement and import identity

Execute assignment 01 of [this sprint](README.md). Read [AGENTS.md](../../../../AGENTS.md) in its
required order, the [implementation practice](../../practices/implementation.md), and the
[documentation practice](../../practices/documentation.md) for explanations affected by the fix.
Follow the [workflow](../../workflow.md) for scope, completion, and verification. Recheck the
current tree before adopting the preparation findings.

Complete the two linked issues as one implementation assignment. Context enforcement is the main
body of work; the import-label correction belongs here because sequence compilation also declares
context policy and the label fix does not warrant another setup and handoff. Use internal iterations
to investigate, implement, and test the complete operation rather than stopping after each symptom.

## Required evidence and protected omissions

Start with the [context issue](../whiteboard/issues/context-policy-enforcement.md),
[ADR 0011](../../../decisions/0011-causal-context-manifests.md), the
[builder](../../../../crates/runtime/src/context.rs), its
[selector](../../../../crates/runtime/src/context/selection.rs), and
[candidate construction](../../../../crates/runtime/src/context/source/candidate.rs).

With `fail_closed = true`, an optional overflow under `StopAtFirstOverflow` followed by later
eligible required evidence must fail before dispatch. Required checks must survive the stopped
selection path, including missing, unsupported, and authority-denied evidence. Preserve the
existing distinction between eligible required evidence and an excluded exact source: the latter's
refusal depends on the complete required/exact/fail-closed condition. Keep optional loss behavior,
`fail_closed = false`, deterministic ordering, and all discovery and selection bounds truthful.

Make omission redaction depend on the candidate's disclosure restrictions even when the reported
reason is `SelectionStopped`, `ExcludedCategory`, or another selection reason. Exercise hidden
scopes and denied authority with both stopping and exclusion. Check source identities and byte
sizes in the serialized manifest, not only the omission enum. Retain useful reasons without
loading omitted content or widening authority to obtain it.

Trace these corrections through [dispatch](../../../../crates/runtime/src/engine/dispatch.rs),
[manifest reads and materialization](../../../../crates/runtime/src/context/source/materialize.rs),
and affected host, adapter, and inspection consumers. Add production-path evidence alongside
builder tests so synthetic candidate flags cannot conceal an acquisition or serialization bypass.
Inspect retry reuse and restart explicitly: dispatch can rebind a prior manifest without running
selection again. Do not rescan newer history or rewrite saved manifest bytes to make a correction
appear retroactive. If retained evidence cannot safely support future execution, establish an
explicit refusal consistent with the recovery contract and test that boundary.

## Session declarations at the invocation boundary

Trace [TaskContextPolicy](../../../../crates/blueprint/src/context.rs),
[SessionSelection](../../../../crates/model/src/task.rs), dispatch, and
[model-provider negotiation](../../../../adapters/model-provider/src/adapter.rs). Choose the
smallest existing boundary that can enforce agreement between governing policy and the actual
model request before external entry. Account for supported inline and artifact-backed request
paths, including retries, so the check cannot be bypassed by changing how a request is supplied.

Prove that matching `Fresh` remains usable, mismatches in either direction are refused, and matching
continuation declarations still face the endpoint's existing support checks. Do not silently
replace a requested continuation with `Fresh`, synthesize missing continuation references, or add
a provider session protocol. Observe refusal before an external request rather than only checking
an error string.

The [sequence compiler](../../../../crates/prompt-sequence/src/compiler.rs) currently creates
`process.execute` stages and carries session intent in their policy and stage data. Trace that
contract separately. Do not impose a model-request shape on process or other non-model capabilities,
and do not claim that an enum declaration implements process-session continuation. Preserve existing
supported process workflows and document any remaining capability-specific limits precisely.

Review any necessary durable-contract changes against current readers and fixtures. Use an ADR
when a durable decision or interpretation changes; do not add schema fields or versions solely
to avoid locating the proper enforcement owner.

## Correct the import reason

Resolve the [import-label issue](../whiteboard/issues/prompt-sequence-version-label.md) in the
compiler and [production reader](../../../../crates/prompt-sequence/src/document.rs). Derive the
reason's version from the validated import instead of maintaining another unrelated literal.
Check it directly in the [sequence suite](../../../../crates/prompt-sequence/tests/sequence.rs).

The [revision owner](../../../../crates/blueprint/src/revision.rs) includes the reason in identity.
Review exact-byte and revision/mutation expectations affected by the correction, distinguishing
the label-only identity change from any other compiler change. Preserve deterministic compilation,
the import's canonical meaning, schema-v1 refusal, and exact decoding of existing stored revisions.
Do not rename historical reasons or regenerate golden expectations without reviewing why they change.

## Verify and hand off

Add regressions that expose the reported defects before accepting the fix, then verify the
corrected production paths. Existing starting suites are:

```sh
cargo test -p milkdrift-runtime --test causal_context --all-features
cargo test -p milkdrift-runtime --test structured_runtime --all-features causal_context_production::
cargo test -p milkdrift-model-provider --test mock_endpoints --all-features
cargo test -p milkdrift-prompt-sequence --test sequence --all-features
```

Run other affected owner suites and the required
[full local gate](../../workflow.md#full-local-gate). Keep new fault-injection tests bounded and
observe the behavior independently of the implementation. Follow failures through to their cause;
do not weaken bounds, assertions, or authority checks to obtain a pass.

Capture useful local reasoning while changing the code and correct affected contract explanations.
Assignment 02 will review the coherent explanation after the design settles. Hand off both fixes,
the design and compatibility choices that matter, evidence tied to the checked tree, and remaining
review questions in the sprint README. Keep the whiteboard topics until final review accepts their
disposition. Stop at this checkpoint when assigned only phase 01; when assigned the whole sprint,
continue directly to phase 02.
