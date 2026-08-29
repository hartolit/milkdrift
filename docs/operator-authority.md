# Daemon execution authority

Daemon configuration schema 2 requires every actor binding to contain an explicit `authority`
object. Preset names choose command operations; they do not imply resource access. The resource
scope, numeric ceilings, validity interval, grant identity/revision, and revocation generation are
all independent inputs to the immutable grant.

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
        "identities": ["local-example"],
        "categories": [],
        "operations": ["process.execute"],
        "provider_profiles": [],
        "trust_zones": ["local-process"],
        "localities": ["local"],
        "peers": [],
        "maximum_side_effect": "read_only"
      },
      "filesystem": [
        { "root": "/usr/bin", "access": ["read", "execute"] },
        { "root": "/var/lib/milkdrift/work", "access": ["read", "write"] }
      ],
      "network": { "profiles": [], "destinations": [] },
      "secrets": []
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

Capability scope fields are allowlists. An empty identity/category/profile/trust/locality/peer set
means that dimension is not narrowed, but operations must be explicit in a safe configuration.
Filesystem roots are normalized absolute lexical roots. Network destinations are credential-free
`host:port` values and network profiles are named immutable transport profiles. Secret references
are opaque names; secret values never belong in the document.

Every numeric ceiling must be present for a safe grant, including provider-neutral `units`. Use a
finite `valid_until`, declare the strongest side effect the actor may cause, and grant only the
filesystem, network, secret, locality, trust zone, and peer facts required by registered adapters.
The daemon validates these facts before it opens storage.

Wildcard workflow scope, unknown side effects, omitted capability operations, infinite validity,
or missing ceilings are rejected unless `dangerous_allow_broad_authority` is `true`. That flag is a
deliberate acknowledgement, not a shortcut for generating hidden wildcard facts. Broad grants
remain limited by the resources written in the configuration.

Schema 1 configurations are rejected. Migration is manual: add an explicit `authority` object,
choose finite limits, advance `schema_version` to 2, run `milkdrift-daemon --config PATH
--check-config`, and inspect the redacted effective configuration before starting the daemon. When
narrowing or revoking, advance the configured revocation generation or disable the actor and restart;
already-entered external work keeps its truthful terminal history, while future resolution or entry
is denied.
