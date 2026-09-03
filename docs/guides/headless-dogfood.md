# Headless prompt-sequence dogfood

Milkdrift can import an ordered Markdown implementation plan, compile it into an ordinary immutable
blueprint revision, execute each prompt in a fresh coding-agent process against one persistent
authorized repository, verify each result, and stop at a durable review/approval boundary when
verification fails. Remediation is a normal digest-bound prospective revision; it does not rewrite
completed work or enlarge the run's frozen authority.

The complete example is [`examples/headless-dogfood-sequence.md`](../../examples/headless-dogfood-sequence.md).
Its capability identities are placeholders for operator-configured trusted-host process profiles.

## What the import creates

For each stage `S`, schema 2 creates only existing blueprint primitives:

```text
stage-S-coding -> stage-S-verification -> stage-S-gate
                                             | pass -> next stage / sequence-succeeded
                                             | fail -> stage-S-review -> stage-S-approval
```

Coding and verification are distinct `Task` nodes. The gate is a `Branch` over the presence of the
declared verification-success artifact. Review is another fresh-context `Task`; approval is an
ordinary durable `SignalWait` for `sequence.approved`. The final outcome is an explicit `Terminal`.
There is no dogfood node kind, scheduler, hidden retry loop, Git implementation, or UI state.

Every coding task requests `fresh` context. Fresh means a new process/provider session, not a new
repository: a `shared_sequential` repository profile and a local-process
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
configured capabilities, never executable prompt data. Schema 2 requires explicit read, write,
and execute operations plus starting-state, diff, and verification evidence policies.

## CLI connection and output contract

Point the client at one daemon and reference a restricted credential file. Bearer values are never
command arguments:

```sh
export MILKDRIFT_ENDPOINT=http://127.0.0.1:9734/
export MILKDRIFT_TOKEN_FILE=/absolute/operator-owned/milkdrift.token
milkdrift --json daemon readiness
milkdrift --json daemon authority
```

Every `--json` success is one compact schema-1 document. A followed run, capability, or health
feed emits one complete JSON document per line and reconnects from the last authenticated cursor;
a retryable reconnect is a typed `stream_status` line. Failures emit one bounded schema-1 `error`
document on stderr. Scripts must check the exit status and retain explicit page or stream cursors.

All command-envelope options are global: `--command-id`, `--expected-sequence`,
`--expected-revision`, bounded `--reason`, and repeatable `--evidence KIND=ID`. Accepted evidence
kinds are `authority_decision`, `worker_observation`, `external_receipt`, `artifact`, and
`recovery_observation`. Use a stable command identity for every noninteractive submission; replay
the exact complete argument and document set under that same identity. Reusing it with changed
content is a conflict.

Canonical blueprint, sequence, proposal, and layout inputs accept a regular file or `-` for one
bounded stdin document. One command may not consume stdin twice. Duplicate JSON keys, excess
size/depth/count, malformed versions, and noncanonical owner documents fail before submission or at
the authoritative daemon validation boundary.

## Import and run

Set the normal daemon endpoint/token options, then validate before storing:

```sh
milkdrift --command-id sequence-plan-validate-v1 \
  sequence validate examples/headless-dogfood-sequence.md
milkdrift --command-id sequence-plan-import-v1 \
  sequence import examples/headless-dogfood-sequence.md
milkdrift sequence show REVISION_ID
milkdrift --command-id run-plan-start-v1 --expected-revision REVISION_ID \
  run start RUN_ID WORKFLOW_ID REVISION_ID
milkdrift --json run timeline RUN_ID --limit 100 --follow
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

If verification omits the declared success artifact, no later stage becomes eligible. The failure
arm runs the independent reviewer and waits durably for approval. First pause the aggregate using
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

The frozen run grant remains authoritative. A proposed verifier/profile or other requirement
outside that envelope is rejected at revision adoption. The sequence document limits remediation
generation only. Runtime retry and authority budgets, adapter timeouts/admission bounds, artifact
budgets, and retention policies are configured and enforced by their owning layers; the import
does not pretend to add additional ceilings and never grants authority by itself.

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

The daemon integration test uses byte-pinned `tee` and `cp` process profiles, a temporary persistent
repository, real daemon/control-client HTTP, redb, authority evaluation, context publication,
ordinary workers, proposals, reconciliation, and restarts. It proves successful stage progression,
a deliberately missing verification-success fact, causal reviewer context, durable proposal and
adoption boundaries, fresh remediation, no duplicate task attempts, and persistent repository
content without network access or provider credentials.

The separate shell-free actual-binary scenario builds and spawns `milkdrift-daemon` and
`milkdrift`, never calls daemon internals from the client, and gives no CLI process the redb path.
It uses temporary storage/artifact roots, private bearer files, an ephemeral loopback port, and
byte-pinned deterministic process profiles. Run it from the repository root:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-cli --bin milkdrift \
  -p milkdrift-evidence --bin headless-cli-evidence
target/debug/headless-cli-evidence \
  --daemon target/debug/milkdrift-daemon \
  --cli target/debug/milkdrift
```

The scenario covers validation without storage, import/start replay and changed-payload conflict,
reads and pagination boundaries, pause/signal/resume, proposal approve/apply guards, artifact
integrity, abrupt-restart uncertainty retention, explicit resolution, durable reads after restart,
and machine failures for invalid input, authorization, conflict, not found, unavailability, and a
failed terminal run. The focused daemon overload test and CLI failure-classification unit test
cover the shared retryable overload exit category. None of this is real model interoperability
evidence.
