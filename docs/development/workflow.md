# Development workflow

This document owns build, test, lint, fixture, and focused verification commands.
Use the exact toolchain in [rust-toolchain.toml](../../rust-toolchain.toml).
[Engineering rules](engineering-rules.md) owns implementation policy.

## Full local gate

Run from the repository root. Build the product daemon before isolated application evidence tests
and the shared process fixture before isolated daemon integration tests:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-local-process --bin milkdrift-process-test-helper
```

The workspace gate also builds these targets. Hermetic external-evidence tests require Python 3
and Git on the harness's `PATH`; generated verifiers use the resolved absolute Git executable
because process adapters deliberately clear the child environment.

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
cargo test -p milkdrift-evidence --test repository_contracts --all-features
```

In PowerShell set `$env:RUSTDOCFLAGS = '-D warnings'` before `cargo doc` and restore its previous
value afterward. Use `cargo fmt --all` to apply formatting. Dependency changes require the full
deny/machete/duplicate-tree audit; pinned CI tool versions are in
[quality.yml](../../.github/workflows/quality.yml).

## Focused suites

Select the owning boundary while iterating; a focused pass does not replace the full gate.

```sh
cargo test -p milkdrift-blueprint --test kernel --all-features
cargo test -p milkdrift-authority --all-features
cargo test -p milkdrift-control-protocol -p milkdrift-control-client --all-features
cargo test -p milkdrift-cli -p milkdrift-prompt-sequence --all-features
cargo test -p milkdrift-daemon --test control_plane --all-features
cargo test -p milkdrift-runtime --test durable_runtime --all-features
cargo test -p milkdrift-runtime --test structured_runtime --all-features
cargo test -p milkdrift-runtime --test causal_context --all-features
cargo test -p milkdrift-model --test contracts --all-features
cargo test -p milkdrift-model-provider --test mock_endpoints --all-features
cargo test -p milkdrift-control --all-features
cargo test -p milkdrift-persistence --all-features
cargo test -p milkdrift-redb-store --test contracts --all-features
cargo test -p milkdrift-redb-store --test application_state --all-features
cargo test -p milkdrift-peer-protocol --test protocol --all-features
cargo test -p milkdrift-peer-http --test peer_service --all-features
cargo test -p milkdrift-daemon --test two_daemon_peer --all-features
```

Tests use temporary stores, ephemeral loopback listeners, fixed identities/clocks, controlled
adapters, and private fixture credentials. Model mocks contact no provider. Local-process and
daemon integration suites share the byte-pinned Rust process helper. See
[status](../product/status.md) for executed platform evidence. Native source checks
or cross-compilation alone cannot establish runtime portability or filesystem durability.

Shared production-adapter conformance and host lifecycle checks:

```sh
cargo test -p milkdrift-capability-host --all-features
cargo test -p milkdrift-local-process --test process_execution --all-features
cargo test -p milkdrift-model-provider --test mock_endpoints --all-features model_endpoint_adapter_passes_shared_conformance -- --exact
cargo test -p milkdrift-control --test control_service --all-features workflow_control_adapter_passes_shared_conformance -- --exact
cargo test -p milkdrift-peer-http --lib --all-features remote::tests::remote_capability_adapter_passes_shared_conformance -- --exact
```

After Unix process-tree tests, `pgrep -af milkdrift-process-test-helper` can reveal surviving
helpers; exclude a match belonging to the inspection command itself. Fix lifecycle/coordination
defects instead of marking hanging or nondeterministic tests ignored.

## Fixtures and documentation contracts

Golden JSON lives under each owner's `tests/fixtures`. Change implementation and hand-reviewed
fixture together, run exact canonical re-encoding and refusal tests, and record reader-compatibility
changes in an ADR. Never regenerate fixtures merely to satisfy an assertion.

Maintained operator examples live under [examples](../../examples/operator/README.md), not fixtures.
Repository contracts check every maintained example through its production reader, canonical
documents and local links, source-derived versions, obsolete history paths, source size/cohesion,
dependency direction, and public re-export boundaries. CLI documentation command parsing runs in
the CLI test target; [actual-binary evidence](verification-evidence.md#actual-binary-scenarios)
checks executable composition, not just syntax.

Public API review follows [the API policy](../reference/public-api-policy.md): generate default
and all-feature inventories under `target/public-api`, trace actual consumers, and review
test/evidence features separately. Nightly rustdoc JSON tooling does not change the product's
pinned stable toolchain.

## Storage-boundary stress

Ordinary projection bounded-state checks remain non-ignored:

```sh
cargo test -p milkdrift-runtime --all-features projection::tests::bounded:: -- --nocapture
```

The five expensive persistence/operational cases are release-only manual/weekly evidence:

```sh
cargo test --release -p milkdrift-redb-store --test application_state --all-features release_receipt_longevity_crosses_many_hot_bounds_and_replays_after_restart -- --ignored --exact --nocapture
cargo test --release -p milkdrift-daemon --test two_daemon_peer --all-features peer_execution_retention_longevity_survives_turnover_and_restart -- --ignored --exact --nocapture
cargo test --release -p milkdrift-control --test control_service --all-features revision_and_lifecycle::release_controller_longevity_stops_once_across_checkpoints_and_restart -- --ignored --exact --nocapture
cargo test --release -p milkdrift-control --test control_service --all-features admission::release_controller_admission_longevity_turns_over_reservations_artifacts_and_restart -- --ignored --exact --nocapture
cargo test --release -p milkdrift-runtime --test structured_runtime --all-features lifecycle::historical_execution_frontier_stays_bounded_across_index_limit -- --ignored --exact --nocapture
```

[stress.yml](../../.github/workflows/stress.yml) also owns the exact selected 10,000-cycle projection
cases. Keep filters synchronized with test discovery; run reports and timings belong in CI
artifacts or ignored `target/` paths. [Verification evidence](verification-evidence.md) owns
mutation, benchmark, operational, and external qualification lanes.
