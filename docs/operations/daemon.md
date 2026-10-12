# Daemon operation and durable task state

Run one `milkdrift-daemon` for a configured data root. It owns persistence, task execution and
configured external adapters. Clients connect through authenticated HTTP. A second writer is
refused while the first holds the store lock. Begin with the
[operator configuration](../../examples/operator/README.md) and use a new explicit development
root for the task cutover. Preserve existing services, old stores and Git checkpoints.

## Configuration and startup

Configuration is TOML schema 14. `milkdrift-daemon --config PATH --check-config` validates configuration;
`--config PATH --print-effective-config` prints normalized redacted configuration. Neither proves storage
recovery or executable readiness. Check the binary's `--help` for the accepted argument form.
Paths resolve relative to the configuration file. Changes to grant content, profile sources,
worker limits or peer relationships require validation and restart. Credentials resolve from the
configured file or exact environment variable at use time; they can rotate without logging them.
The [authority guide](authority.md) owns scopes and revocation.

`role = "workflow_enabled"` constructs the ordinary task runtime. `role = "execution_only"`
serves independent configured capabilities without a workflow scheduler and refuses retained
non-completed workflows. Both roles retain managed installations and independent serving guards.
The stable `host_id` is bound to the store; changing it is not a way to adopt another host's data.

Startup validates the compiled plan, acquires the store lock, checks the exact
[current formats](../reference/versions.md), establishes the durable clock, restores accepted
execution state and registers configured adapters. An interrupted task with durable entry intent
and no terminal evidence becomes uncertain; it is not blindly sent again. Claims and reservations
remain owned until durable settlement or an authorized prospective decision resolves them.
Legacy formats and recognized legacy directories are refused before writes. No migration tool or
compatibility reader is included. Inspect or back up an old store with its matching executable.

Readiness is coarse and authenticated; detailed health reports owner queue, worker and peer-serving
state without task content or credentials. Startup success alone does not qualify a configured
process, endpoint, managed installation or physical peer. Use a bounded real task to observe it.

## Bounds and shutdown

The runtime owner queue, effect threads, dispatch queue, cancellation queue and claim batch are
finite configuration values. Workflow policy also bounds admitted tasks and cumulative usage.
Persistence reserves room to record starts, settlement and the named stop action before ordinary
admission can consume that room. Capacity refusal is explicit. Continuing a workflow does not
refund consumed allowance; uncertainty does not manufacture an exactly-once outcome.

On shutdown, admission closes and task scheduling stops before effect workers drain or cancel under
the configured policy and deadline. A cancellation request is distinct from terminal cancellation
evidence. Deadline expiry can leave uncertainty; do not turn that observation into a success claim.
Restart recovers from durable facts. A clock that cannot safely advance closes admission rather
than backdating effects or authority.

Independent peer/direct serving has separate active, dispatch, observation and retained-record
bounds. Its compact receipts preserve exact replay and conflict behavior. It never acquires local
workflow ownership merely because a remote task carries originating coordinates.

## Browser and static hosting

Configure exact `serving.browser_origins`, for example `http://127.0.0.1:5173`. An empty list disables
browser origins. Mismatched origins and malformed preflight requests are refused. Svelte is an
independent static application; it cannot read the database or invoke the CLI. Its bearer tokens
stay in memory. The daemon checks actor scope and expected installation identity on each task
request. See [web hosting](../../apps/web/README.md) for build commands and connection setup.

For phone/LAN use, terminate browser-trusted HTTPS for both the static site and daemon proxy.
Keep the daemon's listener private, forward bearer/identity headers and disable buffering on the
fetch stream. Loopback HTTP is the development exception. Phone-sized local browser checks do not
qualify a physical phone, a proxy deployment or another network topology.

## Offline inspection and backup

Stop the writer before using `milkdrift-daemon storage-admin`. Inspection, paged record inventory,
integrity checks and manifest-backed backup read the current storage owner. They do not execute
work or reconstruct removed graph projections. Use `storage-admin --help` for exact subcommands;
record families include workflows, receipts, peers, peer tombstones, artifacts and managed state.

Keep database files, immutable artifact blobs, native claim records and their identity/accounting
metadata together. Copying only a database file can lose retained output. A successful backup check
means the declared files and digests matched; it does not prove a stopped process, network service
or physical installation was quiescent. Never edit redb rows to force recovery. Retain the original
root and establish a separate explicitly authorized restore root.

[Peer connectivity](peers.md) and [managed Linux operations](managed-linux.md) describe their
independent lifetimes and protections. The full validation gate remains governed by the
[development workflow](../development/workflow.md).
