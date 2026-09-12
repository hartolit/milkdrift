# Configurable generation and thinking policy

The operator should be able to choose generation settings for the selected model and task. A
single small evidence default is insufficient for reasoning workloads, but removing all bounds
would discard explicit resource authority. The useful design question is how to select and record
effective settings while preserving bounded execution and immutable history.

## Observed contract

Inspected at `005ccb2dd8d31caae8bd14779a7b23d1b25a0f89`:

- [ModelTaskRequest](../../../../../crates/model/src/task.rs) already requires a per-request output
  allowance, with a contract ceiling of 4,000,000 units. `ReasoningControl` has optional effort
  (`low`, `medium`, `high`) and a reasoning-unit cap, but no explicit enabled/disabled mode.
- The [external harness](../../../../../tools/evidence/src/bin/milkdrift-external-evidence/main.rs)
  accepts `--max-output-units` from 1 through 65,536 and defaults to 64. Its guide's 4,096 is an
  invocation example. The [workflow builder](../../../../../tools/evidence/src/bin/milkdrift-external-evidence/workflows.rs)
  freezes the selected allowance into the task and always omits reasoning controls. Omission
  leaves provider defaults in effect; it does not request thinking off.
- The [OpenAI-compatible mapping](../../../../../adapters/model-provider/src/openai_compatible.rs)
  sends `max_tokens` and maps advertised effort to `reasoning_effort`, but refuses a reasoning-unit
  cap. The [Anthropic mapping](../../../../../adapters/model-provider/src/anthropic.rs) refuses
  reasoning controls. A shared typed request therefore does not establish portable support.

The current split between request, profile, authority, and byte/time limits is useful. Neither the
64-unit default nor the harness ceiling represents a measured model capability. The
[implementation practice](../../../practices/implementation.md#22-constants-defaults-and-configuration-are-different)
already distinguishes defaults, deployment choices, per-operation policy, and hard ceilings.

## Questions of purpose

- Which supported Milkdrift workflow needs task-level thinking control, rather than a model chosen
  and configured by its operator? Would exposing the existing effort field or requiring a suitable
  evidence budget solve the demonstrated problem without adding another contract?
- Should a workflow own thinking on/off and reasoning budgets, or should the external capability
  choose how to produce a useful answer within its total allowance? What user decision requires
  each control, and where would that control be misleading across providers?
- Would automatic budget selection improve results enough to justify model-limit discovery,
  estimation errors, and configuration precedence? Could explicit per-task budgets and operator
  presets provide the same value with less machinery? A small qualification harness may also
  reasonably impose a tighter scope than production tasks; what should justify its ceiling?
- What experiment would show an improvement in useful final output, latency, or operator control,
  rather than just more settings? If provider defaults perform adequately, or a thinking toggle
  cannot be verified, should we retain the current design or narrow the supported configuration?

The strongest case for a smaller change is that explicit output allowances and mapped effort
already exist, while the observed run changed two variables. That evidence alone does not justify
adaptive budgeting or broad provider parity. The case for more control must establish a product
need and a reliable mapping; supporting a popular model feature is not sufficient by itself.

## Proposed direction

Assignment 01 implements the narrower output boundary described in
[result acceptance](../../../../guides/result-acceptance.md): required complete prose rejects
output-limit exhaustion, while explicit structured and tool requirements preserve valid empty-text
results. Requested allowances and supported reasoning choices remain inspectable; this adds no
thinking toggle, adaptive allowance, or new provider mapping. The generation-policy choices below
remain proposals.

Keep model inference external. Let a task choose thinking mode, supported effort, and generation
budgets under validated endpoint capabilities and operator authority. Resolve any model/task
defaults before execution and retain the effective choices with provenance. Changes apply to future
tasks or prospective revisions; no silent increase or replay after an entered attempt exhausts its
allowance. If input-aware budgeting is added, distinguish estimates from verified model limits.

Use explicit protocol mappings and reject unsupported combinations before entry. Do not make
generic JSON passthrough or one global thinking boolean stand in for different provider contracts.
For example, LM Studio's [native chat API](https://lmstudio.ai/docs/developer/rest/chat) documents
`reasoning` settings including on/off, while its
[chat-completions parameter list](https://lmstudio.ai/docs/developer/openai-compat/chat-completions)
does not establish that mapping. Milkdrift currently uses the latter; adding a native endpoint is
a separate scope decision, not required just to expose existing effort support.

Evaluate a configurable evidence default against requiring an explicit real-run budget. Review
whether each ceiling protects an actual implementation bound or should be a deployment policy.
Thinking cases should verify final-answer separation, truthful usage when available, streaming,
budget exhaustion, missing final output, and unsupported-control refusal. Internal reasoning is
not itself the final answer, and qualification need not expose a private reasoning trace. Pair
these cases with the [configuration provenance finding](../issues/external-model-configuration-provenance.md).

## Contribution

2026-09-11 — Rowan-20260911-a (agent pseudonym), operator follow-up to the external interoperability
sprint: traced current request, harness, and adapter owners after the operator questioned the
4,096-unit choice and requested configurable thinking support. The mappings and controlled cases
above are proposals; this contribution did not implement or qualify them.
The operator requested explicit purpose questions so later evaluation can reject or narrow the
proposal instead of treating its presence on the board as agreement.
