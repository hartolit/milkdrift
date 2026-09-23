# Verification and operational evidence

Use this guide to choose a reproducible check and understand what its observations support.
The ordinary [full gate](workflow.md#full-local-gate) checks executable changes; the additional
lanes below exercise application use, mutation sensitivity, sustained load, and external resources.
The [evidence package guide](../../tools/evidence/README.md) compares the tools and their entry points.
[Status](../product/status.md#current-validationevidence-snapshot) owns the latest executed state.

## Independent host execution

Build the daemon, CLI and `headless-cli-evidence`, then run:

```sh
cargo run -p milkdrift-evidence --bin headless-cli-evidence -- \
  --independent-host-only --daemon PATH_TO_DAEMON --cli PATH_TO_CLI
```

This uses actual daemon/CLI binaries, the production process adapter and a deterministic external
model HTTP endpoint. Public uploads and `invocation prepare/submit` produce useful outputs on an
execution-only host. Hot replay and complete teardown/restart preserve one entry, direct producers
and absence of workflow records. A second daemon runs workflows against the same serving operations;
the fixture asserts staged artifact/manifest inputs, real origin coordinates, imported useful outputs,
originating controller reservation/settlement and no serving-side synthetic runs. A third model
request reaches the external endpoint, which drops the reply. The origin retains an uncertain
attempt and the exact outstanding token/artifact allowance across restart; the external counter
remains three model requests in total (one direct, two remote). An uncertain outcome is not terminal
settlement and cannot release its reservation. The process marker records two useful entries, one
direct and one remote. It uses an explicit loopback development exception and does not
establish non-loopback TLS deployment, a physical second machine, or real model quality.

The harness prints idle process measurements before either role begins its workflow scenario.
On Windows x64 build 26200 with debug binaries, one 2026-09-18 observation measured execution-only
at 31 threads, 30,457,856 working-set bytes and 6,369,280 private bytes; workflow-enabled measured
35 threads, 31,608,832 working-set bytes and 6,275,072 private bytes. Both had the configured serving
workers and ran on the same machine. These are process snapshots with OS/runtime overhead,
not a portable performance promise or a live-inference memory measurement.

Focused serving tests cover prepared refusal/panic/revocation/expiry, caller separation, archival,
request-bound input staging and cumulative quota replay. Redb artifact tests reopen after interrupted
client uploads, reclaim only that owner through bounded pages, preserve workflow/peer publication
ownership, and verify imported controller charges across replay/reopen. Daemon role tests assert
closed-history preservation and active-obligation refusal. The full gate remains necessary for
changes to these shared boundaries.

The prepared-allowance negative tests were also run with their admission guard deliberately
disabled. Both detected one forbidden external entry instead of zero; both passed after restoring
the guard. The targeted commands and failure assertions are retained under
`target/adaptive-hosts/prepared-allowance-mutant.log` and `prepared-allowance-restored.log`.

## Governed Slotbook method and publication

Run the [maintained CLI qualification](../../examples/adaptive-slotbook/README.md) on a provisioned
rootless Linux host. It freezes the HTTP/API representation, uses explicitly seeded fixture failures,
submits a structured prospective repair, and obtains fresh trusted observations before deploying
immutable source. Its files preserve the original failure, proposal identity, accepted agreement,
new candidate and target generation. A fresh direct evaluation/publication and request replay after
reopen separately exercise the managed owner outside a workflow. Container identity/start time and
mounted bytes support the replay and exact-candidate observations. This does not qualify arbitrary
application behavior, model reasoning, power loss, hostile host administration or another machine.

`cargo test -p milkdrift-blueprint --test agreements` covers indirect dependency/interface/terminal
and scope edits. `cargo test -p milkdrift-control --test control_service agreements::` covers ordinary
automatic proposals, raw adoption refusal, inherited child pins and cumulative limits after reopen.
`cargo test -p milkdrift-managed-linux --test lifecycle protected::` supplies real redb transactions
and deterministic external-effect counters. It checks private failed/unknown evidence, artifact read
authority, concurrent revocation during preparation, policy changes at final entry, expiration,
wrong generation, exact bytes, lost effect responses and before/after evaluation-commit failures.
Those deterministic tests establish owner behavior without making an OS-isolation claim.

The opt-in `protected_verification` target exercises actual detached test containers and filesystem
state. Set `MILKDRIFT_PROTECTED_TEST_IMAGE` to an approved preloaded Python image identity, and
optionally `MILKDRIFT_LINUX_EVIDENCE_PARENT` to the directory for retained evidence, then run:

```sh
cargo test -p milkdrift-managed-linux --test protected_verification -- --ignored --nocapture
```

It checks fresh evaluations of unchanged bytes, cleanup after a verifier that omits its own cleanup,
cleanup after timeout, fencing a retained container on platform reopen, and candidate-file drift
refusal before configuration or startup. The CLI qualification separately proves that new evaluation
requests retain the prior evidence and can publish, including with a registry-qualified image digest.

Three deliberate guard removals in a disposable worktree made the graph-protection, failed-evidence
publication and candidate-file entry tests fail at their refusal assertions. Original code remained
unchanged. Commands, mutated diffs and failures are retained under
`target/adaptive-hosts/03-review/mutation-*`; successful owner suites are in the same directory's
final gate. The handoff records the exact execution profile
and physical observations so later publication/reuse work can preserve their finite provenance.

## Local process reporting cleanup

Run `cargo test -p milkdrift-local-process --all-features` for adapter conformance and lifecycle
regressions. The [reporting cases](../../adapters/local-process/tests/process_execution/reporting_cleanup.rs)
reject initial, stdout/stderr progress, and heartbeat reports while a real helper keeps its pipes
open. They observe child exit, propagated post-entry errors, and absence of a fabricated terminal
report. Deadline failures invoke fixture cleanup. Reporter panic, cancellation, timeout, output-limit
termination, and shutdown use the same child observation.

The [private ownership tests](../../adapters/local-process/src/process/lifecycle/tests.rs) hold stdin,
stdout, and stderr workers at completion while separately waiting for child absence. A worker
reaching its gate does not prove the cleanup thread has already reaped the child. The tests
establish that cleanup joins each started worker
before removing registration, including partial startup and unwinding, and that a duplicate
registration leaves the original cancellation control intact. Windows execution qualifies the
immediate child; the Unix-only reporting cases additionally check owned descendants when run there.
See [status](../product/status.md) for executed evidence and platform limits.

The inherited-pipe fixture creates a holder outside the owned group and synchronizes readiness
through its PID record. Tests own both explicit release and fallback termination. They require
invocation completion while the holder is still alive, so child termination alone cannot satisfy
the regression. Normal-output tests compare every final byte. Startup-failure injection and a
stopped Unix child exercise partial ownership and escalation when TERM cannot complete cleanup.
The shutdown regression pauses the monitor at its initial report, requests cancellation or shutdown,
then requires a fresh child-written marker before releasing the monitor. This detects competing
signal delivery even when an exited child remains visible as a zombie. After release, it requires
child absence, cancelled terminal evidence and no retained adapter load.

`cargo test -p milkdrift-daemon --test control_plane --all-features process_cleanup::` drives the
same byte-pinned helper through drain/cancel/retain shutdown, reopens the store, checks the durable
uncertainty explanation and refuses unsafe retry without another entry. Build the helper first.
This fixture deliberately delays holder startup beyond 800 ms. Readiness has a separate bounded
allowance; drain/retain request parent exit explicitly after shutdown is requested, while cancel
must terminate the parent itself. The three-second shutdown bound and live external-holder checks
remain independent of startup speed.
The platform workflow also runs these daemon cases and the capability-host lifecycle suite.
The per-platform uploaded logs, not cross-compilation, establish which OS paths were executed.

## Constrained peer placement

Constrained peer placement is exercised by
`cargo test -p milkdrift-daemon --test two_daemon_peer --all-features` using the maintained
`examples/operator/peer-placement.json`. Three production daemon host instances communicate through
real HTTP listeners on local TCP.
An origin with a local look-alike connects to two serving peers, each with its own temporary store,
credential and repository directory. Exact peer requirements admit a narrowed
grant, enter each approved process once, return verified core artifacts, and retain the same
requirement/peer/catalog provenance through reconnect and reopening all stores. The fixture uses
the production daemon host and configuration reader plus the built process helper. This is a
repeatable test topology for the implemented execution path. It does not establish physical
multi-machine deployment qualification.

Capability/authority/blueprint/sequence tests cover strict placement sets, immutable revision identity
and obsolete snapshot refusal. Runtime and capability-host suites exercise final-entry revocation,
deterministic
selection, local look-alikes, conflicting exact requirements, empty sets, stale health, removed
generations and catalog-update races with entry counters. Peer protocol/service tests retain exact
acceptance/replay/conflict, observation paging and expiry behavior. Artifact tests check the entered
execution's shared input/output allowance, exact publication replay at its bound, empty/corrupt
outputs, metadata ownership, and revocation before further bytes. Remote-adapter tests advance a
controlled clock across multiple HTTP chunks to prove lease renewal through publication. Refused
renewal and shutdown stop further downloads and release incomplete core staging. Run these through
the ordinary full gate; the operator/model/controller binary lanes below check the affected common
composition.

## Historical query cost

The integrated review measures the public daemon/client paths with an ignored reproduction under
`target/acceptance-08/query/`. It creates the ordinary process → signal wait → process workflow,
then appends 128, 1024 or 4096 settled terminal occurrences using atomic journal contracts. The
semantic frontier remains fixed: one live wait before release, four current nodes and no live
obligations after completion. Seeded checkpoint envelopes are verified by the production reader;
the daemon runs the actual processes and selects the early output as the late task's context.

The Windows build 26200 x86_64/MSVC Rust 1.95.0 debug comparison runs on an i9-13905H (20 logical
processors) with about 32 GiB RAM and uses source base `f80cce8` plus the acceptance diff.
Both binaries read the same reopened completed stores; every public attempt field must equal
the baseline result. Temporary counters were removed after building private measurement binaries.
They count decoded journal rows/pages and returned stored-event payload bytes, not physical disk
I/O, total allocation, TLS cost or operating-system cache misses. Three attempt samples use median
elapsed time; broad browsing and node/denial reads are individual observations.

| Settled occurrences | Attempt rows / pages / payload bytes before, per read | Near-start / near-end before | Near-start / near-end after | Execution entries retained before → after |
| --- | --- | --- | --- | --- |
| 128 | 427 / 2 / 161804 | 61 / 60 ms | 34 / 35 ms | 131 → 1 |
| 1024 | 3116 / 13 / 1013719 | 344 / 344 ms | 35 / 36 ms | 1027 → 1 |
| 4096 | 12331 / 49 / 3946426 | 1324 / 1327 ms | 32 / 35 ms | 4099 → 1 |

After correction each retained attempt reads one page of 15–17 events, about 17.6–18.8 KB. The
existing occurrence's creation/terminal anchors replace the whole-history reconstruction, and the
current-attempt path no longer scans again for peer identity already frozen in its snapshot. A
retired occurrence without an anchor uses two page-bounded passes and retains one owning execution.
No new index or cache is justified by this workload. Original revision, retry timing, exact prefix,
missing/discontinuous page refusal, whole-result equality and restart checks protect the change.
Historical reconstruction also recognizes recovery and reconciliation remediation creation. The
regression checks both full-history and anchored reads after later revision pins; reconciliation
work keeps its authorizing plan's target revision rather than the prior or latest run revision.
Runtime's existing rejected/missing optional-checkpoint tests remain applicable without modification.

Broad timeline browsing still returns 427 / 3116 / 12331 rows in 2 / 13 / 49 pages, with a peak page
of 256 events and 0.16 / 1.01 / 3.95 MB of event payload. Its post-change elapsed times are 92 / 619 /
2393 ms. Known execution reads require no event rows on these checkpointed stores (22–74 ms in the
post-change observations). A credential scoped to another run is refused before journal decoding:
zero rows and payload bytes at every size (9–27 ms). Existing context tests additionally verify
protected-artifact omission/redaction and post-claim permission refusal.

Causal discovery before late-task entry takes 49 / 59 / 60 ms. It reads four exact anchors and up
to two bounded tail pages: 415 / 516 / 516 rows and 145176 / 164185 / 165133 payload bytes. Its tail
limit is 512 records; maximum retained execution entries are 131 / 171 / 171, with one candidate,
at most one attempt, one revision-distance entry and one indexed source sequence. It selects one
authorized early artifact in all three histories. This is bounded by the configured discovery
budget, not constant memory for every possible task policy.

The separate missing-checkpoint probe shows why these figures are not universal latency promises:
at 1024 occurrences a near-start read takes about 2.2 seconds and a full browse about 24.8 seconds;
at 4096, a subsequent CLI command exceeds its harness deadline while replaying the uncapped tail.
This seeded probe deliberately omits normal checkpoint persistence. Keep snapshots available and
distinguish projection rebuild from exact historical lookup. Reconsider a rebuildable lookup index
only if measured retired-occurrence access remains an operator problem after using existing anchors
and cursors. Journal authority, exact replay and authority filtering must remain unchanged.

`query-summary.json`, `query-snapshot-before/`, `query-snapshot-after/`, the earlier cold probe,
instrumentation scripts and build logs retain the reproduction and raw samples under
`target/acceptance-08/`. These are machine-specific observations, not throughput guarantees.

## Controller activation acceptance

Explicit `enabled` activation is accepted by the integrated review. Default startup remains
disabled. The finite prerequisite list is closed by these owners and scoped observations:

| Prerequisite | Evidence and acceptance meaning |
| --- | --- |
| Concurrent final entry | Persistence/redb account contracts and control admission tests require exact revisions and reserve the final allowance atomically. The actual-binary race retains two process reservations and refuses another entry. |
| Crash/reopen and unknown usage | Runtime/host/model effect-stage and account suites retain uncertain effects and reservations through preparation, entry and reporting boundaries. The controller process-kill case reopens after lease expiry and again without resetting its account. |
| Artifact accounting and compaction | Atomic publication/charge, replay/abort/restart and exact-bound artifact tests pass; the binary lane archives receipts, retains linked acceptance/proposal artifacts and verifies unchanged public accounting after cold replay. |
| Approval and reconciliation | The installed binary loop refuses revoked approval, reopens at the approval hold, adopts an ordinary prospective revision and preserves failed history and exact command replay. |
| Mutation sensitivity | Retained accounting/model faults from `review-03a` still apply to unchanged owners. The acceptance diff additionally targets historical owner selection and omitted installation for explicit enablement. Raw fault outcomes remain under `target/acceptance-08/final/`. |
| Longevity | Release `revision_and_lifecycle::release_controller_longevity_stops_once_across_checkpoints_and_restart` and `admission::release_controller_admission_longevity_turns_over_reservations_artifacts_and_restart` exercise the unchanged lifecycle/account owners. Retained executed results are `target/review-03a-extra-results.json`. |
| Operational and full gate | Current Windows full gate and default-build operator/model/controller binaries cover the combined diff. Retained hosted benchmark/stress scope is recorded in status; it does not become new platform evidence. |
| Real external loop | `target/review-03a-real/report.json` records the authorized Windows Codex/Bonsai loop, separate approval, accepted repair, two settled direct-model calls and pre-transmission refusal of a third. Account/lifecycle/metering owners are unchanged; current fixtures cover later operational integrations. No additional cloud call is required. |

This decision changes source support for explicit configuration. It neither deploys an installation
nor qualifies arbitrary providers, managed sessions, thinking controls, physical peers or power loss.
The [status owner](../product/status.md#current-validationevidence-snapshot) records the accepted
configuration, source identities, current results and supported limits.

## Actual-binary scenarios

The deterministic `local-model-evidence` lane also exercises explicit model continuation. It
inspects a canonical predecessor, submits and adopts an ordinary prospective proposal, restarts
before release, and captures the continued request beside a Fresh request. Inspection exposes the
exact companion and remains unchanged after a second restart. A separate case gives a complete text
response to a tool-output acceptance contract: continuation must refuse that rejected answer before
scheduling or contacting its endpoint. Both cases use the actual daemon and CLI, not a provider
session service. Run the lane with the `evidence-process-helper` binary, as specified below and in CI.

`cargo test -p milkdrift-model-provider --test mock_endpoints --all-features` exercises both wire
mappings through runtime and the capability host. Continuation cases cover same-run source linkage,
sibling scope and other-actor references, missing/corrupt/unsupported evidence, retained-policy
refusal, bounds, current grant revocation, three-invocation chains, complete/incomplete tool pairs,
excluded traces in saved request messages, source corruption or read revocation after claim, later artifact publication,
and store reopen after selection. Model contract tests pin the companion/request fixtures and reject
cycles, depth overflow, unknown fields and instruction-role injection. Parser tests reject unsupported
response roles and content; sequence tests reject non-Fresh process imports. These mock lanes make no
real provider-session interoperability claim. The stream-cancellation unit regression forces a
cancellation during the body read before EOF, a transport error or malformed content returns;
the HTTP fixture holds its connection until cancellation has been acknowledged. Both preserve
uncertainty without claiming remote termination.

Offline storage composition is exercised by
`cargo test -p milkdrift-daemon --test storage_admin --all-features` and the structured-runtime
`context_enforcement::offline_binary_inspects_blocked_legacy_context_without_disclosing_or_rewriting_it`
test. Build the daemon first as described in the [workflow](workflow.md). These tests use temporary
current-format stores and private scratch: writer refusal, create-new backup/restore, source-byte
identity, clone execution refusal, and normal startup refusal beside redacted unsafe-context
inspection. Redb offline/artifact/account tests and peer retention tests establish exact cold
receipt and tombstone replay/conflict, unfinished publication offsets and outstanding-account
preservation. They do not establish filesystem power-loss or hostile OS-actor protection.

The structured-runtime `context_enforcement::recovery_binary` scenario starts that actual daemon
with `--recovery` over the same legacy fixture and configured retained grant. It submits, approves
and applies a prospective repair through the real control client, checks withheld context and
execution readiness, and reopens the daemon to prove exact receipt replay/conflict. Normal runtime
startup then completes the repaired run with one fresh invocation and unchanged prior evidence.
`context_enforcement::recovery_controls` covers both legacy failure classes and permanent runtime
effect/scheduler refusal. Control-service replay tests retain the recorded policy across a switch
to normal startup; daemon host tests verify shutdown without workers. These prove supported
safe-restart behavior, not general corruption repair or external-effect settlement.

The integrated review retains gate and deterministic binary-scenario logs under `target/review-main`
and default/all-feature API inventories under `target/public-api/review-main`. Its real-endpoint
model smokes use `target/review-main/profiles/bonsai-1.json` and `bonsai-2.json`; each saved session
and failure log records the 150-second harness deadline without terminal evidence. These failures
do not qualify the live endpoint or prove that generation stopped when the harness exited.

Build and run the headless operator scenario:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-cli --bin milkdrift \
  -p milkdrift-evidence --bin headless-cli-evidence
target/debug/headless-cli-evidence \
  --daemon target/debug/milkdrift-daemon \
  --cli target/debug/milkdrift --examples examples/operator
```

On Windows append `.exe` to executable paths. The scenario uses maintained operator files,
temporary redb/artifact roots, private bearer files, ephemeral loopback HTTP, deterministic
byte-pinned processes, and a controlled model endpoint. It checks starter setup, import,
replay/conflict, inspection, pause/signal/resume, guarded proposal adoption, artifact download,
abrupt-restart uncertainty resolution, durable reads, stable failure exits, and model provenance.
Every CLI action is a real process; no CLI receives the database path.

The installed controller scenario uses the same daemon composition with explicit `enabled`
activation. Build the daemon with ordinary default features or `--all-features`, then
run `headless-cli-evidence` with the same `--daemon` and `--cli` paths plus
`--controller-qualification`. Optional `--controller-output NEW_DIRECTORY` retains the private
fixture configuration, pinned process profiles, exact proposals, accounting reads, and reports.
CI runs this deterministic scenario after building the real applications.

It composes ordinary process, verification, model review, acceptance, proposal, approval, reconciliation, repeat,
and signal nodes. Failed work remains rejected; an ordinary workflow-control invocation submits
the artifact-linked repair, a distinct authorized actor approves it, the child adopts a new
revision prospectively, and independent verification accepts the repair. Two model calls complete;
the next model task is refused by their cumulative allowance. Separate concurrent workers compete for two process entries; both active
reservations and the denied third attempt are inspected. Process/model entry counts are charged
at admission, while unit/cost/artifact remainders stay reserved until their owning evidence settles.

The scenario checks disabled start, explicit enablement, disabled recovery of an active
account, revoked approval, exact command replay after receipt archival, retained proposal/acceptance
artifacts after compaction, settled restart, and crash/reopen of an entered process with a retained
artifact obligation. These are process-kill boundaries, not power-loss or escaped-descendant proof.
Both disabled and enabled modes refuse an unmarked subworkflow placed before the marked
repeat: its parent remains created and no child is admitted outside the cumulative account.
The process fixture emits bounded entry progress, lives at most ten seconds, and has a thirty-second
adapter deadline; concurrent verification uses a sixty-second execution lease.
The separate crash case uses a thirty-second lease and reopens after expiry; an unexpired lease
does not yet establish the recovery classification. Its non-idempotent process effect remains
uncertain with the same reservation through another reopen.

Its counting model fixture proves zero endpoint calls on unknown hard input-unit bounds. Up to
three explicitly supplied `--controller-model-profile PATH` options exercise the same negative
path against operator-selected profiles. Registration and local preparation must succeed before
the exact admission refusal is accepted as evidence. A refusal is not a qualifying model/controller
loop. Requested output units, endpoint/profile provenance, unknown effective server settings, and
the unchanged model-admission count are retained separately. Activation is an explicit operator
configuration described in [daemon operation](../operations/daemon.md#controller-activation).

The same scenario's `--controller-review-profile PATH` selects an explicitly approved loopback
model for two connected review stages. Real mode also requires `--controller-server-facts PATH`,
with bounded inspected settings and operator declarations matching that model and endpoint.
`--controller-agent-profile PATH` supplies a byte-pinned
local Codex/LM Studio launcher and creates a fresh disposable repository. The first agent writes
the deliberately incorrect 41; a separate JSON checker requires 42. A stopped or malformed model
decision prevents proposal submission. A supported decision is evidence for the ordinary proposal,
separate approval and future-only repair; the second checker and acceptance must pass. Restart
holds and final accounting prove that completed calls are not repeated. The two-call model
allowance then refuses a third task before transmission, retaining successful repair evidence;
the workflow stops failed on that deliberate refusal. Omitting the real-resource options runs
the same deterministic fixture lane. Process-ceiling evidence runs separately.

The corrective contract reserves the complete prepared body under the supported byte-BPE/template
contract. Unbilled final tokens settle without an invented monetary report. Billed fixtures cover
exact currency, cached/input/output rates and conservative rounding; unknown charge remains a
different refusal. Missing/contradictory final usage, over-envelope tokens, reporting failure,
revoked preparation and lost-stream restart are covered by adapter/account/host contracts. The
[local guide](../guides/local-model-endpoint.md#controlled-local-text-requests) owns reproduction,
server-contract obligations and the distinction between direct model calls and agent launches.

Current review evidence is retained under `target/review-03a-*`: gate/focused logs,
installed reports, selected profiles and server facts, source patches, and binary identities.
The successful real loop is `target/review-03a-real/report.json`: two direct model reviews
settle 6661 input and 102 output tokens, four process entries and 45487 logical artifact bytes.
Provider spend is zero, currency is absent, reservations are empty, and the repair is accepted.
An additional model task is rejected with `Limit { dimension: "model_admissions" }`, without
uncertainty or another admission. The workflow stops failed on that deliberate refusal; its
completed controller status has no `reached_bound` value. Recorded restart, replay, compaction,
concurrent entry and entered-process crash assertions pass. This single bounded run takes 477
seconds; the repair attempt succeeds, with no retry or allowance increase. The original corrective
reports remain under `target/corrective-03a-*` as evidence for their own source and binaries.

The reviewed deterministic controller report is `target/review-03a-controller/report.json`; the
ordinary operator log is `target/review-03a-operator.log` and the ordinary deterministic model
report is `target/review-03a-local-deterministic/report.json`. The scenario explicitly places the
third task after the completed-review restart hold. Earlier corrective fixture assertion failures
remain in their original evidence directories rather than being replaced by the passing runs.
The reviewed full gate passes 750 tests, 24 doctests and all 24 repository contracts, with five
manual tests ignored in the ordinary gate. Both required release controller longevity lanes pass
again. Four review faults in streamed usage overwrite, nullable detail parsing, raw usage retention
and permission-cost rounding are caught by assertions and restored. The model suite passes again.
Prior unspecified-unit, conflicting-charge and zero-spend-repeat faults remain applicable to those
unchanged owners in `target/corrective-03a-mutation-results.json`.
Exact commands and results are in `target/review-03a-final-gate-results.json`,
`review-03a-binary-results.json`, `review-03a-mutation-results.json`,
`review-03a-permission-mutation.json` and `review-03a-extra-results.json`. The final gate's initial
Clippy diagnostic identifies a redundant default initializer; after its removal, formatting,
full workspace checking/Clippy and the model suite pass. Eight default/all-feature inventories for
capability, model-provider, persistence and control under `target/public-api/review-03a/` match the
corrective inventories; the review fixes add no public surface.

Read-only configuration and bounded template/tokenization observations are retained in
`target/corrective-03a-server/observed.json` and `probes.json`. The one-token HTTP probe reports one
completion token; the oversized prompt is refused at the inspected 8192-token context. The SDK
process hit a Windows shutdown assertion after saving the probe results; these are retained
request observations, not a clean SDK lifecycle test. Before the reviewed real run, a fresh read-only
inspection confirmed the same accounting-relevant settings; its observations, comparison and copied
facts are under `target/review-03a-server/`. That inspection performs no loading or generation and
does not repeat the prior probes. No running server was reconfigured.
Earlier `target/model-budgets-*` reports remain evidence for their own source and settings, including
a stopped review and a 300-second agent timeout. Windows process termination/reopen does not
qualify power-loss durability or escaped descendants. Cloud testing was not authorized or performed.

The separate [local-model lane](../guides/local-model-endpoint.md#run-the-maintained-daemoncli-lane)
checks wait/restart/release, selected/omitted context, streaming/usage/artifact provenance,
nonduplication, post-entry response loss, unsafe-retry refusal, and explicit retain. Its deterministic
mode is non-qualifying. Real mode requires an explicit separately managed loopback profile and
never falls back to a mock. Both modes also run an isolated local-preparation refusal through
actual daemon/client binaries. A request-body limit rejects the attempt before entry intent;
the counting listener observes zero connections, and restart preserves that rejected attempt
without uncertainty or another request.

The deterministic lane also exercises production result acceptance with actual daemon/client
binaries: an empty exhausted model response retains a successful invocation, acceptance rejects
it, and a dependent endpoint stays unentered until an authorized repair produces usable review.
It restarts before evaluation, after rejection, and after acceptance before dependent dispatch.
Retained decisions, exact release-command replay, and endpoint entry counters check that restart
does not replace history or repeat work. The control tests cover structured-only and tool-only
contracts, whitespace, malformed decisions, checkpoint mismatch, justified no-change, unavailable
evidence, and failed publication. These mechanical checks do not qualify semantic review quality.

The September 12 acceptance change in the checkout based on `48fc896` passes the Windows/MSVC full
gate and both maintained actual-binary lanes. Its final deterministic report is
`target/result-acceptance-model-06/report.json`; the operator log is
`target/result-acceptance-final-headless.log`. Full-gate results and test counts are retained in
`target/result-acceptance-final-gate.json` and `target/result-acceptance-final-test-counts.json`,
with the corresponding `target/result-acceptance-final-*.log` files. The dependency audit logs are
`target/result-acceptance-deny.log`, `target/result-acceptance-machete.log`, and
`target/result-acceptance-duplicates.log`; affected public API inventories are under
`target/public-api/result-acceptance/`. The source and final binary hashes are recorded in
`target/result-acceptance-verification.json`.

Real model observations made during this change are retained separately. The first Bonsai alias
passes in `target/result-acceptance-real-bonsai-a/report.json`. The second alias times out at the
declared 180-second bound; `target/result-acceptance-real-bonsai-b.log` and that directory's
`recovered-run.json` and `recovered-attempt.json` retain the single uncertain attempt. Both requests
declare 4,096 output units and leave effective server thinking settings unknown. These earlier
observations cover the unchanged model/request path only; the final deterministic report covers
the finished acceptance and publication implementation. They are model-only observations, not a
new combined external-agent qualification or evidence of remote cancellation.

The model-preparation change based on `bf8cbd5` retains its actual-binary deterministic report at
`target/model-stages-deterministic-02/report.json` and its operator log at
`target/model-stages-headless.log`. The report covers zero-request refusal and restart, the existing
lost-response/unsafe-retry boundary, and result acceptance before dependent entry. The full suite
passes 727 workspace tests and 24 doctests; five manual longevity tests remain ignored. Gate results
are in `target/model-stages-gate.json` with corresponding `target/model-stages-*.log` files and
`target/model-stages-test-counts.json`. The initial duplicate-worker regression and its successful
correction are retained in `target/model-stages-initial-tests.log` and
`target/model-stages-duplicate-worker.log`. API inventories are under
`target/public-api/model-stages/`; `target/model-stages-verification.json` records source and binary
hashes. Preserved evidence binaries are under `target/model-stages-binaries/`.

Its real model smoke passes for `prism-ml/bonsai-27b` in
`target/model-stages-real-bonsai-a/report.json`. The separate `prism-ml/bonsai-27b:2` request reaches
the 180-second limit without a terminal. `target/model-stages-real-bonsai-b.log` and that output
directory's `recovered-run.json`, `recovered-attempt.json`, and `recovery-summary.json` retain two
reopens with one uncertain attempt and no replacement attempt. Both requests declare 4,096 output
units; effective thinking settings remain unknown. Only the first real lane completes, and neither
model-only observation qualifies the combined external-agent boundary or remote cancellation.

The reviewed September 12 controller integration based on `741b230` passes the Windows/MSVC full gate
(737 workspace tests, 24 doctests, all 24 repository contracts) and both required release controller
longevity lanes. Five manual tests remain ignored in the ordinary gate. Its rebuilt installed
scenario passes in `target/controller-review-scenario/report.json`; the ordinary operator log is
`target/controller-review-operator.log` and the deterministic model report is
`target/controller-review-model/report.json`. The installed report covers accepted prospective
repair followed by the exact cumulative stop, concurrent descendant admission, revoked approval,
cold replay, charged-artifact preservation, and an entered non-idempotent process remaining
uncertain through two reopens, plus the unaccounted-child refusal in both activation modes. The
counting model fixture proves conservative unknown-bound refusal. The prior supplied Bonsai profile
refusals remain scoped to `target/controller-scenario-final/`; review used deterministic fixtures
only. No qualifying real controller loop or thinking-mode claim is made. Production activation
stays refused.

`target/controller-review-verification.json` records the reviewed source hashes, base commit,
executable hashes, and checks. Exact binaries are preserved under `target/controller-review-binaries/`.
Gate results and logs are `target/controller-review-final-gate-results.json` and
`target/controller-review-final-gate-*.log`. Release logs are
`target/controller-review-lifecycle-longevity.log` and `target/controller-review-admission-longevity.log`.
Default/all-feature API inventories are under `target/public-api/controller/`; review introduces
no further exports. The verification command sequence is retained in
`target/controller-review-final.ps1` and `target/controller-review-final-gate.ps1`.
These local process-kill observations do not qualify power loss,
another platform, or an independently reviewed production operating decision.

[Strict external evidence](../guides/external-evidence.md) requires both a real byte-pinned coding
agent and a supported real model endpoint with operator-owned resources. Only its complete
validated report can qualify that interoperability boundary. Hermetic external mode tests the
harness. It cannot authorize production controller activation.

The accepted local combined report and its supporting evidence are retained under ignored
`target/external-interop-20260910/`. `acceptance.json` identifies clean candidate
`8c6cdb9137bc8f688fd6cc800d3adf416d4f4625`, its binary/build/run provenance, and the private session.
The report is `real-bonsai-03/report.json`, with BLAKE3 digest
`b3_182a6602a7276cee5e6daac4ae842fcc3612d0dd1891315392e08e89bd11a1bf`.
`review/accepted-handoff.md` retains the independent acceptance; `review/verification.json` and
`review/contract-validation.log` retain the artifact, journal, production-reader, and redaction
inspection. The exact binaries, full-gate logs, session, failed attempts, and private resource
profiles remain available at the paths in that record and handoff. Preserve this evidence before
clearing build output; Git retains source history, not these private files. Accepted coverage and
its limits live in [status](../product/status.md#current-validationevidence-snapshot).

The local Bonsai evidence does not qualify thinking mode. The operator subsequently reported
disabling thinking after the initial attempt; the retained endpoint profile does not bind that
server setting to each request. The truncated 64-unit probe and successful 4,096-unit probe
therefore do not isolate the effect of the larger budget. See the
[configuration evidence finding](virtual-office/whiteboard/issues/external-model-configuration-provenance.md).

The development-only evidence package shares one child lifecycle owner for deadlines, bounded
captured output, readiness, restart, CLI JSON decoding, and cleanup. Production configuration and
composition remain in the actual daemon. Build the daemon before isolated evidence-package tests.

## Mutation

The Cargo runner and [.cargo/mutants.toml](../../.cargo/mutants.toml) pin mutation tooling. Install
the tool outside workspace dependencies, then list or run a semantic shard:

```sh
cargo install cargo-mutants --version 27.1.0 --locked
cargo mutation-evidence authority --list
cargo mutation-evidence authority
cargo mutation-evidence retention
cargo mutation-evidence runtime
cargo mutation-evidence uncertainty
cargo mutation-evidence controller
cargo mutation-evidence context
cargo mutation-evidence peer
```

`CARGO_MUTANTS_JOBS` selects the campaign concurrency (default 2), and
`CARGO_MUTANTS_BUILD_TIMEOUT` sets a positive build deadline in seconds (default 180). A slow host
may require serial builds and a larger measured deadline. Preserve failed campaign artifacts and
rerun after infrastructure failures; a timed-out build or a file-lock failure is not evidence that
a mutant was caught. Finish workspace builds before running a campaign, particularly on Windows
where the live runner executable cannot be replaced.
On a host dominated by debug-symbol linking, Cargo's `CARGO_PROFILE_DEV_DEBUG=0` and
`CARGO_PROFILE_TEST_DEBUG=0` retain test assertions while omitting debug symbols. Record these
settings and `CARGO_BUILD_JOBS` alongside the campaign; they do not classify failed builds.
Hosted campaigns omit debug symbols and retain assertions. Every matching push selects the
full campaign. Authority/runtime use four disjoint partitions each, and controller/peer use eight
each; the other groups use one. These partitions divide mutation rebuilds and test runs among
more jobs, at the cost of more baseline builds. Total completion time also depends on hosted
runner availability; each job has a 180-minute timeout. Locally,
`cargo mutation-evidence peer --partition 0/8` runs the first partition. The pinned tool owns
partition parsing and selection; qualification requires the union of every partition to match the
unpartitioned mutant list exactly. Partitioning preserves the selected tests and enforces each
mutation's separate deadlines.
Hosted builds allow 600 seconds and use two Cargo compiler jobs. Each peer partition runs one
mutation worker so application tests do not compete with another mutation's compilation; its test
selection includes the redb owner's contracts as well as peer, daemon, process, and evidence tests.
The peer group excludes the three process/model-only external fixture scenarios, which configure
no peers and previously produced unrelated HTTP timeouts before peer assertions ran. They remain
in the full workspace gate and the authority/receipt groups. Peer service, daemon-peer, storage
corruption, and operational peer tests remain selected.
The runner passes each shard's complete package selection to Cargo for both the baseline and every
mutation. The pinned tool's `--test-package` option alone changes only mutation scenarios, leaving
the baseline dependent on which source packages happen to occur in a partition. A baseline must
exercise the same suite to detect missing prerequisites before they can make a mutation look caught.
Shards that run the retained-context binary tests include the daemon and shared process-helper
packages. Cargo builds their executables inside each isolated checkout, including any mutation;
prebuilt binaries from the caller's workspace cannot supply that evidence.
The runner prints each completed mutation, including caught and unbuildable cases, so CI logs
show progress during long campaigns instead of remaining silent until the final summary.
Set `CARGO_MUTANTS_MINIMUM_TEST_TIMEOUT` from a measured full selected-suite run when the host needs
more time; an inadequate automatic deadline must be rerun, not classified as a caught mutant.
Hosted shards allow at least 600 seconds for that suite.
Mutation checkouts retain Git metadata because application-evidence tests require exact source
provenance; a missing repository must never make a mutation look caught.

Scope covers authority conjunctions, application/peer idempotency and retention, runtime optimistic
replay/recovery/reconciliation, controller accounting, context budgets, peer lifecycle, and exact
catalog renewal with bounded registration retirement. It
excludes generated fixtures and unrelated constructors. A failing unmutated baseline cannot
qualify a campaign.
Controller transaction revalidation is included. Independent contract tests cover originating-run
binding, action bounds, exact revision guards, and canonical fingerprints. Cargo-mutants 27.1 skips
constructors named `new`; constructor fault injection must be recorded separately from its campaigns.

Retain each `mutants.out` directory with exact source identity, logs, and `outcomes.json`.
Inspect the actual failing test: an unrelated fixture timeout cannot qualify a caught mutant.
Cargo may stop at an early failing target before later owner tests run. Recheck each affected exact
mutation through its relevant owner contracts, using `--cargo-test-arg=--no-fail-fast` when needed,
and retain both the original failure and the recheck evidence.
Unclassified survivors and timeouts fail. Fix missing assertions or record an exact reviewed entry
in [.cargo/mutation-classifications.json](../../.cargo/mutation-classifications.json). Accepted
classifications are only equivalent behavior, unreachable under a validated public contract, or
mutation-tool limitation. The runner rejects duplicate identities and validates its classification
policy. When source moves, review the affected contract again and refresh its exact mutant identity
from the current list; a stale line number must not silently match another mutation.
A healthy benchmark cannot justify a survivor. Historical counts do not qualify new source.

The September 12 controller integration based on `741b230` has a focused changed-line campaign,
not a new full controller-shard qualification. Across 46 exact mutants, 36 are caught, five are
classified under validated contract invariants, and five are ill-typed `Default` substitutions.
`target/controller-mutation-summary.json` maps every selected identity to its retained outcomes
and classification; `target/controller-mutation-failure-review.json` identifies the failing tests.
The main all-feature run selects control, daemon, persistence, and runtime libraries plus
`control_service`, `structured_runtime`, and `durable_runtime`. Default-feature configuration and
exact runtime follow-ups close feature-dependent and child-pin coverage. An initial entry-test
timeout is preserved; after adding a bounded wait, that exact mutant is caught, not classified
from its timeout. A diagnostic run that also selected unchanged struct fields is excluded from
the changed-line result and retained separately. No unresolved selected mutant remains.

Commands and lists are retained in `target/controller-mutation.ps1`,
`target/controller-default-mutation.ps1`, `target/controller-mutation-exact-closeout.ps1`, and
`target/controller-mutants-selected.json`. Runs use cargo-mutants 27.1.0, one mutation worker,
two compiler jobs, and zero dev/test debug symbols with assertions retained. The main build/test
deadlines are 600/120 seconds; exact runtime follow-ups use 600/60 seconds. Raw outcomes remain
under the corresponding `target/controller-mutation-*/mutants.out/` directories.

Review's additional child-creation regression observes one unaccounted adapter entry on the
original implementation (`target/controller-review-child-baseline.log`). Inverting the new exact
controller-marker comparison is independently caught in `target/controller-review-child-mutant.log`.
The original source was restored before final verification. The prior 46-mutant results apply to
their unchanged production functions; this additional fault is separate from that campaign.

## Benchmarks and operations

Divan is pinned in the evidence [manifest](../../tools/evidence/Cargo.toml). The process fixture
emits 256 KiB to each of stdout and stderr with no shell, network, credentials, time, or randomness.
Model measurements feed fixed OpenAI-compatible and Anthropic SSE through production parsers.
The non-default provider `operational-evidence` feature exposes only that network-free driver.

```sh
cargo test -p milkdrift-evidence --test operational_contracts --all-features
cargo build --release -p milkdrift-evidence \
  --bin evidence-process-helper --bin operational-evidence \
  -p milkdrift-daemon --bin milkdrift-daemon
MILKDRIFT_EVIDENCE_PROCESS_HELPER="$PWD/target/release/evidence-process-helper" \
  cargo bench -p milkdrift-evidence --bench core_paths -- --test
```

Capture distributions and operational reports on Unix:

```sh
mkdir -p target/evidence
MILKDRIFT_EVIDENCE_PROCESS_HELPER="$PWD/target/release/evidence-process-helper" \
  cargo bench -p milkdrift-evidence --bench core_paths 2>&1 | tee target/evidence/benchmarks.txt
MILKDRIFT_EVIDENCE_PROCESS_HELPER="$PWD/target/release/evidence-process-helper" \
  target/release/operational-evidence --operations 256 --output target/evidence
cargo test --release -p milkdrift-capability-host --test effect_worker \
  bounded_queues_backpressure_and_forced_shutdown_preserves_unresolved_truth \
  -- --exact --nocapture
```

Benchmarks cover journal transactions/batches, projection rebuild/checkpoint tail, receipt turnover,
peer hot/compact replay, context discovery/selection/materialization, artifact publication/ranges,
process/model streams, and authenticated daemon round trips. Their fixtures and dimensions live in
[core_paths.rs](../../tools/evidence/benches/core_paths.rs); this guide is not a generated inventory.

The operational runner asserts a bounded frontier over 10,000 events, cold-receipt replay after
reopen, and sustained receipt/peer turnover. It distinguishes logical document bytes from physical
redb-directory allocation. Daemon phases measure sequential/concurrent load, bounded-queue overload,
slow SSE consumption, authenticated reconnect, post-overload recovery, and public Ctrl-C shutdown.
Recovery and reconnect use separate bounded phases; reconnect requires a fresh health observation
with a changed cursor. Retryable stream errors alone never establish reconnection.
The signal lane requires Unix; child thread counts use Linux `/proc` when available. A separate
blocking-adapter regression checks fixed worker backpressure and unresolved forced-shutdown truth.

Reports are `operational-evidence.json` and `scenario-summary.csv`, with scenario identity,
operation/byte counts, checksums, storage/reopen facts, latency, overload, stream/shutdown outcomes,
platform, Git commit/tree/dirty state, and `rustc -vV`. Synthetic selection and in-memory projection
serialization are distinguished from durable discovery and snapshot recovery. Credentials,
prompts, provider payloads, artifact bytes, and environment values are excluded.

## CI and qualification

| Workflow | Configured evidence |
| --- | --- |
| [quality](../../.github/workflows/quality.yml) | Linux full gate, actual CLI/daemon independent-host and operator scenarios, deterministic local model, and installed controller qualification/refusal scenario. |
| [platform](../../.github/workflows/platform.yml) | Pinned Ubuntu, Windows, macOS checks and selected domain/protocol/client/process tests. |
| [mutation](../../.github/workflows/mutation.yml) | Seven weekly/manual shards and complete mutation artifacts. |
| [benchmarks](../../.github/workflows/benchmarks.yml) | Smoke/full distributions, operational reports, worker saturation. |
| [stress](../../.github/workflows/stress.yml) | Receipt, peer, controller lifecycle/admission, and runtime frontier longevity. |

Actions use immutable SHAs, jobs have timeouts/read permissions/concurrency limits, and platform
logs remain available on failure. Workflow definitions establish configured lanes, not successful
execution. Closure requires successful runs on their declared hosts for the source being qualified;
a local or cross-target check cannot substitute.
The mutation, benchmark, and stress workflows also run when their own workflow file changes, so
edits to manual/weekly evidence commands are checked before their next scheduled run.
Mutation, benchmark, and stress lanes also run for Rust source and Cargo manifest/lockfile changes.
Mutation configuration edits also trigger its checks; its uncertainty group covers external text
normalization and bounded reasons as well as retained-effect classification.
A newer push supersedes the older mutation run on the same branch.

Deterministic fault/reopen, clock rollback, reservation/artifact, conformance, and corruption tests
prove software invariants, not filesystem power-loss behavior, sandbox strength, provider service
levels, or production traffic capacity. OS time remains trusted during daemon downtime. Physical
store size is a trend observation rather than a portable quota; benchmarks are neither universal
throughput guarantees nor allocator/network/TLS profiles. Keep raw reports, API inventories, and
timings outside source control. Report unavailable credentials, profiles, tools, or runners explicitly.

## Managed Linux installations

Run `cargo test -p milkdrift-managed-linux --all-features` and the redb `managed::` contract cases
for deterministic intent/action/result faults, exact receipt replay, guarded child transfer,
physical-evidence refusal, cancellation and restored-store fencing. Recovery permission tests use a
replacement with different resource requirements; transaction tests independently enforce originating
scope when a command key already exists. Concurrent-command regression checks prevent duplicate
platform drivers; service-start checks refuse foreign ownership before invoking the supervisor,
platform names and labels separate identical requests under different manager roots, and
prerequisite tests require enough UID/GID ranges for a worker beside its owned model. The lifecycle and attached-model
adapters use common conformance with real serving acceptance/artifact storage, prepared-envelope
allowance checks and a bounded model HTTP fixture. These establish contracts, not Linux containment.

The opt-in actual Podman/Quadlet lane and its exact environment variables are in
[managed operations](../operations/managed-linux.md#platform-qualification). It includes common
worker conformance through the production resource/adapter path. Normal CI does not silently skip
missing prerequisites within that lane: an explicitly invoked lane fails without its recipe,
rootless engine or user session. Qualification must retain the actual image/model digests, Podman
and systemd versions, observed kernel limits/devices/backend, and preservation/removal outcomes.
A daemon reopen is not a host reboot; GPU configuration/log evidence does not alone qualify useful
GPU inference or shared-memory pressure on the UM790.

The 2026-09-22 review's full gate and API results are retained under
`target/adaptive-hosts/managed-02-review/`; the [handoff](virtual-office/adaptive-hosts/handoffs/02.md)
records the optimization settings and the initial physical gaps. The follow-up evidence under
`target/adaptive-hosts/managed-02-qualification/` closes desktop Podman 6.1.2 worker and owned-CPU
lifecycle qualification, actual kernel limits, supervisor recovery without Milkdrift, ID-bound
cleanup under a real foreign-container collision, and the complete daemon/CLI path. It includes
concurrent model progress, busy-removal refusal, replay/restart and preserved data. The physical
lane retains its store on failure. Machine reboot and managed UM790/Vulkan qualification remain
explicit in the linked handoff and hardware issue. The actual daemon/CLI
Gemma smoke is negative evidence: one entered attempt remains uncertain, without another entry,
when the strict response reader encounters LM Studio's unmapped `reasoning_content` deltas.
`gemma-final.log` and its retained session store preserve the result; `gemma-diagnostic-sse.txt`
records a separate bounded diagnostic request. No provider mapping or server setting was changed.

The follow-up desktop Ornith and Drifty smoke reports use an explicit per-request
`reasoning_effort: "none"`; their passing text streams do not qualify the default reasoning format.
Drifty uses an authenticated SSH forward to its NetBird-bound listener. The managed desktop model
call additionally passes finite-accounting admission and terminal settlement. Unknown optional
reasoning fields remain refused, and no remote-termination or inference-throughput claim is made.


Post-02 evidence for recipe schema 2 and mechanism v3 is retained under
`target/adaptive-hosts/post-02/`. It repeats the physical worker/owned-CPU and collision lanes with
independent container limits, a configurable alias and a BusyBox worker that has no Git/Rust/C
suite. The maintained Slotbook image binds its initial brief; actual daemon/CLI execution checks
its ordinary initializer and preservation of edited files. Both product paths exercise exact
replay after restart, headroom diagnosis, incompatible apply refusal, explicit update and safe
removal. Short varied deadline/cancellation and combined-output tests complement the existing
retained-uncertainty and lifecycle fault cases. Alternate model inputs and larger configuration
values have deterministic reader/generation coverage; only the recorded Ornith weights have new
owned physical evidence. The [02 handoff](virtual-office/adaptive-hosts/handoffs/02.md) identifies
commands, inputs, final gate and API review. This supersedes recipe-1 evidence for changed
configuration consumers and leaves the existing hardware issue open.
