# Daemon control and execution authority

A credential selects an actor and immutable grant revision; it grants no permission by itself.
Configuration schema 14 compiles operation presets, resource selectors, numeric ceilings,
validity and revocation into the authority owner's grant. Current wire versions are recorded in
[the version reference](../reference/versions.md). Changing grant content requires a new revision.

Use the [operator configuration](../../examples/operator/daemon.toml) as a complete starting shape.
Preset names choose typed operations; they do not imply resource access. Workflow creation, edits,
reads, stop, capability entry and artifact reads each check their own operation/resource facts.
The owner rechecks current authority at final external entry, so an accepted queued task cannot
use a grant revoked before it starts. Frozen authority is provenance, not a perpetual permission.

## Workflow and capability scope

Workflow scope can select an exact workflow, a finite set or explicit `any`. A legacy mechanical
run selector refers to the task workflow identity at this boundary; there is no independent graph
run owner. Discovery filters every candidate through the current workflow scope and returns no
hidden count. A cursor is bound to the actor, grant and daemon. Receipt lookup belongs to the
request's actor and rechecks current workflow authority.

Creation assigns unpredictable workflow identities. An operator allowed to create new workflows
currently needs explicitly acknowledged broad workflow scope. Narrow an actor to existing IDs for
later reads/edits when appropriate. `owner_actor` is provenance and does not act as an implicit
private-workflow filter: an actor whose grant covers another workflow may inspect it. Independent
daemons, credentials and roots are separate installations; a browser connection is not an authority
boundary implemented by local storage.

Capability allow scopes are conjunctive across identity, operation, category, provider, trust,
locality, peer and maximum side-effect class. A selector is `any` or a nonempty bounded `only` set;
an empty set never means wildcard. The chosen capability generation and actual execution resources
must fit the grant. Choosing a configured capability in the UI cannot widen it. Missing or refused
selection leaves the task queued with a reason, without an invented start.

## Files, artifacts and external effects

Projects require `InspectProject` to discover/read and `ManageProject` to attach, create, rename,
change settings, associate or remove registrations. Controller includes management; the read-oriented
workflow presets include inspection. Each operation also checks the requested and canonical folder
against current filesystem scope. Registration does not grant access. A person who can inspect a
workflow but cannot inspect its project receives no project path or name through project discovery.
Associating a workflow requires both project management and that workflow's edit authority.

The folder chooser lists only permitted existing directories. Attaching changes no files. Creating
a folder additionally requires write access on its pinned existing parent, reserves an exact durable
operation before exclusive creation, and preserves uncertain results for explicit reconciliation.
Removing a registration never removes its folder and refuses retained workflow references.
An initial project supplies read tools only; enabling project changes requests write access subject
to the grant. Host process tools require that explicit choice because native claims do not confine
a trusted process. A compatible read-only Podman worker can enforce read-only project access.

Adding these explicit operations changes preset grant digests. Adoption keeps old task definitions,
frozen grants and exact receipts unchanged; it does not rewrite an old grant to authorize current
entry. Finish unfinished work with its matching binary/configuration before adopting revised presets,
or retain an honest refused/uncertain state. Changing defaults and organizing earlier workflows do
not retroactively change their accepted tools or paths.

Filesystem roots use canonical absolute paths. Canonical Unix paths and uppercase drive-rooted
Windows paths retain platform-specific meaning; one platform's tests do not establish another's
containment. The host additionally owns durable native path claims for declared task read/write
roots, handles aliases and retains uncertain writers across restart. A path claim coordinates
admitted tasks; it is not a security sandbox for an untrusted executable.

Network profiles/destinations and exact secret references are independent permissions. A capability
allowlist does not grant network or filesystem access. Secret values never belong in task JSON,
URLs, logs or browser storage. Trusted process profiles pin executable identity, argument mapping,
materialized inputs, output bounds and supported lifetime behavior.

Artifact scope limits identities and sensitivities. Task-state inspection does not imply result
content permission: selected task reads expose `result_restricted`, and explicit result/download
routes refuse unauthorized bytes. Uploaded text is actor-owned and immutable; knowing an upload ID
does not permit another actor to bind it. Selected producer output is authorized and frozen before
use. Managed-resource generation, claims and verification remain independent protections.

## Revocation and diagnosis

Disable/rotate credentials for authentication revocation; change grants/revocation generation and
restart with a validated configuration for durable policy changes. Current requests, final entry,
streams and output reads enforce their relevant boundaries. Revoking a credential or requesting
stop does not prove a running external process stopped. Uncertain effects and writer claims remain
uncertain until evidence or an authorized reconciliation resolves them.

Use `milkdrift daemon authority` and capability inspection to diagnose permitted scope. Redacted
errors distinguish failed authentication, refused authorization, missing retained content and
conflicting generations. Do not repair an authority error by editing the store or copying a
credential into another connection. [Daemon operation](daemon.md) covers lifecycle and browser
hosting; [control API](../reference/control-api.md) defines requests and recovery.
