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

Current workflow presets include project inspection, and Controller also includes project management
and daemon renaming. Expanded operation sets change grant digests; publish changed grant content
under a new `grant_revision`. Existing workflows retain their
original execution claim. Storage adoption preserves that claim and its exact command receipts; it
does not authorize them under the expanded grant. Current authenticated clients can still inspect
retained work and recover their receipts, but a ready queued task whose original grant no longer
matches is refused before entry and records a failure. `Continue` does not replace that claim.
Finish or deliberately stop outstanding work using its preserved executable and original
configuration before upgrading an affected preset. Never edit saved claims, digests or receipts to make a
new grant appear to have authorized earlier work.

`role = "workflow_enabled"` constructs the ordinary task runtime. `role = "execution_only"`
serves independent configured capabilities without a workflow scheduler and refuses retained
non-completed workflows. Both roles retain managed installations and independent serving guards.
The stable `host_id` is bound to the store; changing it is not a way to adopt another host's data.

Startup validates the compiled plan, acquires the store lock, checks the exact
[current formats](../reference/versions.md), establishes the durable clock, restores accepted
execution state and registers configured adapters. An interrupted task with durable entry intent
and no terminal evidence becomes uncertain; it is not blindly sent again. Claims and reservations
remain owned until durable settlement or an authorized prospective decision resolves them.
An unentered frozen start paused by a person returns to the ordinary queue with its same inputs,
capability and invocation identity. Continue permits scheduling; execution rechecks current authority
and physical identity before entry. It never repeats an entered or uncertain invocation. Known overlapping native
project claims make ordinary tasks wait with **Project is in use**. Independent direct/peer serving
retains its separate pre-entry refusal policy. Native claims coordinate Milkdrift operations, not
external editors or a filesystem sandbox.
Physical schema 21 atomically adopts exact physical schemas 19 or 20 with internal format 22 by
adding the project tables; schema 19 also receives its missing daemon metadata tables. Existing
workflow and receipt encodings remain unchanged; interruption leaves either the original table set
or the complete adopted format. Older document formats, graph stores and recognized legacy
directories are refused before writes. Inspect or back up an older store with its matching executable.

The optional top-level `name` seeds shared metadata only when absent; existing schema-14
configurations without it seed `Milkdrift`. Restart never overwrites an accepted rename. Names
trim surrounding whitespace, require 1–80 Unicode scalar values and refuse control characters.
The `host_id` remains the installation identity; equal display names do not join two installations.

An administrator can change the name in **Daemon settings** or with the
[CLI daemon commands](../../apps/cli/README.md). Controller authority must also permit
`authority.resources.daemon.configuration`; workflow-edit permission alone is insufficient.
Read the current generation before renaming and keep the exact request ID/name/generation if its
reply is lost. Receipt lookup or exact replay recovers the accepted result. Conflicts require a
fresh read and explicit review. At 1,024 accepted receipts the daemon refuses new renames while
retaining reads and replay; there is no eviction or automatic reset.

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
stay in memory. Connect with its address and credential. The client learns the daemon identity and
shared name, saves the identity with the endpoint, and binds subsequent task requests to it.
An address serving a different daemon requires deliberately forgetting that saved connection and
connecting afresh. The name refreshes across connected clients; disconnected names are explicitly
cached. First-use trust comes from the configured endpoint and transport, not the returned identity.
See [web hosting](../../apps/web/README.md) for build commands and connection setup.

The client shows the daemon's shared name in its navigation and contextual header. On a narrow
screen, open **Workspace navigation** to switch daemons or reach **Daemon settings**. Register a
permitted ordinary folder as a project and review its execution defaults before creating work.
**New workflow** creates a workflow in that project with its first task; **Add task** submits to the
selected workflow. Both use a compact composer with instructions, optional context/files and a visible executor. The rail keeps
current/recent workflows available, while **All workflows** provides discovery and search. Selected task
details sit beside the timeline on wide screens and below it on narrow screens. Hiding the inspector
preserves an unfinished edit; it does not stop or change daemon work. Results and technical facts
use labeled disclosures, while execution state and stop requests remain visible.
Task drafts remain in memory across navigation, bounded to eight per authenticated connection;
disconnect or lost authority clears them. See [client authoring and recovery](../../apps/web/README.md)
for exact replay, advanced editing and draft disposal.

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
