# Verification and operational evidence

This document defines the repeatable pre-UI evidence lanes. Correctness remains owned by the
ordinary test, lint, documentation, dependency, and focused regression gates. Benchmark numbers
are observations, not correctness thresholds, and are not used to conceal an unbounded design.

## Pinned tools and fixtures

- Rust is pinned to 1.95.0 by `rust-toolchain.toml`.
- Targeted mutation testing uses `cargo-mutants` 27.1.0 from `.cargo/mutants.toml` and the
  Cargo-native `mutation-evidence` binary owned by `milkdrift-evidence`.
- Microbenchmarks use Divan 0.1.21 as an exact development dependency of
  `milkdrift-evidence`.
- The local-process measurement executes the separately built, byte-pinned
  `evidence-process-helper`. It emits exactly 256 KiB on stdout and 256 KiB on stderr without a
  shell, network, credential, clock, or random input.
- Model-stream measurements use fixed OpenAI-compatible and Anthropic SSE documents through the
  production bounded SSE and provider state machines. No provider endpoint is contacted.
- Daemon measurements use temporary redb/artifact roots, private temporary bearer files, an
  ephemeral loopback listener, and controlled local concurrency. No fixed port or external
  service is required.

`milkdrift-evidence` is a development-only leaf package. Product crates do not depend on it. The
model-provider `operational-evidence` feature only exposes a network-free driver around its
existing private parser state machines; the feature is disabled by default and changes no
production mapping or policy.

## Measurement inventory

| Area | Measured operation | Representative fixture | Correctness owner |
| --- | --- | --- | --- |
| Persistence | one accepted journal transaction | one command and event | redb journal contract tests |
| Persistence | bounded journal batch | 64 events in one transaction | atomic/fault-boundary tests |
| Runtime | full projection rebuild | 4,096 durable events | projection and structured-runtime tests |
| Runtime | checkpoint plus tail | serialized projection plus 128 events | recovery/reconciliation tests |
| Application state | hot/cold lookup, replay, turnover | hot bound 8, archive batch 3 | application-state tests |
| Peer state | active/hot lookup, page/resume, compact replay | four executions, 68 observations, four tombstones | peer/redb fault tests |
| Context | metadata discovery and bounded selection | 2,048 candidates to 128 selections | causal-context tests |
| Context | selected-only materialization | 64 exact node-execution sources | materialization tests |
| Artifact | publication and range read | 1 MiB object, 256 KiB range | artifact contract tests |
| Local process | stdout/stderr streaming and publication | fixed 512 KiB combined output | local-process tests |
| Model provider | SSE plus provider state machines | 2,048 complete fixed responses | mock-endpoint tests |
| Daemon | authenticated owner round trip | loopback health request | control-plane tests |

The operational runner separately performs sustained receipt turnover and reopen recovery. It
replays a 10,000-event projection to assert a bounded current frontier, verifies old cold receipt
replay after reopen, records durable store bytes before and after final archival, and reports
primary/hot/cold receipt document counts and logical bytes. Its peer lane scales to the configured
operation count and reports final/peak active and hot counts, tombstones, observations, and logical
bytes observed for active, hot, compact, and observation documents. These logical document sizes
are distinct from the report's physical redb-directory bytes. The daemon scenario performs
sequential low/medium phases, a concurrent
saturated phase against an owner queue of one, checks the stable overload classification, keeps a
slow SSE consumer, reconnects with its authenticated cursor, verifies a post-overload request,
compares Linux `/proc/self/task` counts when available, and joins graceful shutdown. The separate
effect-worker regression uses a blocking controlled adapter with one worker and a queue of one to
prove fixed backpressure and truthful unresolved work on forced shutdown.

## Local commands

Install the mutation tool outside the workspace dependency graph:

```sh
cargo install cargo-mutants --version 27.1.0 --locked
```

List or execute one focused shard:

```sh
cargo mutation-evidence authority --list
cargo mutation-evidence authority
cargo mutation-evidence retention
cargo mutation-evidence runtime
cargo mutation-evidence uncertainty
cargo mutation-evidence controller
cargo mutation-evidence context
cargo mutation-evidence peer
```

The committed mutation scope is semantic: authority selector, validity, revocation, resource,
side-effect, and budget conjunctions; application and peer idempotency/accounting/archival;
optimistic runtime replay, recovery uncertainty, reconciliation, and controller bounds; context
selection/budgeting; and peer admission, claim, entry, cancellation, uncertainty, observation, and
archival transitions. It does not mutate generated fixtures or expand
to unrelated constructors merely to inflate a mutation count. `mutants.out/outcomes.json` is the
machine-readable result. A missed mutant must be fixed by a test or recorded by exact identity in
`.cargo/mutation-classifications.json`. The only accepted classifications are equivalent behavior,
unreachable under a validated public contract, or a mutation-tool limitation; unclassified survivors
fail the lane.

The seven current-source shards enumerate 546 focused mutants. The controller, runtime,
uncertainty, and retention shards pass against the current source: 169 mutants are caught, nine are
compiler-unviable, and the one surviving runtime reconciliation mutant has the existing exact
`unreachable_by_valid_contract` classification. The previous complete campaign
covered 398 mutants before authority, controller, and peer policy moved into their named child
modules, so it no longer qualifies the current source paths; a fresh complete campaign remains
required. Per-shard `mutants.out` directories retain source identities, logs, outcomes, and
generated classification reports. The Rust runner validates its strict classification policy,
rejects duplicate identities, and fails closed on timeouts or unclassified survivors.
Checksum-correct raw-row corruption tests exercise the peer primary-record and tombstone validators
instead of classifying their individual guards.

The controller admission release lane turns over exact final-entry reservations and logical
artifact charges across checkpoints and a redb reopen, then proves the terminal account has no
outstanding reservation. It complements the lifecycle-only controller checkpoint/restart lane;
both are explicit `--ignored --exact` release tests in `stress.yml`, not ordinary pull-request
tests. The hermetic external-evidence fixture remains an integration regression check and is
explicitly non-qualifying for real provider interoperability.

Deterministic clock boundary tests inject unavailability and rollback at inbound authority, remote
catalog registration, artifact transfer, post-entry worker recovery, daemon owner/health, and
restart boundaries. Redb tests independently prove that artifact acceptance and watermark
advancement commit or roll back together and that a reopened store refuses time behind durable
high-water evidence. These tests establish fail-closed software behavior; elapsed time while the
daemon is absent still relies on the operating-system clock trust stated in ADR 0029.

Build and smoke every benchmark once:

```sh
cargo build --release -p milkdrift-evidence \
  --bin evidence-process-helper --bin operational-evidence
MILKDRIFT_EVIDENCE_PROCESS_HELPER="$PWD/target/release/evidence-process-helper" \
  cargo bench -p milkdrift-evidence --bench core_paths -- --test
```

Capture distributions and machine-readable operational evidence:

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

The runner writes `operational-evidence.json` and `scenario-summary.csv`. Reports include scenario
identities, operation/byte counts, stable result checksums, storage/reopen facts, daemon accepted
and overload counts, latency distribution, stream/recovery/shutdown outcomes, platform identity,
Git commit/tree/dirty state queried from the checkout, and `rustc -vV` output. Raw credentials,
prompts, provider payloads, artifact content, environment values, and database internals are never
reported. Scenario identities explicitly distinguish synthetic candidate selection and in-memory
projection serialization from production durable discovery and snapshot-envelope recovery.

## Continuous integration evidence

- `quality.yml` is the required Linux formatting/check/test/Clippy/rustdoc/deny/machete gate.
- `platform.yml` checks all workspace targets and runs pure/domain/protocol/client/local-process
  contracts on pinned Ubuntu 24.04, Windows 2025, and macOS 15 runners.
- `mutation.yml` runs the seven focused weekly/manual shards with pinned cargo-mutants and uploads
  every complete `mutants.out` directory, including logs and JSON outcomes.
- `benchmarks.yml` runs the Divan smoke/full lanes, operational runner, and fixed effect-worker
  saturation regression, then uploads the text, JSON, and CSV evidence.
- `stress.yml` retains the receipt, peer, controller lifecycle, controller admission, and runtime
  bounded-frontier cases that are intentionally inappropriate for every pull request.

Checkout, Rust installation, and artifact upload actions use immutable commit SHAs. Jobs have
explicit timeouts, least-privilege read permissions, and workflow-level concurrency policy. The
platform logs are uploaded even on failure so OS-specific path, process, encoding, permission, or
cleanup defects remain inspectable.

## Interpretation and limitations

On 2026-09-01 the complete post-change local Linux gate passed: formatting,
all-target/all-feature check and tests, Clippy with warnings denied, rustdoc with warnings denied,
`cargo deny check`, and `cargo machete`. The full release evidence lane also passed with 256 receipt
operations and 256 peer executions/4,352 peer observations. It retained four hot and 252 cold
receipts, replayed the oldest cold receipt after reopen, and measured 1,589,248 physical store bytes
both before and after the final bounded archival transaction. The daemon queue bound was one; 70
requests were accepted and 186 received the stable overload result, accounting for all 256
requests. Stream observation/reconnect, post-load recovery, and graceful shutdown succeeded, and
the Linux task count remained 11. Exact latency distributions and logical byte breakdowns remain
in `target/evidence/operational-evidence.json` because they are machine- and filesystem-specific.

The harness does not claim production traffic shape, universal throughput, a memory allocator
profile, network/TLS performance, a real provider service-level objective, or sandbox strength for
trusted local processes. Divan isolates repeatable code paths but cannot replace end-to-end tests.
Store byte counts include redb allocation behavior and are useful for trend inspection, not a
portable quota. `/proc/self/task` is Linux-only; Windows/macOS still exercise lifecycle cleanup
through tests and CI logs.

Literal pre-UI closure additionally requires successful hosted runs of the Windows and macOS
matrix and the scheduled/manual mutation and benchmark workflows. A local Linux run can validate
their definitions and commands but cannot honestly substitute for those hosted results. Until the
new workflows have run successfully on their declared runners, status must say that cross-platform
and hosted evidence is configured and awaiting execution, not complete.
