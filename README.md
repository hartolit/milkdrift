# Milkdrift

Milkdrift is a local-first foundation for durable, live-editable workflows whose tasks can be satisfied by explicitly constrained capabilities: hosted AI providers, local servers, coding agents, tools, humans, or peer machines. Its semantic core keeps workflow meaning independent of any executor, UI, database, network, or provider.

Milkdrift currently has a headless Rust execution center. It stores immutable workflow revisions, authorizes versioned idempotent run commands against exact scoped grant revisions, freezes the accepted actor/grant/policy basis into each started run, and records canonical decisions at capability resolution, exact-generation claim, and final adapter entry. It rebuilds pure projections, schedules bounded work through exact authorized capability snapshots, keeps branch-local workspace values, publishes content-addressed artifacts, recovers local runs after restart, and applies compatible revision changes prospectively through persisted reconciliation plans. Its workflow-control application layer accepts bounded digest-bound proposals from humans, services, processes, or models; creates immutable prospective revisions; classifies risk; and uses the same authorized runtime reconciliation path for approval and apply. Typed controller-policy, durable-assessment, and checkpoint contracts exist for focused library validation, but the production daemon deliberately does not admit continuous controllers until their cumulative resource ceilings are enforced at the final external-entry boundary.

The production local backend uses redb plus a filesystem artifact directory. `milkdrift-daemon` is the single durable owner: it validates versioned host configuration, authenticates local clients and configured peers, recovers the runtime with admission closed, registers the generation-safe process/model/workflow-control/remote-peer capability host, runs bounded effect workers, and serves separate versioned control and peer authentication realms. A dedicated bounded owner thread keeps synchronous redb/runtime work off the async HTTP reactor. `milkdrift-cli` and the reusable control client use only that API; they never open storage or resolve adapter secrets. The desktop UI is not implemented.

```sh
cargo test --workspace
```

Repeatable mutation, benchmark, storage-growth, daemon-saturation, and cross-platform evidence is
documented in [verification and operational evidence](docs/development/verification-evidence.md). These lanes
produce reviewable artifacts without treating benchmark values as correctness gates.

## Two-daemon peer example

Peer support is disabled unless `peers.mode = "enabled"` supplies one local identity and explicit
relationships. For a local test, daemon B can point at daemon A with this TOML fragment:

```toml
[peers]
mode = "enabled"
local_peer_id = "peer-b"

[peers.serving]
worker_threads = 4
maximum_global_active = 256
maximum_dispatch_queue = 256
maximum_hot_terminal_records = 10000
archive_batch_size = 256
observation_hot_retention_ms = 86400000
recovery_page = 128
poll_interval_ms = 100

[[peers.relationships]]
peer_id = "peer-a"
endpoint = "http://127.0.0.1:9734/"
credential_ref = "credential:peer-a"
insecure_loopback_development = true
actions = ["read_catalog", "invoke", "cancel"]
capability_allow = ["my-process-capability"]
capability_deny = []
operation_allow = ["process.execute"]
maximum_side_effect = "read_only"
execution_filesystem = [{ root = "/opt/milkdrift-tools/my-process", access = ["execute"] }]
execution_network_profiles = []
execution_network_destinations = []
execution_secrets = []
maximum_concurrent = 2
maximum_requests_per_minute = 600
maximum_artifact_bytes = 1048576
artifact_sensitivities = []
maximum_duration_ms = 30000
maximum_observations = 128
trust_zone = "operator-wireguard"
delegation_ref = "delegation:peer-a-b"
expires_at_unix_ms = 1798761600000
```

The peer credential remains in `secret_sources`, is resolved at each request, and is never printed. Capability/operation allowlists do not grant host resources: `execution_filesystem`, network, and secret scopes must explicitly contain the selected adapter's declared requirements. Configure the inverse relationship on daemon A, start both daemons, then run:

Filesystem authority uses canonical durable roots: `/opt/...` on Unix or an uppercase drive form
such as `C:/tools/...` on Windows. Windows configuration still uses `/` in this authority field;
native adapter paths are canonicalized before conversion. UNC/device, drive-relative, mixed
separator, traversal, and alternate-data-stream forms are refused.

```sh
milkdrift peer list
milkdrift --yes peer connect peer-a
milkdrift capability list
milkdrift peer show peer-a
```

The insecure mode refuses non-loopback URLs. Use ordinary HTTPS directly or terminate TLS in an operator-controlled reverse proxy; WireGuard and Tailscale are possible connectivity choices, not Milkdrift dependencies. See `docs/operations/peers.md` and `docs/reference/peer-protocol.md`.

## Local daemon quick start

Create a version-nine TOML daemon configuration. Relative paths are resolved from the configuration
file directory. Presets are deterministic shorthand for exact operation sets only; the required
`authority` table supplies every executable resource scope, ceiling, and validity boundary. The
checked fixture is a complete minimal starting point:

```sh
cp apps/daemon/tests/fixtures/daemon-config-v9.toml daemon.toml
export MILKDRIFT_OPERATOR_TOKEN='replace-with-a-long-random-local-token'
cargo run -p milkdrift-daemon --bin milkdrift-daemon -- --config daemon.toml --check-config
cargo run -p milkdrift-daemon --bin milkdrift-daemon -- --config daemon.toml --print-effective-config
cargo run -p milkdrift-daemon --bin milkdrift-daemon -- --config daemon.toml
```

In another terminal:

```sh
export MILKDRIFT_TOKEN='replace-with-the-same-long-random-local-token'
cargo run -p milkdrift-cli -- daemon readiness
cargo run -p milkdrift-cli -- --json capability list
cargo run -p milkdrift-cli -- --command-id quickstart-blueprint-import-v1 \
  blueprint import crates/blueprint/tests/fixtures/revision-v2.json
```

With exact coding, verification, and reviewer capability profiles registered and included in the
actor's scoped grant, the same headless client can import and run an ordered Markdown implementation
sequence:

```sh
cargo run -p milkdrift-cli -- --command-id quickstart-sequence-validate-v1 \
  sequence validate examples/headless-dogfood-sequence.md
cargo run -p milkdrift-cli -- --command-id quickstart-sequence-import-v1 \
  sequence import examples/headless-dogfood-sequence.md
cargo run -p milkdrift-cli -- --command-id quickstart-run-start-v1 \
  --expected-revision REVISION_ID \
  run start RUN_ID milkdrift-core-convergence REVISION_ID
cargo run -p milkdrift-cli -- run timeline RUN_ID --follow
```

Successful verification advances to the next fresh coding-agent process while one explicitly
authorized repository remains persistent. Failure routes to independent review and a durable
shared-control approval hold; remediation is a normal prospective revision. See [the headless
dogfood guide](docs/guides/headless-dogfood.md), [schema-2 reference](docs/reference/prompt-sequence-v2.md),
and [complete example](examples/headless-dogfood-sequence.md).

To generate redacted operator evidence against one real coding-agent executable and one real model
endpoint, run `cargo external-evidence` with the safe templates under
[`examples/external-evidence`](examples/external-evidence/README.md). The exact
prerequisites, qualification rules, costs, restart/failure scenario, report schema, fixture mode,
and cleanup guidance are in [the external-evidence guide](docs/guides/external-evidence.md). Fixture mode
tests the harness but never qualifies as external interoperability proof.

The daemon refuses non-loopback plaintext binds and permissive CORS is not enabled. Older configuration/storage schemas and legacy sidecar authority are not silently migrated, and broad/unbounded authority requires an explicit dangerous acknowledgement. Empty artifact, layout, peer, and workspace scopes deny access. See [the daemon operation guide](docs/operations/daemon.md), [the authority configuration guide](docs/operations/authority.md), and [the control API reference](docs/reference/control-api.md).

A minimal revision is constructed through a validated mutation batch; see the crate-level example in `milkdrift-blueprint` and the integration tests under `crates/blueprint/tests`.

## Repository map

- `crates/capability`: provider-neutral capability, exact resolution, and invocation contracts.
- `crates/authority`: actor identity, scoped immutable grants, deterministic decisions, and opaque secret references.
- `crates/control`: shared human/service/AI workflow proposals, risk policy, authority presets, the typed controller lifecycle/read model, and the in-process workflow-control capability adapter.
- `crates/control-protocol`: pure protocol-2.2 commands, read models, envelopes, authenticated cursors, streams, and layout schema 1.
- `crates/control-client`: authenticated typed HTTP queries, exact command submission, bounded artifact ranges, and resumable SSE.
- `crates/prompt-sequence`: bounded JSON/Markdown implementation sequences, ordinary blueprint compilation, and prospective remediation proposal construction.
- `crates/capability-host`: live adapter generations, resolution, admission, cancellation, health, drain, and shutdown.
- `crates/blueprint`: immutable workflow definitions, fingerprints, and revision transactions.
- `crates/model`: provider-neutral model task/response and exact schema-v2 causal-context manifest contracts.
- `crates/peer-protocol`: bounded transport-neutral peer session, catalog, execution, cancellation, observation, and artifact-transfer contracts.
- `crates/workspace`: scoped immutable values, branch lineage, artifact metadata, and budgets.
- `crates/persistence`: versioned events and narrow journal/revision/snapshot/workspace/artifact ports.
- `crates/runtime`: commands, pure projections, scheduling, execution ownership, recovery, reconciliation, and authoritative causal-context discovery/materialization.
- `adapters/redb-store`: transactional local redb storage and content-addressed artifact bytes.
- `adapters/local-process`: byte-pinned schema-v2 safe-argv profiles and the trusted-host process adapter; it is not a sandbox.
- `adapters/model-provider`: bounded HTTP endpoint profiles plus OpenAI-compatible and native Anthropic mappings.
- `adapters/peer-http`: authenticated HTTP peer transport, durable serving/reconnect, and remote capabilities mapped into the ordinary capability host.
- `adapters/local-secret`: explicit opaque-secret-reference resolution from bounded environment or restricted-file sources.
- `apps/daemon`: authoritative local host, bounded runtime owner, authentication, HTTP/SSE API, recovery, and shutdown.
- `apps/cli`: comprehensive storage-free operator client with human output, stable schema-v1
  success/failure JSON, and resumable JSON Lines streams.
- `tools/evidence`: development-only Divan and operational fixtures for critical bounded paths.
- `.github/workflows`: pinned Linux quality/stress/evidence lanes plus Linux, Windows, and macOS contract validation.
- `docs`: product, architecture, development, operator, reference, and durable decision documentation.
- `.github/workflows/quality.yml`: the primary format/check/test/lint/documentation workflow.
- `.github/workflows/stress.yml`: weekly and manually triggered long-run storage/projection boundary evidence.

Start with [AGENTS.md](AGENTS.md). The [documentation index](docs/README.md) links the canonical detailed documents, including the [product vision](docs/product/vision.md), [architecture constitution](docs/architecture.md), [current status](docs/product/status.md), [roadmap](docs/product/roadmap.md), [development workflow](docs/development/workflow.md), [public API policy](docs/reference/public-api-policy.md), and [ADR index](docs/decisions/README.md).

Milkdrift is licensed under either the [MIT license](LICENSE-MIT) or the [Apache License 2.0](LICENSE-APACHE), at your option.
