# Development

This document owns local build, format, test, lint, dependency, and fixture-update commands.

Use Rust 1.95.0 as declared by `rust-toolchain.toml` and the workspace
`rust-version`. CI uses the same exact toolchain:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
cargo deny check
cargo machete
cargo tree --workspace --duplicates
cargo test --workspace --all-features -- --list
```

Focused daemon/control-plane checks are:

```sh
cargo test -p milkdrift-control-protocol --all-features -- --nocapture
cargo test -p milkdrift-control-client --all-features -- --nocapture
cargo test -p milkdrift-daemon --test control_plane control_workflows::daemon_auth_startup_readiness_and_authority -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane control_workflows::scoped_read_matrix_and_continuations_fail_closed -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane durability::daemon_command_idempotency_restart_and_stale_conflict -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane operations::daemon_bounded_overload_returns_stable_error -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane operations::daemon_stream_reconnect_auth_rotation_and_shutdown -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane operations::daemon_graceful_shutdown_and_restart -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane operations::daemon_configured_process_adapter_executes_to_terminal -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane durability::layout_is_optimistic_restart_durable_and_semantically_inert -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane durability::proposal_listing_uses_durable_projection_and_survives_restart -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane control_workflows::prompt_sequence_validate_import_inspect_and_restart_are_one_control_path -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane control_workflows::headless_dogfood_failure_remediation_and_restart_are_durable -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane operations::daemon_startup_refuses_legacy_sidecar_and_peer_prototype_authority -- --exact --nocapture
cargo test -p milkdrift-cli --all-features -- --nocapture
cargo test -p milkdrift-prompt-sequence --all-features -- --nocapture
cargo test -p milkdrift-local-process --test process_execution authorized_host_working_directory_ -- --nocapture
```

The prompt-sequence contract suite covers JSON/Markdown equivalence, ordinary-node compilation,
canonical digests, hostile shell-shaped fields, duplicates, and prompt/document bounds. The
headless daemon proof uses a temporary repository and real byte-pinned local processes with no
network or credential. It crosses proposal-created and revision-adopted restart boundaries, proves
failure gating and causal reviewer context, and verifies exact-once attempts plus persistent files.

Focused capability-adapter contract checks are:

```sh
cargo test -p milkdrift-capability-host --all-features --no-fail-fast
cargo test -p milkdrift-local-process --all-features --test process_execution local_process_adapter_passes_shared_conformance -- --exact --nocapture
cargo test -p milkdrift-model-provider --all-features --test mock_endpoints model_endpoint_adapter_passes_shared_conformance -- --exact --nocapture
cargo test -p milkdrift-control --all-features --test control_service workflow_control_adapter_passes_shared_conformance -- --exact --nocapture
cargo test -p milkdrift-peer-http --all-features --lib remote::tests::remote_capability_adapter_passes_shared_conformance -- --exact --nocapture
```

The feature-gated capability-host harness owns the shared lifecycle, exact reporting, reporter
failure, cancellation, health, and canonical host-drain assertions. Each command supplies a fresh
mechanism-specific production fixture and declares only its legitimate start replay, stateless
health, and unknown-cancellation differences. Capability-host registry and effect-worker tests add
failed-start cleanup, lifecycle/admission/execute/cancel panic containment, exact cancellation
correlation, bounded concurrency, in-flight drain behavior, and registry-lock re-entry evidence.

Focused application-persistence and cross-transaction recovery checks are:

```sh
cargo test -p milkdrift-redb-store --features test-admin --test application_state -- --nocapture
cargo test -p milkdrift-redb-store --features test-admin --test contracts runtime_acceptance_reconciles_external_receipt_without_competing_effect_authority -- --exact --nocapture
cargo test -p milkdrift-redb-store --features test-admin --test contracts command_fault_boundaries_are_atomic_and_replayable -- --exact --nocapture
```

Focused peer protocol, durability, artifact, reconnect, and real two-daemon checks are:

```sh
cargo test -p milkdrift-peer-protocol --all-features --test protocol -- --nocapture
cargo test -p milkdrift-peer-http --all-features --test peer_service -- --nocapture
cargo test -p milkdrift-daemon --all-features --test two_daemon_peer -- --nocapture
```

They use temporary redb/core-artifact stores, ephemeral loopback listeners, local bearer files, controlled adapters, and one pinned local `/bin/echo` profile. They require no internet, VPN, provider credential, or manual peer service. Coverage includes exact v1.2 negotiation, hostile bounds/duplicates, HTTPS/loopback safety, atomic final-slot admission, exact replay/conflict across response loss and restart, durable queue/claim/entry recovery, post-entry clock-loss convergence without duplicate adapter entry, deterministic transaction and spawn faults, fixed worker/shutdown bounds, append-only cursor pages, cancellation before/after entry and terminal, revocation history, late terminal evidence, fail-closed remote catalog/authority/artifact expiry, core artifact resume/deduplication/digest/provenance/range reads, prototype-directory refusal, reconnect registration replacement, and real two-daemon remote process execution. Redb and daemon unit lanes additionally prove transactional clock-watermark advancement, health evidence, and restart rollback refusal before readiness.

These tests use temporary redb/artifact roots and ephemeral loopback listeners only. They cover fail-closed configuration/authentication, server-owned actor mapping, workflow/run/revision/proposal/layout/artifact/capability/provider/health scope, protected metadata/content audit, cross-actor cursor rejection, grant-narrowing page and stream failure, queue overload, exact accepted/rejected command replay across restart, stale layout guards, first-class proposal discovery, credential rotation, legacy-sidecar refusal, startup corruption, runtime/receipt crash reconciliation, and ordered shutdown. They require no internet or real credential. Run the daemon manually with `cargo run -p milkdrift-daemon --bin milkdrift-daemon -- --config PATH`; run the client with `cargo run -p milkdrift-cli -- [GLOBAL OPTIONS] COMMAND`. Keep bearer values in a private file or a referenced environment variable, never a command argument.

Focused causal-context and model-adapter checks are:

```sh
cargo test -p milkdrift-blueprint --test kernel -- --nocapture
cargo test -p milkdrift-model --test contracts -- --nocapture
cargo test -p milkdrift-runtime --test causal_context -- --nocapture
cargo test -p milkdrift-model-provider --test mock_endpoints -- --nocapture
cargo test -p milkdrift-runtime --all-features --test structured_runtime causal_context_production -- --nocapture
cargo test -p milkdrift-capability-host --test materialization -- --nocapture
```

Focused workflow-control checks are:

```sh
cargo test -p milkdrift-control --test proposal_contracts -- --nocapture
cargo test -p milkdrift-control --test authority_policy_controller -- --nocapture
cargo test -p milkdrift-control --test control_service -- --nocapture
cargo test -p milkdrift-persistence controller_account --all-features -- --nocapture
cargo test -p milkdrift-redb-store --features test-admin --test contracts controller_account -- --nocapture
```

These cover canonical/digest-bound and hostile proposal documents, exact ordinary-grant preset
behavior, explicit bounded controller construction, low-risk reviewer insertion, approval before
terminal changes, stale/no-effect rejection, deterministic replay, restart recovery, and malformed
control-capability input becoming a normal rejected terminal. They use only temporary local redb
stores, the deterministic executor, and fixed clocks/identities; no network, model endpoint, or
credential is required.

The model-provider suite binds local ephemeral loopback listeners and uses fixed mock
OpenAI-compatible/Anthropic payloads. It performs no internet requests and needs no real
credentials. The context suites prove deterministic ordering across candidate page order,
branch isolation, exact budget behavior, fail-closed authority, schema/hostile decode, and
restart-safe manifest publication. The production vertical additionally exercises architecture and
implementation artifacts, a failed verification plus retry, an isolated sibling branch, join
exposure, reviewer materialization, exact output provenance, and redb reopen. The host
materialization suite proves that a process profile can explicitly request the reserved manifest
input without changing ordinary invocation inputs. The process suite uses a small Rust helper binary rather than shell scripts or a real coding
agent. It has no network or credential dependency. After cancellation/tree tests on Unix, check
that no helper survived with `pgrep -af milkdrift-process-test-helper`; a match belonging to the
test command itself is not a surviving child. Live-host/worker coordination uses condition
variables and bounded queues. Timing loops are limited to OS-process observation and always have
hard deadlines.

Apply formatting with `cargo fmt --all`. Golden JSON is hand-reviewed compatibility data under each crate's `tests/fixtures` directory. To update a fixture, change the schema implementation and fixture in one review, run its exact canonical re-encoding test, and record any compatibility decision that changes reader behavior in an ADR. Never regenerate fixtures merely to make a failing test disappear.

Dependencies must be stable crates.io releases with a concrete use in current code. Run
the following required audit after any dependency or lockfile change; CI installs pinned
`cargo-deny` and `cargo-machete` versions:

```sh
cargo tree --workspace --duplicates
cargo machete
cargo deny check
```

Git dependencies require a documented necessity.

## Mutation, benchmark, and operational evidence

The repeatable fixtures, exact pinned tools, mutation classification policy, report schema,
cross-platform matrix, local commands, and interpretation limits are owned by
[`verification-evidence.md`](verification-evidence.md). The shortest local evidence pass is:

```sh
cargo test -p milkdrift-evidence --test operational_contracts --all-features
cargo build --release -p milkdrift-evidence \
  --bin evidence-process-helper --bin operational-evidence
MILKDRIFT_EVIDENCE_PROCESS_HELPER="$PWD/target/release/evidence-process-helper" \
  cargo bench -p milkdrift-evidence --bench core_paths -- --test
MILKDRIFT_EVIDENCE_PROCESS_HELPER="$PWD/target/release/evidence-process-helper" \
  target/release/operational-evidence --operations 256 --output target/evidence
```

Run `cargo mutation-evidence SHARD` for the seven focused mutation areas. Do not accept a
survivor because a benchmark looks healthy: add the missing correctness assertion or record an
exact reviewed classification in `.cargo/mutation-classifications.json`.

For a public-surface review, follow
[`../reference/public-api-policy.md`](../reference/public-api-policy.md). Install `cargo-public-api` as
local tooling (it is not a workspace dependency), then inventory each library package with both
`cargo public-api -p PACKAGE -sss --all-features --color never` and the default-feature equivalent.
The tool uses a nightly rustdoc JSON toolchain, while all product builds and gates continue to use
the pinned stable toolchain. Store raw reports under `target/public-api`, compare them against
actual production, test, application, and documentation consumers, and review test/evidence
features separately. A lower count is evidence, not permission to hide a real port or wire
contract.

Repository contract checks keep canonical links/version statements aligned with source constants,
guard dependency direction, keep public re-exports explicit, and prevent narrowed exports from
returning:

```sh
cargo test -p milkdrift-evidence --test repository_contracts --all-features
```

Keep module ownership visible. A source file that accumulates multiple lifecycle or
domain responsibilities must be split into real Rust child modules; `include!` and
facade-wide wildcard re-exports are not substitutes for module boundaries. Production
files approaching roughly 1,000 lines require an explicit cohesion review. The repository
contract rejects production sources above 1,000 lines unless their exact path has a meaningful
review rationale and a bounded ceiling. Missing, stale, duplicate, over-broad, and exceeded
exceptions fail the contract; test/evidence sources are classified separately and cannot grant a
production exception. Every Rust source remains below the 2,000-line hard backstop, and `mod.rs`
is rejected; use `owner.rs` with named `owner/child.rs` modules. Integration tests must be grouped
by behavior with shared support kept separate.

## Storage-boundary stress tests

The ordinary projection long-run checks are named, non-ignored tests and should remain
part of normal validation. They report compacted active-state metrics with `--nocapture`:

```sh
cargo test -p milkdrift-runtime \
  projection::tests::bounded:: \
  -- --nocapture
```

The weekly/manual stress workflow also runs the ordinary-task, fork/join,
subworkflow/repeat, revision-node-churn, unmatched-signal-budget, and pre-start
worker-lease-churn 10,000-cycle cases individually in release mode. Keep their exact
names synchronized only in the workflow; use the bounded-module filter above for local
development.

The ordinary workspace test suite excludes five tests whose fixtures intentionally cross large
persistence or operational bounds. The manual and weekly `stress` workflow retains these
end-to-end receipt, peer, controller, controller-admission, and runtime-frontier proofs. Run them
locally in release mode rather than making every local and pull-request test pay their cost:

```sh
cargo test --release \
  -p milkdrift-redb-store \
  --test application_state \
  --all-features \
  release_receipt_longevity_crosses_many_hot_bounds_and_replays_after_restart \
  -- --ignored --exact --nocapture
cargo test --release \
  -p milkdrift-daemon \
  --test two_daemon_peer \
  peer_execution_retention_longevity_survives_turnover_and_restart \
  -- --ignored --exact --nocapture
cargo test --release \
  -p milkdrift-control \
  --test control_service \
  revision_and_lifecycle::release_controller_longevity_stops_once_across_checkpoints_and_restart \
  -- --ignored --exact --nocapture
cargo test --release \
  -p milkdrift-control \
  --test control_service \
  admission::release_controller_admission_longevity_turns_over_reservations_artifacts_and_restart \
  -- --ignored --exact --nocapture
cargo test --release \
  -p milkdrift-runtime \
  --all-features \
  --test structured_runtime \
  lifecycle::historical_execution_frontier_stays_bounded_across_index_limit \
  -- --ignored --exact --nocapture
```

Do not mark a hanging or nondeterministic test ignored; correct the lifecycle or coordination
defect instead.

### 2026-08-25 local measurement

With Rust 1.95.0 on the available development machine, the closure all-feature workspace
suite passed in 54.427 seconds and the normal debug `projection::tests::bounded::` group
passed in 27.190 seconds. Seven representative 10,000-cycle cases passed in release mode
in 42.090 seconds, including a 39.32-second release rebuild; test execution was 2.72
seconds. The ignored release redb frontier case passed in 0.543 seconds after that build. The 10,000-transition
projection invariants therefore remain normal non-ignored CI tests, while the durable
cross-index boundary case remains a weekly/manual release stress test because it is
purposefully excluded from the ordinary workspace suite.

These are one-machine observations, not universal timing guarantees. `/usr/bin/time -v`
was not installed, so Bash wall/user/system timings were saved and peak resident memory
was unavailable. The measured commands were the all-feature workspace test command and
the debug/release bounded and ignored release stress commands shown above.
