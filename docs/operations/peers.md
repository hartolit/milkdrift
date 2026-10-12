# Operating peer connectivity

Use peers when a workflow needs a capability on another configured machine. The originating daemon
keeps the workflow history; the serving daemon retains its accepted execution and reports evidence
back. Configure connectivity and authority on both sides before expecting remote registrations.

## Connect two known hosts

Milkdrift consumes a URL you configure; it does not discover peers or make them internet reachable. Provide connectivity with localhost, a private LAN, WireGuard, Tailscale, an SSH/reverse tunnel, or an HTTPS reverse proxy according to your own operational model. No one option is endorsed or required.

For any non-loopback URL, use HTTPS with certificates managed by your platform or reverse proxy. Milkdrift does not issue certificates or run a CA. Keep the daemon listener on loopback behind the proxy unless a later hardened listener explicitly supports your deployment. The named insecure development option accepts only loopback/localhost and is disabled by default.

Peer state is explicit in TOML schema 14. Both roles require a top-level stable `host_id` and share
the top-level `serving` policy. `[peers] mode = "disabled"` excludes relationships; `mode = "enabled"`
permits explicit relationship tables. Configure both sides with different host identities and inverse
relationship entries. The origin uses `workflow_enabled`; the target may use `execution_only`.
Keep bearer values in file/environment `secret_sources` and use distinct relationship credentials.
Values rotate at request time. Set request rate, concurrency, duration, cost with its exact currency,
input/output units, observations and artifact-byte ceilings explicitly. `artifact_sensitivities`
must permit the intended content: an empty set denies transfer even with upload/download actions.
Process/model capabilities also require `execution_filesystem`, `execution_network_profiles`,
`execution_network_destinations` and `execution_secrets` matching their adapter requirements.
Capability allowlisting grants none of those resources. Changing identities, scopes, quotas, expiry
or revocation requires validated restart. `peer reload` replaces the authenticated catalog;
`peer revoke` rejects inbound actions and drains outbound registrations until restart. Update the
credential and configuration for durable revocation.

Start with `actions = []`, empty capability/operation allowlists, and add only required capability identities and operations. The configured action list is expanded at startup into an ordinary immutable authority grant: `read_catalog` covers session negotiation, peer inspection, capability listing/health, and provider-profile inspection; invoke, cancel, upload/download, and administration remain separate typed operations. The action list is not consulted as a second executable permission system. Controller, process, filesystem, model, artifact, and workflow-mutation access is never implied by a valid credential. Use finite expiry, conservative concurrency/duration/cost/artifact limits, and a trust zone that workflow capability policy can require or forbid.

## Pin tasks to approved hosts

Select the exact peer capability returned by `milkdrift workflow capabilities` after connecting
the approved host. Put that identity and operation in the task request's `capability` object.
The [maintained task request](../../examples/operator/peer-placement.json) contains a placeholder
that must be replaced with an actual advertised identity. Submit it with `milkdrift workflow create`.
A local capability with the same operation does not satisfy a different exact peer identity.

Each serving host pins its own executable and working directory. Connectivity does not copy a
checkout, credentials or filesystem mount. The origin grant must cover the selected remote
capability and the serving relationship separately authorizes the exact execution resources.
Expired, unavailable or drained selections leave tasks queued with a reason; no wrong-host fallback
is allowed. Start freezes the selected peer/catalog/descriptor generation and authority. Reconnect
and restart preserve accepted selection. Public task reads expose generation and selected request;
the retained runtime snapshot owns complete dispatch provenance.

Observed remote outputs are imported through authorized core artifact transfers before the origin
reports them as outputs. Enable the serving relationship's `artifact_download` action and permitted
sensitivities; the origin also enforces its configured relationship byte and sensitivity bounds.
The accepted peer execution owns the serving publication allowance. Its producer and origin-side
causal references remain tied to that request, without creating a local workflow history. Transfer
failure after entry preserves uncertainty. Referenced inputs still require explicit preparation
and transfer; this does not make origin-side workspace references readable on another host.

The [workflow test](../../apps/daemon/tests/two_daemon_peer/placement.rs) runs three production daemon
host instances: an origin and two serving peers. They communicate through real HTTP listeners on
local TCP, use separate temporary stores, credentials and repository directories, and execute the
built process helper. It checks distinct entry counters, returned bytes, frozen provenance and
restart. Temporary loopback is the automated test topology; the implemented peer path uses the
configured URLs described above. Physical multi-machine deployment has not been qualified by this
test.

## Inspect and refresh registrations

Use the local CLI to inspect configured relationships and request an explicit lifecycle action:

```sh
milkdrift peer list
milkdrift peer show PEER_ID
milkdrift --yes peer connect PEER_ID
milkdrift --yes peer reload PEER_ID
milkdrift --yes peer drain PEER_ID
milkdrift --yes peer revoke PEER_ID
```

`peer list/show` first filters configured relationships through the local actor's exact peer scope, then reports authenticated connection health, remote session ID, exact catalog generation/digest/expiry, registration count, and live revocation without transport secrets. `connected` means an authenticated session and verified catalog—even when filtering produces zero registrations—and is not shared workflow truth. Check the reported expiry before starting work. The serving catalog is filtered by the relationship's expanded capability/provider/health grant before projection. Inspect `capability list` for mapped generation health and provenance. A disconnect drains new resolution while exact accepted work follows its durable observation/uncertainty rules; reconnect after an irreversible drain creates a fresh local registration generation even when the remote catalog is still cached.

Reload may return a still-live cached catalog with the same expiry. A renewed catalog replaces the
local adapter even when the remote descriptor is unchanged. Old registrations remain while exact
execution permits are held and are reclaimed on later reload/disconnect after those permits leave.
The host's generation bounds still apply when old work has not drained. Plan a bounded invocation
inside the catalog's remaining lifetime; expiry during execution may preserve uncertainty.

## Retain and recover accepted work

Serving work uses the schema-14 top-level `serving` policy in both roles. `worker_threads`,
`maximum_global_active`, and `maximum_dispatch_queue` bound live ownership;
`maximum_hot_terminal_records` bounds terminal detail and reserves capacity for active work's
eventual disposition. `archive_batch_size` bounds each compaction pass, and
`observation_hot_retention_ms` sets the minimum age for removing reconnectable detail. Keep the
hot bound at least as large as the global active bound and the batch no larger than the hot bound.
These settings are independent of workflow receipts and security-audit retention.

Graceful shutdown stops new acceptance and claims, joins workers until the configured deadline, and reports retained workers/effects if the deadline expires. Active executions are never archived. Once terminal or uncertain history is older than the observation horizon, maintenance or the new-admission transaction can atomically replace it with a compact tombstone and reclaim a hot slot. Exact request replay and digest conflict remain permanent within the store generation. Archived lookup/observation responses are explicitly typed and retain the final terminal observation or uncertainty plus an observation-chain digest; intermediate progress/stream rows are no longer queryable. The archive retains at most 256 named output observations so an interrupted consumer can recover exact result bytes through current transfer authorization. Output overflow refuses append and preserves uncertainty. Core artifact retention and provenance are unchanged.

After durable entry, the peer execution service classifies adapter failure, missing terminal
evidence, and adapter panic once. The worker owns only the exact pending release-or-uncertainty
operation and retries it unchanged through transient clock or store failure. A pre-entry claim is
released for retry, while an entered claim is never sent to the adapter again. Existing terminal or
uncertain evidence makes a repeated recovery write an idempotent no-op. Shutdown performs a final
in-memory retry when possible; a still-unavailable boundary leaves the durable entered claim for
startup recovery rather than guessing an outcome.

Detailed health reports active, dispatch, hot-terminal, and tombstone counts; configured active/queue/hot/batch bounds; archive generation/time; and a redacted degraded reason. Readiness remains deliberately coarse. A nonzero degraded state means admission stays closed after restart verification; inspect storage and restore a known-good generation rather than editing redb rows.

Storage compatibility and legacy-directory refusal follow the [daemon startup and backup rules](daemon.md).
[Development workflow](../development/workflow.md) owns the ordinary and release peer tests.

The [peer protocol](../reference/peer-protocol.md) owns exact message and archived-history forms.
The [HTTP adapter guide](../../adapters/peer-http/README.md) explains origin and serving lifecycles.
Keep endpoints operator-owned and grants explicit; a relationship does not authorize sharing a
database, forwarding secret/config artifacts, or treating remote filesystem state as local state.
