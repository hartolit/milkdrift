# Current coordinator and execution handoff

Coordinator: `Cedar-20260910-a` (agent pseudonym), 2026-09-10. The current task owns
[01-execute.md](01-execute.md). Independent review is unassigned and acceptance is pending.
Execution is complete: the clean candidate and inspected qualifying combined report are ready
for independent review. The operator's two local Bonsai instances completed the maintained
scenarios through the production daemon. Sprint acceptance and controller activation remain pending.

## Candidate and reviewed scope

The clean candidate is `8c6cdb9137bc8f688fd6cc800d3adf416d4f4625`, tree
`7ae25644b92906b4b88ad02e1d73f646174f5648`, on local branch
`codex/external-interop-20260910`, at
`C:/code/milkdrift/target/external-interop-20260910/candidate`.
The main workspace retains matching implementation, tests, schema, and guide changes as
uncommitted changes above `ca7ad60870c6407eaa36837b6e2ca6037d3778f2`. Office notes and the status
clarification about pending acceptance are separate documentation updates. Nothing was pushed.

The candidate contains the earlier process-report and helper-authority corrections: a qualifying
repair must preserve the repository commit/tree and produce a nonempty uncommitted diff, and
grants cover a separately installed Python helper's execute directory. It also resolves two
resource-fit problems and a report-finalization defect demonstrated during this execution:

- The fixed 64-unit model allowance returned no final text and `length` from Bonsai. The harness
  now accepts an explicitly bounded output allowance and records it in the task and report.
  The maintained response assertion is unchanged. The scenario, Rust validator, and consumer
  schema require normal `stop` completion, including when truncated text otherwise matches.
- A real agent connection probe took 105.5 seconds, beyond the old polling loop's nominal
  30-second window. Each workflow wait now has an explicit deadline covering daemon reads and
  sleeps. The process and model profiles retain their independent execution limits. A timeout
  preserves the session and does not authorize replay of an entered effect.
- Both real scenarios completed, but a duplicate preflight write validated their success before
  the aggregate qualification flag was updated. The harness now finalizes once and writes once.
  The writer owns secret/forbidden-field checks before publishing bytes, including generated
  daemon tokens on failed scenario paths. Regression tests cover completed real-mode-shaped
  data and refusal of sensitive fields without leaving a report file.

The [external evidence guide](../../../guides/external-evidence.md) explains the bounds and
completion requirement. These changes stay in the unpublished harness, its tests, and existing
v1 report consumers. No production adapter, public library API, provider family, workflow
primitive, controller lifecycle, or durable format changed. The final source/diff review traced
all wait and model-task callers and found no remaining fixed polling loop in this harness.

## Verification and binary origin

Unless stated otherwise, evidence paths below are relative to ignored
`C:/code/milkdrift/target/external-interop-20260910/`.

- `gate-bonsai-report-final/results.json`, its source identity, and named logs establish the full local
  Windows x86_64/MSVC gate with Rust 1.95.0: formatting, fixture prerequisites, all-target checking,
  709 workspace tests and 24 doctests, Clippy, warning-denying rustdoc, dependency audits, discovery,
  and all 24 repository contracts passed. Five manual longevity cases remain ignored.
  The harness coverage includes twelve binary tests and five external integration tests; the latter
  exercise the selected allowance, both failure paths, and refusal of truncated valid-looking text.
- `gate-bonsai/` retains the earlier gate's Clippy refusal of a test's `expect_err`; that test was
  corrected and the complete gate was rerun. The final gate supersedes it.
- `consumer-bonsai-schema.json` records draft 2020-12 schema validation, an accepted complete
  response, and five incomplete/missing finish refusals. Existing fixture evidence remains
  non-qualifying; it was not relabeled as real evidence.
- `bonsai-report-build-provenance.json` and `bonsai-report-build.log` are the clean-candidate build
  record for fresh `build-bonsai-report/`, including source equivalence to the gate, command,
  toolchain, timestamps, and SHA-256 identities of both binaries. Earlier `build/`, `build-final/`,
  and `build-bonsai/` outputs do not identify this candidate.
  The equivalence check excludes only office notes and the pending status clarification.
- Prior correction evidence remains under `gate-final/`, `fixture-final/`, and the accompanying
  schema/artifact inspection files. It establishes preparation only; current executable coverage
  comes from `gate-bonsai-report-final/`; `gate-bonsai-final/` predates the report correction.

These are local executor checks, not independent acceptance or new portability guarantees.
`bonsai-final-documentation.log` and `bonsai-final-checks.json` record the final documentation
contracts, diff check, source comparison, clean candidate, and unchanged binary hashes.

## Selected resources and preparation

Private profiles and resource provenance are at
`C:/Users/beho5/AppData/Local/Milkdrift/external-interop-20260910/`:

- `model-profile.json` selects capability `external-evidence-bonsai`, model
  `prism-ml/bonsai-27b`, and the operator's `http://127.0.0.1:1234` endpoint. It declares only
  streaming and system-role support, uses loopback `no_auth`, and sets explicit HTTP bounds.
  `model-preflight-64.json` retains the truncated response; `model-preflight-4096.json` records
  the exact required final text, normal completion, usage, identity/model metadata, and stream end.
  Neither is a combined qualifying report.
- `coding-agent-profile.json`, revision 5, pins the installed `codex-cli 0.153.4` executable's
  bytes and selects `prism-ml/bonsai-27b:2` through LM Studio. It consumes stdin, declares bounded
  stdout/stderr artifacts, and runs with a cleared allowlisted environment. Authentication sources
  are empty. Windows process-group/descendant guarantees remain explicitly false.
  The original source profile's missing writable-root entry was corrected to the exact next
  disposable repository, without changing the successfully preflighted executable or arguments.
- The CLI uses the existing elevated Windows sandbox with workspace-write restrictions and
  `CODEX_HOME=C:/Users/beho5/.codex`. Its own user config/rules are ignored, approval is disabled,
  and no hosted model is selected. The sandbox was already configured; no new administrative
  setup or unrestricted fallback was used. A native scratch path under `C:/code/milkdrift/target/`
  avoids the desktop application's virtualized LocalAppData path.
- `resource-provenance.json` records profile/instruction digests and the successful real agent
  repair preflight. That preflight lives at `agent-repository-preflight-elevated/` under the
  evidence root: exit zero, 198.7 seconds, and an independent unittest pass after the agent's
  edits. The weaker sandbox's two refused attempts remain in private storage. They did not edit
  their test source. The private copies preceding each profile correction are retained.

The real run uses 900 seconds per workflow wait and 4,096 model output units. Agent wall time
is separately bounded at 900 seconds. The preparation scripts and raw logs stay ignored/private.

## Real evidence and review handoff

`real-bonsai-01/report.json` is a retained non-qualifying preparation failure: the source process
profile omitted a required read-write root. Both scenarios are `not_run`; no effect was entered.
`bonsai-real.log` and `bonsai-run-provenance.json` identify that failed attempt.

`real-bonsai-02/` retains the failed finalization attempt from candidate `19d70c9`. Both scenario
functions completed, but the inconsistent aggregate flag prevented either report write.
`bonsai-run-02-provenance.json` and `bonsai-real-02.log` record the nonzero exit after 475.5 seconds.
`bonsai-02-failure-inspection.json` verifies all 27 stored artifact digests and the exact model
response, normal finish, and usage. No report was reconstructed or claimed from that session.

The final report is
`C:/code/milkdrift/target/external-interop-20260910/real-bonsai-03/report.json`, with BLAKE3 digest
`b3_182a6602a7276cee5e6daac4ae842fcc3612d0dd1891315392e08e89bd11a1bf`.
`bonsai-run-03-provenance.json` and `bonsai-real-03.log` record command success after 924.0 seconds.
The report has `fixture_mode: false`, `dirty_at_start: false`, and qualifying process, model,
and top-level results. It was produced directly by the corrected harness, without rewriting.

`bonsai-inspection.json` records consumer-schema acceptance and executor inspection of all
14 linked artifact references (38,422 bytes), the five required distinct successful process
invocations, two fresh coding-agent attempts, and unchanged repository commit/tree with the exact
nonempty diff digest. Both settled restart boundaries retain their sequences without duplicate
attempts. The model has two selected and two omitted context items, the exact required final text,
normal completion, and linked usage/provider metadata. Generated-token redaction, executable
bytes, rendered process-profile/policy digests, source identity, and binary/resource hashes pass.
The report and complete private session remain together under the ignored evidence destination.

Assign [02-review.md](02-review.md) to a separate session that did not implement this candidate or
produce its report. The reviewer needs the candidate, cited gate/build records, private profiles,
and complete `real-bonsai-03/` session. The coordinator must resolve accepted evidence retention,
integrate the candidate and documentation, and obtain an explicit acceptance disposition before
closing the sprint. No further real invocation is needed for this execution handoff.

The main [status](../../../product/status.md) update states that a qualifying local report exists
while independent acceptance is pending. After acceptance, the coordinator should describe this
Windows Codex/Bonsai coverage and its controlled restart limits in status and
[verification evidence](../../verification-evidence.md), then close the first roadmap item.
The guide already explains the bounds and finalization corrections. This evidence does not
qualify model quality, additional platforms, power loss, or production controller activation;
controller activation remains a separate assignment with its own evidence requirements.
