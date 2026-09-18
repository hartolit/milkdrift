# Daemon

`milkdrift-daemon` runs workflows and retains their accepted history in one data root. Operators
configure it once, then use the [CLI](../cli/README.md) or
[control client](../../crates/control-client/README.md) to submit work and inspect results. It
starts the configured external capability adapters; model servers remain separately managed.

Configuration explicitly selects `workflow_enabled` or `execution_only`. Execution-only startup
constructs common storage, authority and capability owners without runtime, control or workflow
workers. It refuses active workflow obligations and reports absent workflow operations explicitly.
Closed history remains available through offline storage inspection. Both roles expose authenticated
direct discovery, upload, invocation, bounded observations, cancellation and downloads through the
[independent execution recipe](../../examples/operator/README.md#independent-execution).
Managed service setup is separately assigned under
[resource ownership](../../docs/decisions/0039-managed-resource-ownership.md).

Use the [fresh-directory operator recipe](../../examples/operator/README.md) to build the binary,
prepare a private credential, check `daemon.toml`, start a terminal-only workflow, and restart it.
[Daemon operations](../../docs/operations/daemon.md) owns startup, retention, shutdown, and backup.

`milkdrift-daemon --config ./daemon.toml --recovery` opens authenticated repair controls with
execution disabled. It uses existing proposal approval and prospective reconciliation, reports
live but not ready, and starts no adapters or workers. Normal restart validates repaired state
before dispatch. See [authorized recovery controls](../../docs/operations/daemon.md#authorized-recovery-controls).
[Authority configuration](../../docs/operations/authority.md) explains which operations and
resources each credential grants. Process, model, and peer setup remain in their linked guides.

The executable's explicit `storage-admin` path provides offline inspection, backup and restore.
It bypasses configuration, credentials and runtime recovery, composing the redb offline owner
with pure historical diagnostics. Follow the
[storage operations procedure](../../docs/operations/daemon.md#offline-storage-administration)
to inspect a blocked generation or preserve a stopped one without dispatching work.

## Follow a request

The HTTP layer authenticates the bearer credential and decodes a protocol request. It sends a
typed call to one bounded owner thread, which checks authority and delegates to the runtime,
control service, or storage owner. Results return through that same call. External processes,
model requests, and peer work run on fixed workers so they do not occupy the socket reactor.

```text
HTTP request → authentication → owner queue
                                    |
                                    v
                      authority and command/read owner → HTTP reply
                                    |
                                    v
                         scheduling → effect workers
                                                  |
                                                  v
                                         durable observations
```

Command acceptance and task completion are different observations. If a command reply is lost,
its retained receipt permits exact replay; changing the request under its old ID conflicts.
If external work loses its result after entry, the runtime can retain uncertainty. Replaying a
command receipt does not resolve that uncertainty or invoke the external capability again.

## Change the host

Begin at `DaemonConfig::load`, which validates and compiles configuration into a `DaemonPlan`.
`DaemonHost::start` opens storage with admission closed, recovers active work, registers adapters,
and starts workers before returning ready. `serve` connects a listener and shutdown future to
that host. A caller embedding the library must arrange orderly shutdown; dropping a socket is
not the completion of the host's lifecycle.

The private modules divide that operation:

- `config` compiles operator choices; `auth` maps rotating credential sources to exact grants.
- `http` owns routing, framing, response bounds, and stream polling.
- `host/queue` and `host/requests` serialize durable calls. Command families, receipts, layouts,
  and read projections delegate to the domain that owns their facts.
- `host/startup`, `maintenance`, `clock`, and `shutdown` keep recovery, scheduling, retention,
  and final worker writes within the host lifecycle. Health reports operational state rather
  than replacing journal evidence.
- `host/peers` and `peer_store` connect peer workers to the same storage owner without extending
  its lifetime through router handles.

The [architecture](../../docs/architecture.md) owns the full responsibility map. Explicit
[controller activation](../../docs/operations/daemon.md#controller-activation) installs the
accounted lifecycle before recovery; ordinary startup remains disabled by default. The non-default
`controller-qualification` feature also permits the isolated development configuration.
The `control_plane`, `configuration_cli`, and `two_daemon_peer` tests check public request,
configuration, recovery, authority, and peer behavior. The
[verification policy](../../docs/development/workflow.md#choose-verification-for-the-change)
selects the required checks for a change.
