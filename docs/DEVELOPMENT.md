# Development

This document owns local build, format, test, lint, dependency, and fixture-update commands.

Use Rust 1.95.0 as declared by `rust-toolchain.toml` and the workspace
`rust-version`. CI uses the same exact toolchain:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
cargo deny check
```

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

With Rust 1.95.0 on the available development machine, the all-feature workspace suite
passed in 41.414 seconds and the normal debug `projection::tests::bounded::` group passed
in 24.877 seconds. The same bounded group passed in release mode in 31.070 seconds,
including a 28.62-second release rebuild; its test execution was 2.42 seconds. The ignored
release redb frontier case passed in 0.483 seconds after that build. The 10,000-transition
projection invariants therefore remain normal non-ignored CI tests, while the durable
cross-index boundary case remains a weekly/manual release stress test because it is
purposefully excluded from the ordinary workspace suite.

These are one-machine observations, not universal timing guarantees. `/usr/bin/time -v`
was not installed, so Bash wall/user/system timings were saved and peak resident memory
was unavailable. The measured commands were the all-feature workspace test command and
the debug/release bounded and ignored release stress commands shown above.
