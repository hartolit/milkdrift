# Milkdrift

Milkdrift is a local-first foundation for durable, live-editable workflows whose tasks can be satisfied by explicitly constrained capabilities: hosted AI providers, local servers, coding agents, tools, humans, or peer machines. Its semantic core keeps workflow meaning independent of any executor, UI, database, network, or provider.

Milkdrift currently has a headless Rust execution center. It stores immutable workflow revisions, authorizes versioned idempotent run commands against exact scoped grant revisions, freezes the accepted actor/grant/policy basis into each started run, and records canonical decisions at capability resolution, exact-generation claim, and final adapter entry. It rebuilds pure projections, schedules bounded work through exact authorized capability snapshots, keeps branch-local workspace values, publishes content-addressed artifacts, recovers local runs after restart, and applies compatible revision changes prospectively through persisted reconciliation plans. Its workflow-control application layer accepts bounded digest-bound proposals from humans, services, processes, or models; creates immutable prospective revisions; classifies risk; and uses the same authorized runtime reconciliation path for approval and apply.

The production local backend uses redb plus a filesystem artifact directory. `milkdrift-daemon` is the single durable owner: it validates versioned host configuration, authenticates local clients and configured peers, recovers the runtime with admission closed, registers the generation-safe process/model/workflow-control/remote-peer capability host, runs bounded effect workers, and serves separate versioned control and peer authentication realms. A dedicated bounded owner thread keeps synchronous redb/runtime work off the async HTTP reactor. `milkdrift-cli` and the reusable control client use only that API; they never open storage or resolve adapter secrets. The desktop UI is not implemented.

```sh
cargo test --workspace
```

## Two-daemon peer example

Peer support is disabled unless `peers.enabled` and explicit relationships are configured. For a local test, daemon B can point at daemon A with:

```json
"peers": {
  "enabled": true,
  "local_peer_id": "peer-b",
  "relationships": [{
    "peer_id": "peer-a",
    "endpoint": "http://127.0.0.1:9734/",
    "credential_ref": "credential:peer-a",
    "insecure_loopback_development": true,
    "actions": ["read_catalog", "invoke", "cancel"],
    "capability_allow": ["my-process-capability"],
    "capability_deny": [],
    "operation_allow": ["process.execute"],
    "maximum_side_effect": "read_only",
    "maximum_concurrent": 2,
    "maximum_requests_per_minute": 600,
    "maximum_artifact_bytes": 1048576,
    "maximum_duration_ms": 30000,
    "maximum_observations": 128,
    "trust_zone": "operator-wireguard",
    "delegation_ref": "delegation:peer-a-b",
    "expires_at_unix_ms": 1798761600000
  }]
}
```

The peer credential remains in `secret_sources`, is resolved at each request, and is never printed. Configure the inverse relationship on daemon A, start both daemons, then run:

```sh
milkdrift peer list
milkdrift --yes peer connect peer-a
milkdrift capability list
milkdrift peer show peer-a
```

The insecure mode refuses non-loopback URLs. Use ordinary HTTPS directly or terminate TLS in an operator-controlled reverse proxy; WireGuard and Tailscale are possible connectivity choices, not Milkdrift dependencies. See `docs/operator-peers.md` and `docs/reference/peer-protocol.md`.

## Local daemon quick start

Create a private bearer-token file and a version-three daemon configuration. Relative paths are resolved from the configuration file directory. Presets are deterministic shorthand for exact operation sets only; the required `authority` object supplies every executable resource scope, ceiling, and validity boundary.

```sh
install -m 600 /dev/null operator.token
printf '%s' 'replace-with-a-long-random-local-token' > operator.token
cat > daemon.json <<'JSON'
{
  "schema_version": 3,
  "data_root": "./milkdrift-data",
  "bind": "127.0.0.1:9734",
  "secret_sources": {
    "credential:operator": { "type": "file", "path": "./operator.token" }
  },
  "actors": [{
    "credential_ref": "credential:operator",
    "actor": "human:operator",
    "grant_id": "grant:operator",
    "grant_revision": 1,
    "revocation_generation": 0,
    "preset": "controller",
    "authority": {
      "resources": {
        "workflow_run": { "type": "workflow", "workflow": "example-workflow" },
        "capability": {
          "deny_all": false,
          "identities": ["local-example"],
          "categories": [],
          "operations": ["process.execute"],
          "provider_profiles": [],
          "trust_zones": ["local-process"],
          "localities": ["local"],
          "peers": [],
          "maximum_side_effect": "read_only"
        },
        "filesystem": [],
        "network": { "profiles": [], "destinations": [] },
        "secrets": [],
        "artifacts": { "identities": [], "sensitivities": [] },
        "layouts": { "revisions": [], "actors": [], "shared": false },
        "peers": { "identities": [], "allow_any": false },
        "daemon": {
          "readiness": true,
          "detailed_health": false,
          "own_authority": true,
          "configuration": false,
          "audit": false
        },
        "workspace": { "scopes": [], "allow_any_in_run": false }
      },
      "budget": {
        "cost_minor": 1000,
        "duration_ms": 300000,
        "invocations": 1000,
        "artifact_bytes": 67108864,
        "units": 1000000,
        "concurrency": 4
      },
      "valid_from": 0,
      "valid_until": 4102444800000,
      "dangerous_allow_broad_authority": false
    },
    "enabled": true
  }],
  "runtime": {
    "request_queue": 128,
    "maintenance_interval_ms": 100,
    "maximum_tick_items": 128,
    "global_concurrency": 32,
    "per_run_concurrency": 8,
    "per_branch_concurrency": 4,
    "per_capability_concurrency": 8,
    "effect_threads": 4,
    "effect_queue": 64,
    "cancellation_queue": 32,
    "maximum_effect_claim": 32,
    "lease_duration_ms": 30000
  },
  "adapters": { "process_profiles": [], "model_profiles": [] },
  "shutdown": { "deadline_ms": 10000, "effect_policy": "drain" },
  "command_ledger_bound": 10000
}
JSON
cargo run -p milkdrift-daemon -- --config daemon.json
```

In another terminal:

```sh
export MILKDRIFT_TOKEN_FILE="$PWD/operator.token"
cargo run -p milkdrift-cli -- daemon readiness
cargo run -p milkdrift-cli -- --json capability list
cargo run -p milkdrift-cli -- blueprint import crates/blueprint/tests/fixtures/revision-v2.json
```

The daemon refuses non-loopback plaintext binds and permissive CORS is not enabled. Older configuration schemas are not silently migrated, and broad/unbounded authority requires an explicit dangerous acknowledgement. Empty artifact, layout, peer, and workspace scopes deny access. See [the authority configuration guide](docs/operator-authority.md) and [the control API reference](docs/reference/control-api.md).

A minimal revision is constructed through a validated mutation batch; see the crate-level example in `milkdrift-blueprint` and the integration tests under `crates/blueprint/tests`.

## Repository map

- `crates/capability`: provider-neutral capability, exact resolution, and invocation contracts.
- `crates/authority`: actor identity, scoped immutable grants, deterministic decisions, and opaque secret references.
- `crates/control`: shared human/service/AI workflow proposals, risk policy, authority presets, read models, and the in-process workflow-control capability adapter.
- `crates/control-protocol`: pure version-one external commands, read models, envelopes, cursors, streams, and layout schema.
- `crates/control-client`: authenticated typed HTTP queries, exact command submission, bounded artifact ranges, and resumable SSE.
- `crates/capability-host`: live adapter generations, resolution, admission, cancellation, health, drain, and shutdown.
- `crates/blueprint`: immutable workflow definitions, fingerprints, and revision transactions.
- `crates/model`: provider-neutral model task/response and exact schema-v2 causal-context manifest contracts.
- `crates/workspace`: scoped immutable values, branch lineage, artifact metadata, and budgets.
- `crates/persistence`: versioned events and narrow journal/revision/snapshot/workspace/artifact ports.
- `crates/runtime`: commands, pure projections, scheduling, execution ownership, recovery, reconciliation, and authoritative causal-context discovery/materialization.
- `adapters/redb-store`: transactional local redb storage and content-addressed artifact bytes.
- `adapters/local-process`: versioned safe-argv profiles and the production local process adapter.
- `adapters/model-provider`: bounded HTTP endpoint profiles plus OpenAI-compatible and native Anthropic mappings.
- `adapters/secret-env`: explicit opaque-secret-reference to environment-name resolution.
- `apps/daemon`: authoritative local host, bounded runtime owner, authentication, HTTP/SSE API, recovery, and shutdown.
- `apps/cli`: thin operator client with human and stable schema-v1 JSON output.
- `docs`: status, roadmap, development commands, and durable decisions.
- `.github/workflows/quality.yml`: the primary format/check/test/lint/documentation workflow.
- `.github/workflows/stress.yml`: weekly and manually triggered long-run storage/projection boundary evidence.

The canonical documents are [VISION.md](VISION.md), [ARCHITECTURE.md](ARCHITECTURE.md), [docs/STATUS.md](docs/STATUS.md), [docs/ROADMAP.md](docs/ROADMAP.md), [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md), and [the ADR index](docs/decisions/README.md).

Milkdrift is licensed under either the [MIT license](LICENSE-MIT) or the [Apache License 2.0](LICENSE-APACHE), at your option.
