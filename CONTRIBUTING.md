# Contributing to Milkdrift

Milkdrift is rebuilding around a small, pure semantic kernel. Changes should preserve the dependency direction and the ownership rules in [the architecture constitution](docs/architecture.md).

Before submitting a change, follow the [development workflow](docs/development/workflow.md) and
[engineering rules](docs/development/engineering-rules.md). Add tests that establish observable
invariants, keep serialized schema changes explicit, and write an ADR only when a decision changes
a durable project boundary. Product-authored Rust remains safe Rust. Dependencies need a concrete
risk or maintenance benefit and must pass `cargo deny check`.

Contributions are licensed under `MIT OR Apache-2.0`.
