# Daemon control and execution authority

Daemon configuration schema 8 requires every actor binding to contain an explicit `authority`
object. Preset names deterministically expand to typed operation sets; they do not imply resource
access and are not retained as executable session policy. The resource scope, numeric ceilings,
validity interval, grant identity/revision, and revocation generation are independent inputs to the
immutable schema-4 grant. Authentication selects that exact actor and grant but grants nothing by
itself.

The following safe pattern grants one actor access to one workflow lineage and one local process
capability. Adjust every value to the actual immutable capability profile and workflow you intend
to run:

```json
{
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
        "type": "allow",
        "identities": { "type": "only", "values": ["local-example"] },
        "categories": { "type": "any" },
        "operations": { "type": "only", "values": ["process.execute"] },
        "provider_profiles": { "type": "any" },
        "trust_zones": { "type": "only", "values": ["local-process"] },
        "execution_trust_classes": { "type": "only", "values": ["trusted_host_process"] },
        "localities": { "type": "only", "values": ["local"] },
        "peers": { "type": "any" },
        "maximum_side_effect": "read_only"
      },
      "filesystem": [
        { "root": "/usr/bin", "access": ["read", "execute"] },
        { "root": "/var/lib/milkdrift/work", "access": ["read", "write"] }
      ],
      "network": { "profiles": [], "destinations": [] },
      "secrets": [],
      "artifacts": { "type": "deny_all" },
      "layouts": { "type": "deny_all" },
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
}
```

Capability authority is either `{ "type": "deny_all" }` or an explicit conjunctive allow scope.
Every allow dimension is `{ "type": "any" }` or `{ "type": "only", "values": [...] }`.
`Only` requires 1..=128 ordered unique values; an empty array is invalid and never means wildcard.
The side-effect ceiling applies in addition to every selector.
Filesystem roots are normalized absolute lexical roots. Network destinations are credential-free
`host:port` values and network profiles are named immutable transport profiles. Secret references
are opaque names; secret values never belong in the document.

Artifact authority is either `{ "type": "deny_all" }` or an allow scope containing an explicit
`Any`/nonempty `Only` identity selector and a nonempty sensitivity set; there is no implicit empty
identity wildcard. Layout authority is either deny-all or a shared-layout scope with an explicit
revision selector. Actor-owned/private layouts are reserved in the authority vocabulary but are
not implemented by the control protocol, daemon, or persistence adapter. Empty peer and workspace
scopes deny access unless their explicit wildcard boolean is set. Daemon flags independently grant coarse
readiness, detailed health, the caller's own authority view, redacted configuration, and bounded
audit views. Protected artifact/provider/peer/health details are therefore not implied by workflow
inspection.

Every numeric ceiling must be present for a safe grant, including provider-neutral `units`. Use a
finite `valid_until`, declare the strongest side effect the actor may cause, and grant only the
filesystem, network, secret, locality, trust zone, and peer facts required by registered adapters.
The daemon validates these facts before it opens storage.

`trusted_host_process` authorizes code that runs with the daemon account's host privileges. The
adapter mediates argv, environment, selected materialization, and declared output import, but it is
not a filesystem or network sandbox. Grant this class only to explicitly byte-pinned process
generations. `sandboxed_process` is a distinct exact class; granting or requiring it never permits
the current local-process adapter.

Wildcard workflow, capability target/operation, artifact, peer, or workspace scope; unknown side
effects; infinite validity; or missing ceilings are rejected unless
`dangerous_allow_broad_authority` is `true`. That flag is a deliberate acknowledgement, not a
shortcut for generating hidden wildcard facts. Broad grants remain limited by the resources
written in the configuration.

Older configuration and authority-grant schemas are rejected. Migration is manual: start from a
reviewed schema-8 configuration, replace every legacy capability and artifact array with an
explicit `Any` or nonempty `Only` selector, retain `DenyAll` where no invocation or presentation
access is intended, choose finite limits,
and explicitly configure each peer relationship's `artifact_sensitivities`. Run
`milkdrift-daemon --config PATH --check-config`, and inspect the redacted
effective configuration before starting the daemon. When narrowing or revoking, advance the grant
revision/revocation generation or disable the actor and restart. Existing page and reconnect
cursors then fail closed; open streams stop future disclosure on their next bounded check;
already-entered external work keeps its truthful terminal history.
