# Milkdrift

Milkdrift is a local-first foundation for durable, live-editable workflows whose tasks can be satisfied by explicitly constrained capabilities: hosted AI providers, local servers, coding agents, tools, humans, or peer machines. Its semantic core keeps workflow meaning independent of any executor, UI, database, network, or provider.

Milkdrift currently has a headless Rust execution center. It stores immutable workflow revisions, accepts versioned idempotent run commands, records checksummed append-only events, rebuilds pure projections, schedules bounded work through exact capability snapshots, keeps branch-local workspace values, publishes content-addressed artifacts, recovers local runs after restart, and applies compatible revision changes prospectively through persisted reconciliation plans.

The production local backend uses redb plus a filesystem artifact directory. Capability execution is deliberately a narrow port with a deterministic executor for tests; real registries, provider/process adapters, causal context construction, secrets/authority mediation, peers, daemon APIs, CLI, and desktop UI remain outside this pass.

```sh
cargo test --workspace
```

A minimal revision is constructed through a validated mutation batch; see the crate-level example in `milkdrift-blueprint` and the integration tests under `crates/blueprint/tests`.

## Repository map

- `crates/capability`: provider-neutral capability, exact resolution, and invocation contracts.
- `crates/blueprint`: immutable workflow definitions, fingerprints, and revision transactions.
- `crates/workspace`: scoped immutable values, branch lineage, artifact metadata, and budgets.
- `crates/persistence`: versioned events and narrow journal/revision/snapshot/workspace/artifact ports.
- `crates/runtime`: commands, pure projections, scheduling, execution ownership, recovery, and reconciliation.
- `adapters/redb-store`: transactional local redb storage and content-addressed artifact bytes.
- `docs`: status, roadmap, development commands, and durable decisions.
- `.github/workflows/quality.yml`: the primary format/check/test/lint/documentation workflow.
- `.github/workflows/stress.yml`: weekly and manually triggered long-run storage/projection boundary evidence.

The canonical documents are [VISION.md](VISION.md), [ARCHITECTURE.md](ARCHITECTURE.md), [docs/STATUS.md](docs/STATUS.md), [docs/ROADMAP.md](docs/ROADMAP.md), [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md), and [the ADR index](docs/decisions/README.md).

Milkdrift is licensed under either the [MIT license](LICENSE-MIT) or the [Apache License 2.0](LICENSE-APACHE), at your option.
