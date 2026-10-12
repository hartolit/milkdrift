# Daemon

`milkdrift-daemon` owns ordinary workflows, task execution and retained results in one data root.
Configure it once, then connect through the [CLI](../cli/README.md), standalone
[Svelte client](../web/README.md), or [Rust control client](../../crates/control-client/README.md).
A client can submit work, disappear and later rediscover it. The daemon starts configured external
capability adapters; it contains no inference engine. Attached model servers retain their owner,
and configured owned model services use the maintained resource supervisor.

Configuration explicitly selects `workflow_enabled` or `execution_only`. Both roles expose
independent discovery, upload, invocation, observations, cancellation and download. The execution-only
role has no ordinary task scheduler and refuses startup over active task obligations. Settled
workflow history remains in storage. Independent host and
[managed resource](../../docs/decisions/0039-managed-resource-ownership.md) protections apply in both roles.

Use the [operator recipe](../../examples/operator/README.md) to choose a new private data root,
prepare credentials, check configuration and start the binary. Existing stores and services should
be preserved during an incompatible format change. [Daemon operations](../../docs/operations/daemon.md)
owns startup, shutdown, retention and offline administration; [authority configuration](../../docs/operations/authority.md)
owns credential scopes. The `storage-admin` executable path opens the offline store owner without
starting adapters or workers.

## Follow ordinary work

The HTTP layer authenticates a bearer credential, validates protocol framing and sends a typed
call to the bounded daemon owner queue. Task creation records exact acceptance and allocates IDs.
Scheduling freezes ready task definitions, selected input bytes, capability generation and authority
before dispatching external work on fixed workers. Durable observations return through the same
storage owner. Socket handling never waits inside external process or model execution.

A submission receipt and a completed result are different facts. Lost replies can be recovered
through the request receipt or exact replay. Changed reuse conflicts. Missing inputs leave a task
queued without an invented start time. A requested stop stays distinct from proof that the task
stopped. Possible external entry without terminal evidence remains uncertain and is not automatically
repeated. Edits and explicitly adopted corrections change future obligations while preserving history.

Task list and workflow reads use bounded summaries and explicit cursors. A selected task exposes
its accepted instructions and frozen facts separately. Text previews report truncation; immutable
artifacts retain the complete content. Artifact grants can restrict result contents while lifecycle
metadata remains readable. Task SSE invalidates the caller's snapshot; it never replaces task state
or receipt storage.

Browser access is disabled unless `serving.browser_origins` names exact allowed origins. The
[web client setup](../web/README.md) describes memory-only bearer credentials, installation binding,
independent static hosting and trusted HTTPS for phones/LAN access.

## Change the host

`DaemonConfig::load` validates and compiles configuration before `DaemonHost::start` opens the store,
registers capabilities, recovers durable tasks and starts workers. `serve` connects a listener to
that host. Library callers must arrange shutdown; dropping an HTTP connection is not host shutdown.
Startup failure attempts bounded rollback, and failed cleanup remains explicit. Shutdown closes
admission before joining workers and retains uncertainty when terminal evidence cannot be established.

`config` and `auth` own configuration and rotating credentials. `http` owns routes and framing.
The host request/queue modules serialize durable task, authority and independent execution calls.
Startup, maintenance and shutdown keep scheduling and final writes inside the store's lifetime.
Peer and resource modules retain their own capability boundaries. See the
[architecture](../../docs/architecture.md) for the responsibility map.

The `control_plane`, configuration and peer integration suites exercise actual public operations.
The independent JSON cases prove upload, task admission, prospective edit, exact replay, restart
rediscovery and immutable download without private runtime access. The
[verification policy](../../docs/development/workflow.md#choose-verification-for-the-change)
selects the focused checks and full gate for each assignment.
