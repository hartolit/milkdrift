# Operating peer connectivity

Milkdrift consumes a URL you configure; it does not discover peers or make them internet reachable. Provide connectivity with localhost, a private LAN, WireGuard, Tailscale, an SSH/reverse tunnel, or an HTTPS reverse proxy according to your own operational model. No one option is endorsed or required.

For any non-loopback URL, use HTTPS with certificates managed by your platform or reverse proxy. Milkdrift does not issue certificates or run a CA. Keep the daemon listener on loopback behind the proxy unless a later hardened listener explicitly supports your deployment. The named insecure development option accepts only loopback/localhost and is disabled by default.

Configure both sides with stable, different `local_peer_id` values and inverse relationship entries. Keep bearer values in file/environment `secret_sources`; use distinct credentials per relationship where possible. Values rotate at request time. Set `maximum_requests_per_minute`, `maximum_concurrent`, duration, cost, observation, and artifact-byte ceilings explicitly for production relationships. Remote process/model capabilities also require explicit `execution_filesystem`, `execution_network_profiles`, `execution_network_destinations`, and `execution_secrets` authority matching their adapter-declared requirements; capability allowlisting alone grants none of those host resources. Unknown secret references and malformed resource scopes fail configuration validation. Changing identity mappings, allowlists, resources, quotas, expiry, or revocation generation requires validated configuration restart; `peer reload` re-authenticates and replaces only the current remote catalog. `peer revoke` immediately rejects inbound protocol actions and drains outbound registrations until restart; update the credential and configuration for durable revocation.

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

Serving work uses the configured fixed runtime worker count and durable global/per-peer/queue bounds. Graceful shutdown stops new acceptance and claims, joins workers until the configured deadline, and reports retained workers/effects if the deadline expires. Terminal peer records are retained by default; explicit archival only marks a bounded eligible page and preserves idempotency, security, observation, and provenance facts. If the non-evicting record ceiling is reached, new request identities are rejected until an operator-approved future retention policy makes room.

This pre-release store format refuses physical schema versions other than 5 and internal document formats other than 8. Startup also refuses obsolete `peer-executions-v1` or `peer-artifacts-v1` directories instead of silently ignoring or partially importing prototype authority.

Never put workflow/model-controlled URLs into relationship configuration, expose a permissive browser CORS realm, forward secret/config artifacts, or mount a peer's database/filesystem as local state. NAT traversal, overlay routing, shared databases, mesh discovery, hosted coordination, consensus, model synchronization, and tensor transfer remain external/non-goals.
