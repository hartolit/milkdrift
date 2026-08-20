# Development

This document owns local build, format, test, lint, dependency, and fixture-update commands.

Use Rust 1.95.0 as declared by `rust-toolchain.toml` and the workspace
`rust-version`. CI uses the same exact toolchain:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps
cargo deny check
```

Apply formatting with `cargo fmt --all`. Golden JSON is hand-reviewed compatibility data under each crate's `tests/fixtures` directory. To update a fixture, change the schema implementation and fixture in one review, run its exact canonical re-encoding test, and record any compatibility decision that changes reader behavior in an ADR. Never regenerate fixtures merely to make a failing test disappear.

Dependencies must be stable crates.io releases with a concrete use in current code. Run `cargo deny check` after any lockfile change. Git dependencies require a documented necessity.

Keep module ownership visible. A source file that accumulates multiple lifecycle or
domain responsibilities must be split into real Rust child modules; `include!` and
facade-wide wildcard re-exports are not substitutes for module boundaries. Production
files approaching 2,000 lines require an explicit cohesion review, and integration
tests should be grouped by behavior with shared support kept separate.
