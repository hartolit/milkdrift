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

The separate [local-model lane](../guides/local-model-endpoint.md#run-the-maintained-daemoncli-lane)
checks wait/restart/release, selected/omitted context, streaming/usage/artifact provenance,
nonduplication, post-entry response loss, unsafe-retry refusal, and explicit retain. Its deterministic
mode is non-qualifying. Real mode requires an explicit separately managed loopback profile and
never falls back to a mock.

[Strict external evidence](../guides/external-evidence.md) requires both a real byte-pinned coding
agent and a supported real model endpoint with operator-owned resources. Only its complete
validated report can qualify that interoperability boundary. Hermetic external mode tests the
harness. It cannot authorize production controller activation.

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
| [quality](../../.github/workflows/quality.yml) | Linux full gate, actual CLI/daemon operator scenario with controlled capabilities, deterministic local model. |
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
