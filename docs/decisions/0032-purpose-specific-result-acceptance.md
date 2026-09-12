# 0032 — Accept stage results through ordinary workflow composition

Provider completion can preserve an empty, tool-only, structured-only, or exhausted response.
Treating every such completion as a usable review allowed dependent work to enter without the
output it required. Changing the model response contract to reject empty text would discard
legitimate tool responses and conflate generation with workflow purpose.

The control owner now evaluates a bounded `ResultAcceptanceContract` through the existing
`workflow.accept_result` capability. An ordinary task publishes a typed decision and, only on
acceptance, a marker consumed by an ordinary branch. Runtime continues to own attempts, outputs,
replay, authorization, and project terminals. No workflow primitive, executor, or generic quality
framework is introduced. The [operator guide](../guides/result-acceptance.md) owns use and limits.

Model response and blueprint schemas are unchanged. Acceptance contract/results are version 1.
The built-in control capability descriptor advances to revision 2 for its added operation.
Prompt-sequence import schema 3 replaces the optional success artifact with a required typed
verifier report and installs acceptance on review continuations. Import schemas 1 and 2 are
refused; stored blueprint revisions keep their bytes, identities, and previous behavior. Historical
schema-2 stage associations remain inspectable, but a schema-3 remediation document cannot be
substituted for an older import. Operators author an explicit prospective change for such a run.

Control protocol 2.4 adds separately authorized acceptance and generation diagnostics to attempt
inspection. The CLI keeps envelope schema 2. Negotiation does not downgrade current DTOs for older
strict clients. No storage, event, or projection migration is required.

Mechanical text checks establish nonempty, complete output, not semantic quality. Structured
decisions bind exact configured evidence; repository checks remain the configured verifier's
responsibility. Approvals remain authenticated control decisions, never accepted prose. A failed
publication or unreadable evidence cannot publish an accepted marker. New decisions apply to new
work and never reclassify a completed provider invocation.

The decision and accepted marker reference the same artifact. The host's publication bridge
therefore keeps distinct invocation input bindings but records each exact artifact causal parent
once. This permits downstream publication without duplicating a durable decision or weakening
the workspace's unique-provenance invariant.
