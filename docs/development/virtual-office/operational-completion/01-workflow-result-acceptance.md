# Assignment 01 — Make task acceptance distinct from invocation completion

## Outcome and scope

Implement purpose-specific workflow result acceptance using the existing task, verification,
branch, proposal, and artifact mechanisms. A completed model invocation must not automatically
satisfy a review or allow dependent work to continue. Preserve valid empty-text tool-call responses.

This is an implementation assignment for the next coding agent. Coordinate acceptance through the
sprint README. Use the current checkout; the reviewed reference is `aa77c483626b100e0f550c63648875ba28182238`,
not a required reset target. No UI, new provider family, general quality-engine framework, or
new workflow primitive is authorized.

Read `AGENTS.md`, the canonical product/architecture/status/roadmap documents, and
`docs/development/practices/implementation.md`, `docs/development/practices/documentation.md`,
`docs/development/workflow.md`, and this sprint's README. Recheck implementation before editing;
retain behavior already correct and prove it instead of recreating it.

## Evidence and source trail

The review records a real fork/join run whose reviewer exhausted its output allowance without final
text, while the workflow mechanically completed. `ModelTaskResponse` intentionally permits empty
text and retains finish reason, tool calls, structured output, and usage. This is not evidence that
every empty response is invalid or that a model's self-reported success is trustworthy.

Start with:
- `crates/model/src/task.rs` and model contract tests;
- `adapters/model-provider/src/adapter.rs`, `openai_compatible.rs`, `anthropic.rs`, and `tests/mock_endpoints/`;
- `crates/prompt-sequence/src/compiler.rs`, its document/validation/tests, and existing verification profiles;
- `crates/blueprint/src/model/contract.rs`, branch/reducer/terminal behavior;
- `crates/control/`, current inspector/control DTOs and CLI rendering;
- the existing local-model and external-evidence workflows in `tools/evidence/`;
- `docs/development/virtual-office/whiteboard/discussions/model-generation-policy.md` and
  `issues/external-model-configuration-provenance.md` in the same whiteboard.

Inspect the existing stage/checkpoint/failure routing before deciding whether a schema extension is
necessary. Prefer composition and a narrow typed acceptance result over a new orchestration layer.

## Implement

1. **Separate three outcomes.** Keep provider invocation completion immutable. Evaluate required
   stage outputs separately. Evaluate project completion from accepted stage results, not provider
   transport status. Make each distinction visible in API/CLI inspection and stable JSON output.
   Do not retroactively turn a successfully completed external invocation into a transport failure.

2. **Express the actual requirement.** For a review, require a usable review artifact or a validated
   structured decision with required evidence references. For coding, validate the specified change
   or explicit justified no-change result against the exact repository state. For verification,
   require the configured checks on the exact checkpoint. For approval, require the authorized
   decision, not arbitrary prose. Do not use word count, one-line rejection, or a model's confidence
   as a universal quality oracle. Distinguish mechanical output validity from semantic review.

3. **Close gates on failure or uncertainty.** Missing required output, budget-exhausted final answer,
   invalid structure, stale checkpoint, and unverified decision must not unblock dependants. Route
   through existing fail/hold/review/remediation policies. Preserve raw response artifacts, finish
   reason, usage, selected context, checkpoint identity, and the reason acceptance failed. A retry
   is a new authorized attempt, not a silent budget increase or replacement of failed history.

4. **Preserve legitimate output shapes.** A tool-only response may satisfy a task explicitly asking
   for tool calls. It must not satisfy a prose-review contract merely because tools are present.
   Structured-only output is valid when that is the requested result. Whitespace-only text is not
   usable prose. A truncated draft remains inspectable but cannot silently pass a completeness gate.
   Required publication failures remain distinct from provider generation outcomes.

5. **Use the production workflow path.** Install these requirements in maintained review/verification
   examples and the real headless/evidence scenarios. A stricter assertion only in an external
   test script is insufficient: the ordinary workflow must prevent the next stage entering.
   Do not make all generic model tasks inherit a review-specific policy.

6. **Explain model limits without inventing support.** Preserve requested output allowance, supported
   reasoning controls, finish reason, and returned usage in diagnostics. Differentiate request,
   profile/deployment limit, hard contract ceiling, and harness default. For real-run reports label
   server settings as verified, operator-declared, or unknown, separately for a direct endpoint and
   any endpoint used inside the coding agent. Omission is not “thinking disabled.” Do not claim that
   the observed experiment proved thinking mode caused the failure. No generic thinking toggle,
   native LM Studio API, adaptive pricing/budget service, or new provider mapping is authorized.

## Acceptance tests

Use local controlled endpoints and actual daemon/client binaries for the central scenario:

```text
model completes with no final review -> stage acceptance fails
    -> dependent task entry counter remains zero
    -> authorized remediation produces valid review
    -> only then may continuation proceed
```

Include valid prose, structured-only review, valid tool-only task, tool-only response in a prose
stage, whitespace final text, malformed decision, output-limit exhaustion, stale checkpoint,
artifact publication failure, and explicit no-change coding work. A valid minimal structured answer
must not fail merely because it is short.

Restart between provider completion and acceptance, between rejection and remediation, and after
acceptance but before the next dispatch. Exact replay must preserve one decision and not execute
validation side effects twice. Verify unauthorized artifacts/decisions do not leak through reasons.
Use stable IDs, clocks, synchronization, and entry counters rather than sleeps or copied event counts.

## Verification, docs, and stop

Run focused model contracts, mock endpoints, prompt-sequence/compiler, control/runtime, and daemon
suites, then the full gate from `docs/development/workflow.md`. Reuse and extend the existing actual-
binary operator/model evidence lanes; do not build a parallel harness. External paid requests are
not implied by this assignment. Record missing real prerequisites rather than fabricating evidence.

Update affected examples, schema fixtures/readers if changed, API/CLI explanations, model adapter
README, and `docs/product/status.md`. Keep the detailed vision intact. Record only newly established
facts on the existing whiteboard; leave broader generation-policy questions unresolved if not tested.

Finish when ordinary workflows—not just evidence scripts—distinguish invocation, accepted output,
and goal completion, the negative cases block continuation, restart preserves truth, and checks pass.
Hand off the canonical acceptance owner and exact controller integration point to Assignment 03.
