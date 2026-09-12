# Verification and operational evidence

Use this guide to choose a reproducible check and understand what its observations support.
The ordinary [full gate](workflow.md#full-local-gate) checks executable changes; the additional
lanes below exercise application use, mutation sensitivity, sustained load, and external resources.
The [evidence package guide](../../tools/evidence/README.md) compares the tools and their entry points.
[Status](../product/status.md#current-validationevidence-snapshot) owns the latest executed state.

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

## Actual-binary scenarios

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

The installed controller scenario uses the same daemon composition with explicit development
activation. Build the daemon with `--all-features` (or `--features controller-qualification`), then
run `headless-cli-evidence` with the same `--daemon` and `--cli` paths plus
`--controller-qualification`. Optional `--controller-output NEW_DIRECTORY` retains the private
fixture configuration, pinned process profiles, exact proposals, accounting reads, and reports.
CI runs this deterministic scenario after building the real applications.

It composes ordinary process, verification, acceptance, proposal, approval, reconciliation, repeat,
and signal nodes. Failed work remains rejected; an ordinary workflow-control invocation submits
the artifact-linked repair, a distinct authorized actor approves it, the child adopts a new
revision prospectively, and independent verification accepts the repair. The next cycle stops at
four cumulative process entries. Separate concurrent workers compete for two entries; both active
reservations and the denied third attempt are inspected. Process/model entry counts are charged
at admission, while unit/cost/artifact remainders stay reserved until their owning evidence settles.

The scenario checks disabled start, refused production enablement, disabled recovery of an active
account, revoked approval, exact command replay after receipt archival, retained proposal/acceptance
artifacts after compaction, settled restart, and crash/reopen of an entered process with a retained
artifact obligation. These are process-kill boundaries, not power-loss or escaped-descendant proof.
Both disabled and qualification modes refuse an unmarked subworkflow placed before the marked
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
the unchanged model-admission count are retained separately. Production activation remains refused
as described in [daemon operation](../operations/daemon.md#controller-activation).

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
Hosted campaigns omit debug symbols and retain assertions. Authority/runtime use two disjoint
partitions each, and controller/peer use four each; the other groups use one. This distributes the
measured longer campaigns within the 180-minute job bound. Locally,
`cargo mutation-evidence peer --partition 0/4` runs the first partition. The pinned tool owns
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
The pinned tool times its baseline over mutated packages, while a shard may select more packages
for each mutation. Set its `CARGO_MUTANTS_MINIMUM_TEST_TIMEOUT` from a measured full selected-suite
run when that broader suite needs more time; an inadequate automatic deadline must be rerun, not
classified as a caught mutant. Hosted shards allow at least 600 seconds for that broader suite.
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
policy. A healthy benchmark cannot justify a survivor. Historical counts do not qualify new source.

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
| [quality](../../.github/workflows/quality.yml) | Linux full gate, actual CLI/daemon operator scenario, deterministic local model, and installed controller qualification/refusal scenario. |
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
