# Milkdrift

Milkdrift is a local-first foundation for durable, live-editable workflows whose tasks can be satisfied by explicitly constrained capabilities: hosted AI providers, local servers, coding agents, tools, humans, or peer machines. Its semantic core keeps workflow meaning independent of any executor, UI, database, network, or provider.

Milkdrift currently has a headless Rust execution center. It stores immutable workflow revisions, authorizes versioned idempotent run commands against exact scoped grant revisions, records the decision atomically with each external command result, rebuilds pure projections, schedules bounded work through exact capability snapshots, keeps branch-local workspace values, publishes content-addressed artifacts, recovers local runs after restart, and applies compatible revision changes prospectively through persisted reconciliation plans.

The production local backend uses redb plus a filesystem artifact directory. A generation-safe live capability host owns deterministic policy-constrained resolution, exact dispatch/cancellation routing, and an explicit bounded effect-worker lifecycle. Its concrete capabilities include a safe-argv local-process adapter and a model-endpoint adapter with independently mapped OpenAI-compatible and Anthropic protocols. Immutable task policies produce deterministic, budgeted causal-context manifests; each exact manifest is persisted and bound to the invocation before provider entry. Endpoint profiles keep model identity, HTTPS/loopback policy, limits, features, and opaque secret references outside workflow semantics. Model responses, final text, structured output, tool calls, and bounded provider metadata are committed artifacts. Milkdrift invokes configured endpoints—it does not load, manage, or infer model architectures. Authentication, AI workflow control, peers, daemon APIs, CLI, and desktop UI remain outside this pass.

```sh
cargo test --workspace
```

A minimal revision is constructed through a validated mutation batch; see the crate-level example in `milkdrift-blueprint` and the integration tests under `crates/blueprint/tests`.

## Repository map

- `crates/capability`: provider-neutral capability, exact resolution, and invocation contracts.
- `crates/authority`: actor identity, scoped immutable grants, deterministic decisions, and opaque secret references.
- `crates/capability-host`: live adapter generations, resolution, admission, cancellation, health, drain, and shutdown.
- `crates/blueprint`: immutable workflow definitions, fingerprints, and revision transactions.
- `crates/model`: provider-neutral model task/response and exact causal-context manifest contracts.
- `crates/workspace`: scoped immutable values, branch lineage, artifact metadata, and budgets.
- `crates/persistence`: versioned events and narrow journal/revision/snapshot/workspace/artifact ports.
- `crates/runtime`: commands, pure projections, scheduling, execution ownership, recovery, and reconciliation.
- `adapters/redb-store`: transactional local redb storage and content-addressed artifact bytes.
- `adapters/local-process`: versioned safe-argv profiles and the production local process adapter.
- `adapters/model-provider`: bounded HTTP endpoint profiles plus OpenAI-compatible and native Anthropic mappings.
- `adapters/secret-env`: explicit opaque-secret-reference to environment-name resolution.
- `docs`: status, roadmap, development commands, and durable decisions.
- `.github/workflows/quality.yml`: the primary format/check/test/lint/documentation workflow.
- `.github/workflows/stress.yml`: weekly and manually triggered long-run storage/projection boundary evidence.

The canonical documents are [VISION.md](VISION.md), [ARCHITECTURE.md](ARCHITECTURE.md), [docs/STATUS.md](docs/STATUS.md), [docs/ROADMAP.md](docs/ROADMAP.md), [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md), and [the ADR index](docs/decisions/README.md).

Milkdrift is licensed under either the [MIT license](LICENSE-MIT) or the [Apache License 2.0](LICENSE-APACHE), at your option.
