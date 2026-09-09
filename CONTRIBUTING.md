# Contributing to Milkdrift

Start with [AGENTS.md](AGENTS.md) and its reading order. Milkdrift's headless applications already
run through a durable workflow kernel; [status](docs/product/status.md) explains current behavior
and limitations, and the [roadmap](docs/product/roadmap.md) identifies the remaining work and scope
freeze. Use [the implementation reading path](docs/README.md#learning-the-implementation) to trace
an operator action to its source and independent tests.

Keep each fact with its [architectural owner](docs/architecture.md). Follow the
[development practices](docs/development/practices/README.md) relevant to the work,
and choose checks using the [verification policy](docs/development/workflow.md#choose-verification-for-the-change).
Executable changes require the full gate; documentation-only changes have their own checks.
Tests should establish observable invariants, and schema changes need explicit compatibility
review. Write an ADR when a decision changes a durable boundary. Product-authored Rust remains
safe Rust; dependency changes need a concrete benefit and the required dependency audits.

Contributions are licensed under `MIT OR Apache-2.0`.
