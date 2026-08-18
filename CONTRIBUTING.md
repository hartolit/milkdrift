# Contributing to Milkdrift

Milkdrift is rebuilding around a small, pure semantic kernel. Changes should preserve the dependency direction and the ownership rules in [ARCHITECTURE.md](ARCHITECTURE.md).

Before submitting a change, run every command in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md). Add tests that establish observable invariants, keep serialized schema changes explicit, and write an ADR only when a decision changes a durable project boundary. Product-authored Rust remains safe Rust. Dependencies need a concrete risk or maintenance benefit and must pass `cargo deny check`.

Contributions are licensed under `MIT OR Apache-2.0`.
