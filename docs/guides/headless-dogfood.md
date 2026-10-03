# Headless prompt-sequence dogfood

Milkdrift can import an ordered Markdown implementation plan, compile it into an ordinary immutable
blueprint revision, execute each prompt in a fresh coding-agent process against one persistent
authorized repository, verify each result, and stop at a durable review/approval boundary when
verification fails. Remediation is a normal digest-bound prospective revision; it does not rewrite
completed work or enlarge the run's frozen authority.

The complete example is [`examples/headless-dogfood-sequence.md`](../../examples/headless-dogfood-sequence.md).
Its capability identities are placeholders for operator-configured trusted-host process profiles.

## What the import creates

For each stage `S`, schema 3 creates only existing blueprint primitives:

```text
stage-S-coding -> stage-S-verification -> stage-S-acceptance -> stage-S-gate
                                             | pass -> next stage / sequence-succeeded
                                             | fail -> stage-S-review -> stage-S-approval
```

Coding, verification, and acceptance are distinct `Task` nodes. Acceptance checks the configured
verifier's exact checkpoint report; its accepted marker opens the `Branch`. Review is another
fresh-context task whose output also passes acceptance before reaching the approval hold. An
unusable review reaches a separate rejection hold. Approval is an ordinary durable `SignalWait`
for `sequence.approved`. The final outcome is an explicit `Terminal`. Configure the report shape
and understand its limits using [result acceptance](result-acceptance.md).
There is no dogfood node kind, scheduler, hidden retry loop, Git implementation, or UI state.

The maintained example requests `fresh` context for each coding task. Sequence import refuses
continuation declarations because the process adapter has no persistent session protocol. Select
durable prior evidence explicitly when another fresh invocation needs it. A fresh invocation need
not use a new repository:
a `shared_sequential` repository profile and a local-process
`authorized_host_path` working directory let accepted files persist across separate invocations.
Parallel designs must select `isolated_worktrees` and use operator-configured version-control and
merge capabilities; the runtime does not manufacture branches or commits.

## Configure the execution boundary

Register separate exact trusted-host process generations for coding, verification, and review. A
sequence names only their capability, exact `process.execute` operation, null provider-profile
field, execution trust class, and maximum side effect. It cannot provide argv, executable paths,
network destinations, environment variables, or secret values.

For a sequential repository process profile, declare the repository itself as a read-write root
and select it explicitly:

```json
{
  "working_directory": {
    "type": "authorized_host_path",
    "path": "/absolute/operator-owned/worktree"
  },
  "filesystem_roots": [
    {"path": "/absolute/operator-owned/worktree", "access": "read_write"},
    {"path": "/opt/coding-agent/bin", "access": "execute"}
  ]
}
```

Registration canonicalizes the exact directory. Every invocation rechecks that it is still the
same ordinary directory under a configured read-write root. Milkdrift-owned context/input
materialization and output publication remain in an isolated execution root. The process runs in
the persistent repository. This is an authority boundary and provenance fact, not a sandbox; see
[local process operator guide](local-process.md).

The repository section of the import is a bounded policy/reference document. `root_ref`, starting
revision, credentials, and remote profile references are opaque identifiers interpreted by the
configured capabilities, never executable prompt data. Schema 3 requires explicit read, write,
and execute operations plus starting-state, diff, and verification evidence policies.

## CLI connection and output contract

Use the [operator setup](../../examples/operator/README.md#startup-and-restart) for endpoint and
private credential configuration. The [CLI automation contract](../reference/control-api.md#cli-automation-contract)
owns success/failure JSON, stdout, exits, deadlines, bounded files/stdin, and reconnect behavior.
Use explicit command IDs and preserve each complete submitted request after a lost response.
The [remediation flow](#failure-and-remediation) constructs a proposal from fresh reads and needs
the additional recovery guidance below.

## Import and run

Set the normal daemon endpoint/token options, then validate before storing:

```sh
milkdrift --command-id sequence-plan-validate-v1 \
  sequence validate examples/headless-dogfood-sequence.md
milkdrift --command-id sequence-plan-import-v1 \
  sequence import examples/headless-dogfood-sequence.md
milkdrift sequence show REVISION_ID
milkdrift --command-id run-plan-start-v1 --expected-revision REVISION_ID \
  run start RUN_ID WORKFLOW_ID REVISION_ID --request-file plan-run.request.json
milkdrift --json --timeout-secs 60 run timeline RUN_ID --limit 100 --follow
```

The import result reports schema version, sequence/workflow/revision identity, semantic and import
digests, repository-profile digest, and the exact stage-to-node mapping. Repeating the same import
under the same command identity is idempotent; importing the same canonical sequence through a new
command returns `replayed` according to immutable-revision storage.

Inspect the bounded frontier and durable history through the API, never redb:

```sh
milkdrift sequence status RUN_ID REVISION_ID
milkdrift sequence stage RUN_ID STAGE_ID
milkdrift run timeline RUN_ID --limit 100
milkdrift attempt inspect RUN_ID ATTEMPT_ID
milkdrift artifact metadata ARTIFACT_ID
milkdrift artifact get ARTIFACT_ID --output NEW_OUTPUT_FILE
milkdrift daemon authority
```

For a compact operator view, use `milkdrift run result RUN_ID`. It shows the saved workflow
name/version, current steps, invocation outcomes, required result decisions and declared final
outputs. `--details` adds model/profile, requested limits, selected inputs, reported usage and
failure evidence. Missing usage stays unknown. A model returning text does not establish that
its required check passed or that the workflow finished.

`milkdrift run result RUN_ID --field OUTPUT_NAME --output NEW_OUTPUT_FILE` downloads a declared
successful terminal output and verifies its size and digest. Previews are bounded and escape
terminal controls; a truncated preview is labelled. Rejected or uncertain model output is evidence,
not final output. Read scope can hide details, and invoke-only published callers use their public
invocation result rather than private run inspection. Offered control operations reflect current
permissions; submitting one still rechecks its guards and applicable rules.
In JSON mode a requested download produces one final `run.result` record after verification;
its `download` field identifies the committed file. A failed download emits only the failure.

Following starts with a current authorized run view and then new events. The initial timeline
command still returns only its requested page; following does not fetch the rest of old history.
The shared client suppresses duplicate positions when reconnecting. If a feed reports an expired
cursor, the CLI obtains a fresh authorized view and subscribes again without that cursor, within
`--max-reconnects` and the overall timeout. Authorization failure or a closing notice ends
observation. Closing the view never cancels the run.

The timeline maps compacted historical nodes to exact attempt identities. Attempt inspection pages
the authoritative journal when an attempt has left the compact frontier and exposes its frozen
capability snapshot, process/model generation, authority-linked context-manifest reference,
outputs, terminal evidence, and uncertainty state. Restricted context detail and artifact content
remain separately authorized.

For a generic canonical blueprint workflow, validate before import and request exact stored bytes
explicitly. `--document` writes only canonical bytes to stdout. `--output` creates a new file and
refuses to overwrite an existing destination:

```sh
milkdrift --command-id blueprint-validate-v1 blueprint validate blueprint.json
milkdrift --command-id blueprint-import-v1 blueprint import blueprint.json
milkdrift blueprint show REVISION_ID --document > canonical-blueprint.json
milkdrift --json blueprint show REVISION_ID --output NEW_CANONICAL_BLUEPRINT_FILE
```

List commands return only one bounded page. Pass the returned opaque cursor explicitly to fetch the
next page; the CLI never silently drains revision, run, proposal, or timeline history.

## Retained or uncertain work

Resolve an exact attempt through the same typed daemon command used by every controller. The CLI
does not decide whether the requested action is legal:

```sh
milkdrift --json --command-id retain-attempt-v1 \
  --expected-sequence RUN_SEQUENCE --expected-revision REVISION_ID \
  --evidence recovery_observation=evidence-retain-v1 \
  attempt resolve RUN_ID ATTEMPT_ID decision-retain-v1 --action retain

milkdrift --json --yes --command-id retry-attempt-v1 \
  --expected-sequence RUN_SEQUENCE --expected-revision REVISION_ID \
  --evidence external_receipt=receipt-query-v1 \
  attempt resolve RUN_ID ATTEMPT_ID decision-retry-v1 --action retry
```

The actions are `query`, `retry`, `compensate`, `retain`, `resolve-succeeded`, and
`resolve-failed`. Compensation alone requires `--remediation-node`. Retry, compensation, and both
evidence-based terminal resolutions require confirmation; JSON mode therefore requires `--yes`.
Query and retain do not claim a terminal outcome.

## Failure and remediation

If a verifier completes but its checkpoint report fails acceptance, the `pause_for_review` failure
arm runs the independent reviewer and waits durably for approval. A verifier invocation that
itself fails does not become this rejection branch; inspect its attempt first. Imports
using `fail_run` instead go to a failure terminal. For the review path, first pause the aggregate using
the shared run command, then create a bounded proposal from the exact original sequence and current
revision:

```sh
milkdrift --command-id run-remediation-pause-v1 --expected-sequence RUN_SEQUENCE \
  run pause RUN_ID
milkdrift --command-id sequence-remediation-submit-v1 \
  --expected-sequence PAUSED_RUN_SEQUENCE --expected-revision REVISION_ID \
  sequence remediate examples/headless-dogfood-sequence.md RUN_ID REVISION_ID STAGE_ID \
  --generation 1 \
  --proposal proposal-remediation-1 \
  --prompt remediation.md
milkdrift proposal show RUN_ID proposal-remediation-1 PROPOSED_REVISION
milkdrift --yes --command-id proposal-remediation-approve-v1 \
  --expected-sequence DECISION_RUN_SEQUENCE --expected-revision PROPOSED_REVISION \
  proposal approve RUN_ID proposal-remediation-1 PROPOSAL_DIGEST \
  PROPOSED_REVISION decision-remediation-1
milkdrift --yes --command-id proposal-remediation-apply-v1 \
  --expected-sequence APPLY_RUN_SEQUENCE --expected-revision PROPOSED_REVISION \
  proposal apply RUN_ID proposal-remediation-1 PROPOSAL_DIGEST PROPOSED_REVISION
milkdrift --command-id run-remediation-signal-v1 --expected-sequence SIGNAL_RUN_SEQUENCE \
  run signal RUN_ID --signal-id signal-remediation-1 \
  --signal-type sequence.approved --payload '{}'
milkdrift --command-id run-remediation-resume-v1 --expected-sequence RESUME_RUN_SEQUENCE \
  run resume RUN_ID
```

The proposal is guarded by the exact base revision/digest and observed run sequence. Its ordinary
mutation removes only the unused failure continuation and prospectively inserts fresh remediation,
verification, success re-review, failure re-review, and renewed approval nodes. The runtime's
existing reconciliation plan preserves completed executions and facts. Apply changes the pinned
revision while the run remains paused; an explicit signal and resume are still required.

Use the returned sequence and proposal facts at each step; earlier guards can become stale after
an accepted decision. Keep the original sequence document unchanged for remediation provenance.
Signalling the original approval wait without applying a repair leads to its failure terminal,
not to an automatically repaired next stage.

`sequence remediate` reads the current run and actor to build the proposal. After a lost reply,
rerunning the same arguments can therefore produce a different document under the old command ID.
Inspect the run, proposal list, and proposed revision to establish what was retained before
proceeding. Automation that needs exact submission replay should retain a complete proposal
document and submit it with `proposal submit`; a fresh command ID is new intent, not recovery.

The frozen run grant remains authoritative. A proposed verifier/profile or other requirement
outside that envelope is rejected at revision adoption. The sequence document limits remediation
generation only. Runtime retry and authority budgets, adapter timeouts/admission bounds, artifact
budgets, and retention policies are configured and enforced by their owning layers; the import
does not pretend to add additional ceilings and never grants authority by itself.

### Repair a held model result

The ordinary model-workflow editor has a narrower repair convenience. It supports a rejected
completeness check at the **final model step's review hold**, before that hold has been signalled.
For the release-notes example, this is `author.review.hold`. Pause the run first, using the
sequence from `run result`. Saving a changed workflow draft affects later starts; it does not
change an existing run. The following proposal instead changes that run's future continuation.

Write `repair.txt` with the correction to request, then prepare and submit it:

```sh
milkdrift --command-id model-pause-1 --expected-sequence RUN_SEQUENCE run pause RUN_ID
milkdrift --command-id model-repair-prepare-1 proposal repair RUN_ID review \
  --proposal repair-final-1 --new-step repair --model operator-model \
  --prompt repair.txt --maximum-output-units 512 --file repair.json
milkdrift --command-id model-repair-submit-1 proposal submit repair.json
milkdrift proposal show RUN_ID repair-final-1 PROPOSED_REVISION
milkdrift --command-id model-repair-approve-1 proposal approve RUN_ID repair-final-1 \
  PROPOSAL_DIGEST PROPOSED_REVISION decision-repair-1
milkdrift --command-id model-repair-apply-1 proposal apply RUN_ID repair-final-1 \
  PROPOSAL_DIGEST PROPOSED_REVISION
```

Preparation writes a new proposal file and leaves the run unchanged. Submission returns the
proposal digest and proposed revision. The document binds the exact base and sequence; stale
submission is refused. Approval and application display the recorded affected work, read the
current sequence and ask for confirmation. Noninteractive callers must explicitly pass `--yes`.
Each operation still needs its own permission, and proposal admission checks the frozen run grant
and protected agreements. This convenience refuses richer definitions it cannot preserve exactly.

The repair receives the selected failed model response and the final step's original run-input
bindings. It receives no implicit earlier conversation or unrelated artifacts. The failed check
and all completed records remain visible. New work must pass a new completeness check before its
output can become the successful terminal result. A second rejection reaches a new review hold;
the convenience does not recursively repair the resulting richer graph.

After application, use `run result` for the next sequence, signal the hold, then resume with the
signal reply's sequence:

```sh
milkdrift --command-id model-release-1 --expected-sequence SIGNAL_SEQUENCE \
  run signal RUN_ID --signal-id release-repair-1 --signal-type workflow.reviewed
milkdrift --command-id model-resume-1 --expected-sequence RESUME_SEQUENCE run resume RUN_ID
milkdrift run result RUN_ID --details --field notes --output repaired-notes.txt
```

Signalling before applying the repair follows the unchanged failure continuation. Once terminal,
the run cannot resume. Where the caller may create and start runs, `run result` offers a new start
with `--evidence recovery_observation=OLD_RUN_ID`; supply a new run/command/request-file identity
and the required inputs. That evidence links the deliberate new execution without rewriting the old
outcome. It does not automatically copy private history or authorize new work.

## Codex as one generic coding-agent profile

Codex may be configured exactly like any other trusted-host coding CLI. Pin the executable bytes,
package revision, operation, argv, fresh invocation behavior, repository working directory,
prompt/context-manifest inputs, diff/result/log outputs, cancellation and timeout behavior, and
opaque secret references in a local-process profile. Then use that profile's capability identity
in `stage.coding`.

Do not place a token, shell command, arbitrary flag string, or executable path in the prompt
sequence. Do not enable provider-managed continuation when the stage requests `fresh`. Verification
must be a separately configured capability with safe named checks and structured artifacts. Core
code and deterministic tests do not know that Codex exists.

## Deterministic proof

The [source-learning route](../README.md#learning-the-implementation) links the independent
sequence/remediation/restart proof. Run the [actual-binary scenario](../development/verification-evidence.md#actual-binary-scenarios)
for CLI/daemon composition. These tests use deterministic processes and are not real-model
interoperability qualification.
