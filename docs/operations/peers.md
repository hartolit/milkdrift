# Operating peer connectivity

Milkdrift consumes a URL you configure; it does not discover peers or make them internet reachable. Provide connectivity with localhost, a private LAN, WireGuard, Tailscale, an SSH/reverse tunnel, or an HTTPS reverse proxy according to your own operational model. No one option is endorsed or required.

For any non-loopback URL, use HTTPS with certificates managed by your platform or reverse proxy. Milkdrift does not issue certificates or run a CA. Keep the daemon listener on loopback behind the proxy unless a later hardened listener explicitly supports your deployment. The named insecure development option accepts only loopback/localhost and is disabled by default.

Peer state is explicit in TOML schema 9: `[peers] mode = "disabled"` has no identity, relationships,
or serving policy, while `mode = "enabled"` requires one `local_peer_id` and permits explicit
relationship and serving tables. Configure both sides with stable, different identities and inverse relationship entries. Keep bearer values in file/environment `secret_sources`; use distinct credentials per relationship where possible. Values rotate at request time. Set `maximum_requests_per_minute`, `maximum_concurrent`, duration, cost, observation, and artifact-byte ceilings explicitly for production relationships. Relationships must also name `artifact_sensitivities`; an empty set denies transfer even when an upload/download action is present. Remote process/model capabilities require explicit `execution_filesystem`, `execution_network_profiles`, `execution_network_destinations`, and `execution_secrets` authority matching their adapter-declared requirements; capability allowlisting alone grants none of those host resources. Unknown secret references and malformed resource scopes fail configuration validation. Changing identity mappings, allowlists, resources, quotas, expiry, or revocation generation requires validated configuration restart; `peer reload` re-authenticates and replaces only the current remote catalog. `peer revoke` immediately rejects inbound protocol actions and drains outbound registrations until restart; update the credential and configuration for durable revocation.

Start with `actions: []`, empty capability/operation allowlists, and add only required capability identities and operations. The configured action list is expanded at startup into an ordinary immutable authority grant: `read_catalog` covers session negotiation, peer inspection, capability listing/health, and provider-profile inspection; invoke, cancel, upload/download, and administration remain separate typed operations. The action list is not consulted as a second executable permission system. Controller, process, filesystem, model, artifact, and workflow-mutation access is never implied by a valid credential. Use finite expiry, conservative concurrency/duration/cost/artifact limits, and a trust zone that workflow capability policy can require or forbid.

Diagnostics:

```sh
milkdrift peer list
milkdrift peer show PEER_ID
milkdrift --yes peer connect PEER_ID
milkdrift --yes peer reload PEER_ID
milkdrift --yes peer drain PEER_ID
milkdrift --yes peer revoke PEER_ID
```

`peer list/show` first filters configured relationships through the local actor's exact peer scope, then reports authenticated connection health, remote session ID, exact catalog generation/digest/expiry, registration count, and live revocation without transport secrets. `connected` means a current authenticated session and verified catalog—even when filtering produces zero registrations—and is not shared workflow truth. The serving catalog is filtered by the relationship's expanded capability/provider/health grant before projection. Inspect `capability list` for mapped generation health and provenance. A disconnect drains new resolution while exact accepted work follows its durable observation/uncertainty rules; reconnect after an irreversible drain creates a fresh local registration generation even when the remote catalog is still cached.

Serving work uses the schema-9 enabled-mode `peers.serving` policy. `worker_threads`, `maximum_global_active`, and `maximum_dispatch_queue` bound live ownership; `maximum_hot_terminal_records` bounds completed/uncertain detailed history; `archive_batch_size` bounds each oldest-first compaction pass; and `observation_hot_retention_ms` prevents premature loss of reconnectable detail. These values are validated independently of `application_receipts` and security-audit retention. Keep the hot bound at least as large as the global active bound and choose a batch no larger than the hot bound.

Graceful shutdown stops new acceptance and claims, joins workers until the configured deadline, and reports retained workers/effects if the deadline expires. Active executions are never archived. Once terminal or uncertain history is older than the observation horizon, maintenance or the new-admission transaction can atomically replace it with a compact tombstone and reclaim a hot slot. Exact request replay and digest conflict remain permanent within the store generation. Archived lookup/observation responses are explicitly typed and retain the final terminal observation or uncertainty plus an observation-chain digest; intermediate progress/stream rows and peer observation-artifact links are no longer queryable. Core artifact retention and provenance are unchanged.

Detailed health reports active, dispatch, hot-terminal, and tombstone counts; configured active/queue/hot/batch bounds; archive generation/time; and a redacted degraded reason. Readiness remains deliberately coarse. A nonzero degraded state means admission stays closed after restart verification; inspect storage and restore a known-good generation rather than editing redb rows.

This pre-release store format refuses physical schema versions other than 8 and internal document formats other than 11. Format 11 persists the hot/tombstone placement authority, rolling observation digest, and independent peer-retention accounting. Startup also refuses obsolete `peer-executions-v1` or `peer-artifacts-v1` directories instead of silently ignoring or partially importing prototype authority.

Run the ordinary two-daemon turnover/restart check with `cargo test -p milkdrift-daemon --test two_daemon_peer`. For a longer manual lane, use `cargo test --release -p milkdrift-daemon --test two_daemon_peer peer_execution_retention_longevity_survives_turnover_and_restart -- --ignored --exact`. It crosses the tiny hot bound repeatedly, restarts both daemons, verifies tombstone accounting, and proves that a new remote invocation still completes.

Never put workflow/model-controlled URLs into relationship configuration, expose a permissive browser CORS realm, forward secret/config artifacts, or mount a peer's database/filesystem as local state. NAT traversal, overlay routing, shared databases, mesh discovery, hosted coordination, consensus, model synchronization, and tensor transfer remain external/non-goals.
