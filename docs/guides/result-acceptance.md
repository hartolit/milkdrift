# Accept workflow results before continuing

A completed invocation means its capability reported a result. It does not establish that the
result satisfies a review, that repository checks passed, or that a project is finished. Keep
these facts separate by composing ordinary tasks and a branch:

```text
produce output -> evaluate required output -> branch
                                               | pass -> dependent work -> success terminal
                                               | fail -> hold, failure, or remediation
```

`workflow.accept_result` is installed on `milkdrift-workflow-control`. It reads immutable evidence,
performs fixed purpose-specific checks, and publishes `acceptance_result`. When the requirement
passes, it also publishes `accepted_result`, referring to that same decision artifact. Its own
successful invocation means evaluation finished; inspect `accepted` and `reason` in the decision
or route an ordinary branch on `accepted_result`. Required publication failure publishes no
successful acceptance. The source invocation and its artifacts retain their original history.

## Declare the required result

Use `result_acceptance_task` and `result_acceptance_gate` from `milkdrift-control` when authoring a
blueprint in Rust. The task takes an inline `milkdrift.acceptance` contract, an optional `result`
artifact input, and the named evidence inputs. Connect the upstream result and all named evidence
with ordinary data edges. Connect the task's `out` to the gate's `in`, and its `accepted_result`
output to the gate's same-named input. Only the gate's `pass` arm may lead to dependent work.
The `fail` arm must have its own failure or hold route; ordinary control edges cannot merge.

For a complete final model review, the contract is:

```json
{"schema_version":1,"requirement":{"type":"model_prose"},"evidence_inputs":[]}
```

| Requirement | What acceptance establishes |
| --- | --- |
| `model_prose` | A canonical model response reports `stop` and non-whitespace final text. |
| `model_decision` | A `stop` response contains a structured decision citing all configured evidence. |
| `model_tools` with `names` | A `tool_calls` response contains calls only to declared names. Calls remain data. |
| `review_prose` | A configured reviewer's artifact contains non-whitespace UTF-8 text. |
| `review_decision` | A configured reviewer's artifact contains the structured decision below. |
| `verification` with `checks` | Every named check passed on the same before/after repository checkpoint, with a verified change or justified no-change result. |

A structured decision has exactly `accepted` (Boolean) and `evidence` (an array of exact capability
artifact references). Configure at least one evidence input for decision requirements. Every
configured reference must be cited and every cited reference must be configured; malformed data,
missing evidence, and a negative decision reject acceptance. A short valid decision is sufficient.
Prose checks establish mechanical completeness. They do not judge correctness, confidence, or
whether an independent reviewer would agree. Choose a configured reviewer or verifier whose
authority and capability match the decision the workflow needs.

Empty text can be a valid model result. A task requesting tools can accept tool-only output;
a prose review cannot. Structured-only output can satisfy `model_decision`. A `length` finish,
including a nonempty draft, cannot pass a completeness gate. Raw drafts, finish reasons, usage,
and the selected causal context remain available through their separately authorized artifacts.

## Verify repository work

The configured verifier, running separately from the coding agent, owns observing the exact
repository state and executing the declared checks. It publishes `VerifiedCheckpoint`:

```json
{
  "checkpoint":"sha256:exact-repository-snapshot",
  "checked_checkpoint":"sha256:exact-repository-snapshot",
  "checks":{"python.unittest":true,"git.diff":true},
  "coding":{"type":"no_change","justification":"Existing behavior passes the requested independent checks."}
}
```

Use `{"type":"changed"}` when the verifier established the requested change. The checkpoint
identifier must bind the files, revisions, and uncommitted state relevant to the configured checks;
an unchanged commit name alone cannot identify an edited working tree. Compare observations before
and after verification. A mismatch, failed or missing check, or empty no-change justification
rejects acceptance. Acceptance does not run Git or execute arbitrary check names itself. It trusts
the selected verifier to establish its report; a model's prose is not a verifier report.

[Prompt-sequence schema 3](../reference/prompt-sequence-v3.md) installs verification acceptance and
review acceptance in both initial work and prospective remediation. The approval remains an
authorized proposal decision, application, and exact signal through the existing control path.
No acceptance requirement converts arbitrary approval prose into control authority.

## Inspect and recover

`milkdrift --json attempt inspect RUN_ID ATTEMPT_ID` separates invocation `terminal` from
`result_acceptance.accepted` and its closed reason vocabulary. Model attempts also expose
`model_generation`: literal request allowances and reasoning choices when available, and a finish
reason only when the response artifact is authorized and readable. Artifact-backed or dynamically
bound request choices may be absent from this compact diagnostic; inspect the exact authorized
request evidence to resolve them. Run `terminal` remains the workflow's aggregate outcome.

An absent acceptance field does not mean acceptance passed. It can mean the task is not an
acceptance task, evaluation has not published, or the reader cannot inspect the decision artifact.
Workspace and artifact reads are authorized independently; failure reasons do not repeat hidden
identities, payloads, or provider diagnostics.

Restart replays the ordinary journal and its immutable output references. Acceptance executes no
external validation side effects. Replaying an accepted release command returns its original
receipt. Rejected source output is never replaced: use a separately authorized remediation task or
prospective revision. Increasing a model allowance requires a new declared attempt or task.
