# External model configuration provenance

The external evidence report binds Milkdrift's endpoint profile and task, but those facts do not
identify all mutable server-side generation settings. A server can change thinking mode under the
same model alias and profile. This limits reproducibility and claims about why a probe succeeded.

## Evidence and assessment

At `005ccb2dd8d31caae8bd14779a7b23d1b25a0f89`, the retained Bonsai run uses the supported
OpenAI-compatible mapping. The executor's private evidence under
`target/external-interop-20260910/` retains a 64-unit preflight ending at the output limit without
final text, a successful 4,096-unit preflight, and the qualifying `real-bonsai-03/report.json`.
The source profile advertises streaming and system-role support; the
[harness task](../../../../../tools/evidence/src/bin/milkdrift-external-evidence/workflows.rs)
does not specify reasoning controls. The [endpoint profile owner](../../../../../adapters/model-provider/src/profile.rs)
does not bind the effective LM Studio thinking toggle.

On 2026-09-11 the operator reported that Bonsai initially had thinking enabled and that they later
disabled it to let the test pass. This is an operator declaration, not a contemporaneous
per-request measurement. It supplies no exact transition timestamp or independently verified
setting for each instance. Budget and mode were not isolated variables. The larger allowance alone
cannot be credited with fixing the failure, and the successful run does not qualify thinking-on
behavior. Its observed process/model orchestration remains evidence for the configuration used.

Do not alter the retained report or reconstruct missing observations. Qualification wording must
retain this limitation. The [evidence guide](../../../verification-evidence.md#actual-binary-scenarios)
records it; the historical execution and independent review did not have the later disclosure.

## Questions of purpose

Assignment 01 now makes the missing provenance explicit: local-model reports separate the request
allowance, contract ceiling, harness default, and unknown profile token limit; combined evidence
reports label direct-endpoint and coding-agent endpoint settings independently as unknown.
Attempt inspection preserves supported requested reasoning choices without interpreting omission
as disabled thinking. [Result acceptance](../../../../guides/result-acceptance.md) separately
blocks incomplete output. These changes do not attest to mutable server settings or isolate a
thinking-mode effect; the broader questions below remain open.

- Which claim needs stronger configuration evidence: successful orchestration, reproducible model
  behavior, or thinking-mode compatibility? Would an explicit scope limit and operator declaration
  be enough for the first, without expanding the report for claims it was never meant to make?
- Which server settings materially affect the claim, and what is the smallest useful record?
  Capturing every runtime preference would add maintenance and disclosure costs without necessarily
  making nondeterministic model output reproducible.
- Can the endpoint actually attest to the settings used for a particular request? If it only
  reports mutable defaults before or after execution, would recording them create false confidence
  about drift? What uncertainty should remain explicit?
- Would requiring server-specific inspection prevent otherwise valid external capabilities from
  participating? Would controlled request parameters and separately labeled operator evidence be a
  better boundary than adding a native integration solely to inspect configuration?

Keeping the report narrow and improving its claim wording is a credible alternative. Expand the
evidence contract only when the additional facts answer a defined acceptance question and can be
obtained truthfully. A report should not promise complete model reproducibility merely because it
contains more configuration fields.

## Proposed correction boundary

The evidence tooling should retain effective reasoning mode, context window, relevant generation
defaults, and server/model/runtime identity where they can be verified. Where the endpoint cannot
report them, explicitly label operator declarations and unknowns rather than claiming verification.
Capture settings for both the direct-model instance and any model used inside the coding agent.
Decide which missing facts prevent a particular compatibility claim, rather than treating every
unknown as failure of all orchestration evidence.

Run controlled thinking-enabled and thinking-disabled cases with a recorded budget for each;
include exhaustion as a truthful non-qualifying result. Changing settings during a run must be
detected where possible or remain an explicit evidence limit. This requires report/provenance
design and real endpoint evidence beyond the completed sprint. The
[generation policy discussion](../discussions/model-generation-policy.md) owns the related
configuration design question.

## Contribution

2026-09-11 — Rowan-20260911-a (agent pseudonym), operator follow-up to the external interoperability
sprint: recorded the newly disclosed thinking toggle and separated observed output from causal
claims. No new provider requests or thinking-mode experiments were performed for this finding.
The operator requested purpose questions; the alternatives above challenge whether report
expansion is necessary, what it would establish, and where its cost would be justified.
