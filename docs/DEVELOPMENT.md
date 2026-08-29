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
cargo tree --workspace --duplicates
```

Focused daemon/control-plane checks are:

```sh
cargo test -p milkdrift-control-protocol --all-features -- --nocapture
cargo test -p milkdrift-control-client --all-features -- --nocapture
cargo test -p milkdrift-daemon --test control_plane daemon_auth_startup_readiness_and_authority -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane scoped_read_matrix_and_continuations_fail_closed -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane daemon_command_idempotency_restart_and_stale_conflict -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane daemon_bounded_overload_returns_stable_error -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane daemon_stream_reconnect_auth_rotation_and_shutdown -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane daemon_graceful_shutdown_and_restart -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane daemon_configured_process_adapter_executes_to_terminal -- --exact --nocapture
cargo test -p milkdrift-daemon --test control_plane daemon_startup_corruption_refuses_command_admission -- --exact --nocapture
cargo test -p milkdrift-cli --all-features -- --nocapture
```

Focused peer protocol, durability, artifact, reconnect, and real two-daemon checks are:

```sh
cargo test -p milkdrift-peer-protocol --all-features --test protocol -- --nocapture
cargo test -p milkdrift-peer-http --all-features --test peer_service -- --nocapture
cargo test -p milkdrift-daemon --all-features --test two_daemon_peer -- --nocapture
```

They use temporary stores, ephemeral loopback listeners, local bearer files, and a fake capability adapter. They require no internet, VPN, provider credential, or manual peer service. Coverage includes incompatible major negotiation, hostile bounds/duplicates, HTTPS/loopback safety, authenticated identity cross-checking and live revocation, authority-filtered catalogs, request-rate/concurrency/terminal-reserved observation bounds, generation-safe remote registration, durable acceptance replay/conflict, acceptance response loss, observation-append failure, contiguous resume cursors, restart uncertainty after adapter-entry intent, terminal cancellation evidence, verified/deduplicated/interrupted/mismatched artifact transfer, artifact-publication recovery, path/content/quota rejection, auth-realm separation, request-time peer credential rotation, disconnect, and graceful two-daemon shutdown.

These tests use temporary redb/artifact roots and ephemeral loopback listeners only. They cover fail-closed configuration/authentication, server-owned actor mapping, workflow/run/revision/proposal/layout/artifact/capability/provider/health scope, protected metadata/content audit, cross-actor cursor rejection, grant-narrowing page and stream failure, queue overload, durable command replay across restart, stale guards, credential rotation, startup corruption, and ordered shutdown. They require no internet or real credential. Run the daemon manually with `cargo run -p milkdrift-daemon -- --config PATH`; run the client with `cargo run -p milkdrift-cli -- [GLOBAL OPTIONS] COMMAND`. Keep bearer values in a private file or a referenced environment variable, never a command argument.

Focused Pass-03C checks are:

```sh
cargo test -p milkdrift-blueprint --test kernel -- --nocapture
cargo test -p milkdrift-model --test contracts -- --nocapture
cargo test -p milkdrift-runtime --test causal_context -- --nocapture
cargo test -p milkdrift-model-provider --test mock_endpoints -- --nocapture
cargo test -p milkdrift-runtime --test structured_runtime causal_context_production -- --nocapture
cargo test -p milkdrift-capability-host --test materialization -- --nocapture
```

Focused Pass-03D workflow-control checks are:

```sh
cargo test -p milkdrift-control --test proposal_contracts -- --nocapture
cargo test -p milkdrift-control --test authority_policy_controller -- --nocapture
cargo test -p milkdrift-control --test control_service -- --nocapture
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
the following audit after any dependency or lockfile change; `cargo machete` is optional
local tooling and CI does not assume it is installed:

```sh
cargo tree --workspace --duplicates
cargo machete
cargo deny check
```

Git dependencies require a documented necessity.

Keep module ownership visible. A source file that accumulates multiple lifecycle or
domain responsibilities must be split into real Rust child modules; `include!` and
facade-wide wildcard re-exports are not substitutes for module boundaries. Production
files approaching 2,000 lines require an explicit cohesion review, and integration
tests should be grouped by behavior with shared support kept separate.

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

The ordinary workspace test suite excludes tests whose fixture intentionally crosses a
large persistence bound and therefore performs thousands of durable redb index
mutations. The manual and weekly `stress` workflow retains this end-to-end evidence. Run
it locally in release mode rather than making every local and pull-request test pay its
cost:

```sh
cargo test --release \
  -p milkdrift-runtime \
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
