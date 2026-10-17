# 02a — Connect simply and share the daemon's name

**Implemented after explicit assignment; user recheck pending.** This corrective assignment follows the user's October 17 partial
[early review](handoff.md#actual-user-review-and-corrections). Prompt 03 is already complete; do not
rerun it or discard its work. Complete this correction before [02b](02b-visual-design-and-brand.md),
then resume 02's user journey before 04. The [handoff](handoff.md) records its checks and review setup.

The user connected Studio A but found naming the connection and supplying an installation identity
unnecessarily complicated. A daemon's name must belong to that daemon and be consistent across its
clients. Deliver first connection using an address and credential, automatic identity handling,
and an authorized rename that survives restart and appears in another client.

Read [AGENTS](../../../../AGENTS.md), canonical product docs, [README](README.md),
[ADR 0051](../../../decisions/0051-daemon-task-workflows.md#browser-discovery-and-connections),
[journey](journey.md), [implementation](../../practices/implementation.md),
[documentation](../../practices/documentation.md) and [workflow](../../workflow.md).
Inspect current source, callers, tests and Git before editing. The technical choices below adopt
the ADR's correction; do not ask the user to design the handshake or choose storage internals.

## Complete the shared boundary

The installation identity identifies the daemon, not a connected device. Keep its stable identity
for requests, receipts, cursor scope, artifacts and cache separation. A mutable display name cannot
replace it, prove endpoint trust or grant authority. Browser-to-daemon onboarding is distinct from
05's directional execution-host relationships; preserve those host identity and credential checks.

Use the existing authenticated session bootstrap. On first connection, accept the daemon address
and scoped credential, read its stable identity and current display name, then bind subsequent
requests to that identity. Present the returned name immediately. Do not ask for a local nickname
or an installation ID. First-use trust comes from the operator-supplied endpoint and the existing
transport/authentication policy; a discovered ID is not independent cryptographic verification.
Keep browser-trusted HTTPS and the existing loopback development exception, origin checks,
redirect refusal and in-memory credentials. No discovery service, pairing system or credential
relay is needed.

Persist the learned identity with the endpoint for reconnect. Compare the returned identity before
loading workflows or enabling actions. An endpoint now serving a different installation must stop
ordinary use, clear inaccessible active state and explain the change. Let the user deliberately
forget the saved connection and connect afresh; never silently overwrite a pin or carry old drafts,
requests, selections or results into the replacement. Advanced connection details may expose the
identity with an explanation, but it is not a required ordinary form field.

Store one bounded display name on the daemon, with a generation and exact mutation receipts.
Initial operator configuration supplies a seed when metadata is absent; subsequent startup must
not overwrite an accepted rename. Existing current-format stores get metadata without resetting
workflows, changing stable identity or losing replay. Use `Milkdrift` when no initial name was
configured; test that path without inventing a hostname-based identity. Validate
the name once in its Rust owner: trim surrounding whitespace, require 1–80 Unicode scalar values,
and refuse control characters. Duplicate names across installations are allowed; the selector's
secondary endpoint distinguishes them. Names never become keys or authority.

Expose the current name and generation through the public session/settings reads. Implement a narrow
daemon-administration rename command through the shared authority and durable owner path, with exact
request replay, generation conflict and read-only refusal. CLI and Svelte must use that same command.
Keep it separate from permission to edit a workflow. The settings UI states that the change affects
every client of this daemon. A lost reply offers receipt lookup or exact replay, not another rename.

Refresh another authenticated client's name through bounded daemon observation/invalidation and
session refresh, including a client showing no workflows. Reconnect and a fresh browser must read
the accepted name. A disconnected client's last observed name is explicitly cached, not a live
claim. Retain local endpoints, identity pins and layout preferences only as client concerns.
Adopt existing saved profiles without losing their pins; replace browser-authored labels with the
server's name on authentication and remove local renaming as a competing authority. Do not add a
generic settings framework or change unrelated workflow/host semantics.

## Follow the owners and consumers

Start with `apps/web/src/components/ConnectionForm.svelte`, `apps/web/src/App.svelte`,
`apps/web/src/lib/connections.ts` and `apps/web/src/lib/transport.ts`; follow session construction,
daemon configuration/control, the authority contract and the persistence/redb boundary. The public
Rust DTOs and generated TypeScript, control client, CLI, operator examples and connected-task Rust
fixture must agree. Inspect current store/version readers before deciding whether a format change
is required; document the actual compatibility result, not an assumed migration. Redb currently
checks exact table membership and versions. Add a bounded adoption of the current prompt-03 store
format if new tables require it; preserve its workflow/receipt bytes and test interrupted adoption.
This does not restore obsolete graph-store readers. Keep the metadata module private under the
daemon's existing `Owner` queue and reuse `Owner::authorize`. Add a narrow `DaemonMetadataStore`
port under persistence and implement it in redb; do not create a fake workflow to hold rename receipts.

Add the specific `RenameDaemon` authority operation using the existing `daemon_configuration`
resource fact and configuration scope. Include it in the Controller preset, not Supervisor or
workflow-editor presets. Inspectors may read the name through their session permission. Update
grant/preset revisions and digest fixtures deliberately when their content changes. Metadata and an
actor-scoped exact receipt commit atomically; replay lookup precedes stale-generation rejection.
Bound this small record family to 1,024 accepted rename receipts and 16 KiB of canonical request/result
data per receipt. At capacity refuse new renames truthfully while preserving reads and exact replay;
never evict accepted keys. A separate settings framework or receipt archival feature is not required.

Use the existing two-second invalidation stream to coalesce session metadata refresh alongside
workflow refresh. Apply a checked identity/name generation even if workflow discovery fails, and
refuse late responses after disconnect or replacement. No new event journal is needed. During old
profile adoption, retain endpoint, pin and local selection ID; use the old label only as a disconnected
cached value until authentication. The new name must appear in memory even if saving the cache fails;
report that cache failure separately and never publish the old local label as a rename command.

Update user-facing connection/setup guidance. Describe identity mismatch in terms of connecting to
a different daemon and show a useful next action. Do not put protocol headers, opaque identifiers,
store formats or authority type names in the normal connection flow.

## Prove and hand off

Run affected Rust owner, protocol/client/CLI and daemon tests plus the maintained frontend
check/lint/unit/build and focused production-browser cases. Discover intended tests before counting
them. Cover these outcomes through real public operations:

- A fresh browser connects with address and credential, learns the server name and can submit work.
- Two independent clients see one rename, including reconnect, daemon restart and a fresh profile.
  A second daemon and a second actor retain their own work and permissions.
- Duplicate names remain distinguishable; empty, oversized and control-character names refuse.
  Workflow editors without daemon-administration permission cannot rename the installation.
- Concurrent rename conflicts retain the draft; lost replies replay exactly across restart.
  Configuration seeds do not reverse later renames and current workflow stores remain usable.
- Full receipt capacity still permits exact replay; changed-request conflicts and cross-actor
  receipt access remain refused. Metadata refresh cannot apply stale names after disconnect.
- Existing profile pins survive adoption. Wrong installation, bad credential, denied origin,
  endpoint failure and interrupted bootstrap cannot expose a usable cross-daemon session.
  Ordinary requests remain identity-bound, and secrets never enter browser storage or logs.

Update canonical behavior, protocol/operator docs and narrow verification evidence; run documentation
contracts and diff checks. Make coherent tested local commits. Prompt 07 retains the full integrated
gate. Leave a usable review fixture and exact resume instructions in the single handoff, and ask
the user to repeat connection and cross-client naming during the resumed 02 review. An agent test
does not resolve their usability finding. Stop before 02b unless separately assigned.
