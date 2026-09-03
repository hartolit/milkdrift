# Daemon control and execution authority

Daemon configuration schema 9 requires every actor binding to contain an explicit `authority`
table. Preset names deterministically expand to typed operation sets; they do not imply resource
access and are not retained as executable session policy. The resource scope, numeric ceilings,
validity interval, grant identity/revision, and revocation generation are independent inputs to the
immutable schema-4 grant. Authentication selects that exact actor and grant but grants nothing by
itself.

The checked [schema-9 fixture](../../apps/daemon/tests/fixtures/daemon-config-v9.toml) is the complete
safe pattern for one actor, one workflow lineage, and one local process capability. Its central
selection reads as ordinary TOML:

```toml
[[actors]]
credential_ref = "credential:operator"
actor = "human:operator"
grant_id = "grant:operator"
grant_revision = 1
preset = "controller"
enabled = true

[actors.authority.resources.workflow_run]
type = "workflow"
workflow = "example-workflow"

[actors.authority.resources.capability]
type = "allow"
maximum_side_effect = "read_only"

[actors.authority.resources.capability.identities]
type = "only"
values = ["local-example"]

[actors.authority.resources.capability.operations]
type = "only"
values = ["process.execute"]
```

Adjust every value and every remaining selector in the complete fixture to the actual immutable
capability profile and workflow you intend to run.

Capability authority is either `{ "type": "deny_all" }` or an explicit conjunctive allow scope.
Every allow dimension is `{ "type": "any" }` or `{ "type": "only", "values": [...] }`.
`Only` requires 1..=128 ordered unique values; an empty array is invalid and never means wildcard.
The side-effect ceiling applies in addition to every selector. Filesystem roots use one canonical
durable grammar: Unix roots are `/` or begin with `/`, and ordinary Windows drive roots are `C:/`
or begin with an uppercase ASCII drive plus `:/`. Both forms use `/` separators and compare exact
components rather than string prefixes. A Unix root never contains a Windows root, different
Windows drives never contain one another, and component case is exact. Host adapters first
canonicalize native paths and then convert them to this grammar; exact case intentionally fails
closed rather than approximating Windows Unicode case rules. Relative paths, traversal, repeated
or trailing separators, backslashes or mixed separators, drive-relative paths, UNC/device paths,
alternate-data-stream syntax, reserved Windows device names/characters, trailing-dot/space aliases,
control characters, and non-UTF-8 paths are refused. Broad authority is explicit as Unix `/` or a
named Windows drive root such as `C:/`; `/` is not a cross-platform wildcard. Network destinations
are credential-free `host:port` values and network profiles are named immutable transport profiles.
Secret references are opaque names; secret values never belong in the document.

This path correction does not advance authority-grant schema 4 or daemon configuration schema 9.
The field shape and canonical meaning remain an absolute normalized root, existing Unix canonical
bytes and digests are unchanged, and older readers refuse rather than misinterpret newly valid
Windows drive roots.

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

Older configuration and authority-grant schemas are rejected. JSON configuration has no fallback
reader. Migration is manual: start from a reviewed schema-9 TOML configuration, replace every
legacy capability and artifact array with an
explicit `Any` or nonempty `Only` selector, retain `DenyAll` where no invocation or presentation
access is intended, choose finite limits,
and explicitly configure each peer relationship's `artifact_sensitivities`. Run
`milkdrift-daemon --config PATH --check-config`, then inspect
`--print-effective-config` before starting the daemon. The effective output is normalized TOML,
redacts secret-source details, and is independent of source comments and formatting. When narrowing or revoking, advance the grant
revision/revocation generation or disable the actor and restart. Existing page and reconnect
cursors then fail closed; open streams stop future disclosure on their next bounded check;
already-entered external work keeps its truthful terminal history.
