# Milkdrift

Milkdrift is a local-first foundation for durable, live-editable workflows whose tasks can be satisfied by explicitly constrained capabilities: hosted AI providers, local servers, coding agents, tools, humans, or peer machines. Its semantic core keeps workflow meaning independent of any executor, UI, database, network, or provider.

This first rebirth pass implements two pure Rust domains. `milkdrift-capability` defines bounded, versioned capability descriptions, requirements, invocation contracts, observations, and canonical JSON. `milkdrift-blueprint` defines immutable workflow revisions, the semantic graph, a closed atomic mutation model, deterministic content identity, and validation. There is no runtime, persistence, provider adapter, daemon, networking, or desktop application yet.

```sh
cargo test --workspace
```

A minimal revision is constructed through a validated mutation batch; see the crate-level example in `milkdrift-blueprint` and the integration tests under `crates/blueprint/tests`.

## Repository map

- `crates/capability`: provider-neutral capability and invocation contracts.
- `crates/blueprint`: immutable workflow definitions and revision transactions.
- `docs`: status, roadmap, development commands, and durable decisions.
- `.github/workflows/quality.yml`: the single repository quality workflow.

The canonical documents are [VISION.md](VISION.md), [ARCHITECTURE.md](ARCHITECTURE.md), [docs/STATUS.md](docs/STATUS.md), [docs/ROADMAP.md](docs/ROADMAP.md), [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md), and [the ADR index](docs/decisions/README.md).

Milkdrift is licensed under either the [MIT license](LICENSE-MIT) or the [Apache License 2.0](LICENSE-APACHE), at your option.
