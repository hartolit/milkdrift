# 02e — Daemon-owned projects and execution environments

**Prepared, not assigned.** The user approved preparing this correction on October 19 after asking
where work executes and how projects relate to Podman. Implement only when this prompt is explicitly
assigned. The outcome is a coherent **daemon → project → workflow** experience with real project
files, consistent execution and one project-level environment choice. Stop at the repaired early
review handoff before 04; 03 remains complete and 07 retains the integrated full-system gate.

Read [AGENTS](../../../../AGENTS.md), canonical product documents in their required order,
[README](README.md), [ADR 0051's project decision](../../../decisions/0051-daemon-task-workflows.md#projects-and-execution-environments),
[journey](journey.md), [code-change map](code-change-map.md), [handoff](handoff.md),
[implementation](../../practices/implementation.md), [documentation](../../practices/documentation.md),
[cohesive review](../../practices/cohesive-review.md) and [workflow](../../workflow.md).
Inspect current source, consumers, fixtures, manifests and Git before editing. Preserve the existing
canvas, left inspector, direct inputs/results, model selection, real brand, shared daemon names,
03's execution behavior and retained stores. This prompt completes a backend and client boundary;
sidebar grouping alone does not satisfy it.

## Make the project the ordinary place to start

Navigation groups projects beneath their owning daemon and workflows beneath their project.
**Add project** offers **Use existing folder** or **Create folder** on the clearly named daemon.
Use a compact bounded folder chooser restricted by current authority; creation selects an allowed
existing parent and one new folder name. Suggest the project label from the folder, with a readable
rename action. Do not require technical root IDs, mount arguments, installation identities or a
second workspace object. The chosen folder is on the daemon's machine, even when the browser is on
a phone. Show that location in project details rather than repeating it on every task.

The first project can be empty of workflows. **New workflow** within a project opens the existing
unsaved canvas; its first explicit submission creates the workflow and tasks together, bound to that
project. **Add task** retains the same canvas and compact left inspector. Keep daemon, project and
workflow context visible during navigation, reconnect and errors. Two projects may have identically
named workflows, and two daemons may have identically named projects, without moving drafts or work
between them. Preserve bounded pagination and drafts keyed to the actual daemon, actor, project and
workflow. Keyboard, narrow-screen and touch users have the same actions.

Project settings show **Folder**, **Execution environment** and effective **Project access**. Select
an available configured host or Podman environment once. An explicit **Allow project changes** choice
can enable the permitted coding tools for subsequent work; the initial setting supplies no write
tools. Task details display inherited choices and meaningful restrictions/overrides without opening
the old path/tool schema editor. Authority and configured tool compatibility still determine which
choices are possible. Changing a project setting must explain that accepted tasks retain their
current definitions. Keep **Model or agent** separate and visible: model inference may be remote
while its tools act on this project on the daemon.

If no compatible environment exists, explain what the daemon operator must configure and preserve
the project and draft. Reuse the Models & agents/setup presentation. This assignment does not add a
provider marketplace, arbitrary image pulls, model installation, browser credentials editor or a
general host administration interface.

## Complete project ownership and recovery

Implement the ADR's project identity, name, pinned folder, scoped settings and guarded generation in
the existing semantic, control, authority and persistence owners. New workflows refer to that record;
the browser is only a projection. CLI and independent public clients must create/list/read/rename,
remove unused registrations, change settings and associate existing workflows through the same
operations. Expose bounded public directory choices and environment compatibility without leaking
inaccessible paths, private profile arguments, secrets or hidden project counts. Resolve filesystem
authority and native identity on the
daemon, including component containment, aliases and replacement races.

New-folder creation is an external filesystem effect. Reserve a bounded exact command/operation and
persist intent before exclusive creation of one child under a pinned permitted parent. Preserve the
created directory identity and original result on acceptance, with the appropriate durability checks.
Do not use recursive creation to adopt existing contents. A lost reply replays the same accepted
result. Crash after creation but before recorded ownership may leave an unresolved operation: show
it, preserve the directory and offer authorized explicit attachment/reconciliation or abandonment
without deletion. Never fabricate atomicity across filesystem and database, retry under a new name,
overwrite existing files or delete a directory whose ownership cannot be proved. Reuse bounded
operation/receipt mechanisms, without introducing a general transaction framework.

Attaching an existing folder changes registration only. Preserve external files and Git state.
Canonical aliases must share conflict detection; a second project label cannot bypass active or
uncertain writers. Project label changes do not rename the folder. Removing an unused registration
preserves the folder and refuses retained workflow/unresolved-operation references. Externally moved,
missing or replaced roots become unavailable for new work. Do not silently recreate or relocate them.
Automated folder moves, project-file deletion and Git lifecycle management are outside this correction.

Adopt current sprint stores through a bounded additive change with tested readers and exact receipt
preservation. Existing workflows remain visible under **Earlier workflows** without an invented root
or new authority. An explicit organizational association selects a project for later submissions;
accepted definitions, inputs, results, path claims and original request receipts remain unchanged.
Old accepted work still uses its recorded tools/paths and displays that fact when it differs from
current project settings. New ordinary workflow creation requires a project; version the public
contract and update generated types, CLI, fixtures and maintained examples together. Historical
request replay retains its original interpretation. Do not reopen retired pre-reset store formats,
reset current stores or reconstruct projects by guessing from task path strings.

## Make every tool use the selected project

Resolve project defaults on the shared daemon command path at acceptance, guarded by the reviewed
settings generation and current actor authority. Preserve the original canonical request for exact
receipt lookup before resolving current defaults. Accepted task definitions contain the effective
binding and permissions; changing defaults affects newly submitted work only. Prospective edits to
queued tasks are explicit, and started tasks retain their frozen selection. Apply the same resolution
to model-authored future work and expose the contract that 04 will use for method expansion. No client
fallback, model prose or template may expand access or silently select another environment.

Use a typed choice between project settings and explicit task settings. The current DTO's default
empty arrays cannot distinguish omission from a deliberate no-tools/no-access choice; preserve that
distinction in the new contract without reinterpreting old requests. Validate partial overrides as
one effective environment/root/access/tool selection. Title/instructions-only edits preserve existing
selections, and deliberate empty permissions stay empty after project defaults change.

Trace one binding from project through model tool context, native file reads/writes, process working
directory and Podman source/destination mount. Current process profiles may fix a different host
working directory; choosing project A while Cargo still runs in B must refuse before effects. Add a
narrow typed project-aware preparation mode where needed, constrained by the registered profile and
current authority. Retain existing fixed-directory and isolated invocation modes for their own uses;
never retarget them by changing labels or injecting uncontrolled argv. Ensure tools know which paths
they accept: a container path and an absolute host path are not interchangeable. Demonstrate actual
file creation, editing and compilation in the selected folder through production adapters, rather
than a fixture-specific profile per project.

Use the existing runtime and native path claims for all Milkdrift-managed writers, across workflows
and projects on the same host. Include process tools that previously depended on their configured directory and
containers with a writable project attachment. Resolve aliases before claiming, bound concurrency and
retain claims while an entered writer is uncertain. Parallel alternatives may use separately prepared
worktrees; creating a second container alone provides no independent source tree. Project defaults
do not change workflow limits, automatic-continuation opt-in, required inputs or completion semantics.

Native claims coordinate declared work; they do not lock out external editors or sandbox a trusted
host process. Count any mutation-capable process/coding agent as a writer even without the native
write-file tool. Read-only
tasks exclude those tools unless the actual execution boundary enforces read-only access. Preserve
that limitation in the UI and guides. Managed workers already provide bounded process capabilities;
test their use from the model tool loop with exact generation/lifetime holds and uncertain-effect
recovery. Generic installation lifecycle tools remain excluded from ordinary model tool selection.

## Add the explicit Podman project attachment

The current worker receives an installation-owned volume at `/workspace`; its verifier rejects
unexpected host mounts. Extend that actual managed owner with an explicit attached-project mode.
Expose the authorized host project at `/project` with the task's permitted access and use it as the
working directory. Keep existing owned-volume workers working under their original contract; any
private tooling/cache volume has separate ownership and cleanup from the attached project.

Carry the exact attachment through recipe/descriptor or invocation preparation as appropriate, final
authority and resource checks, worker creation, actual-mount verification, evidence and restart.
Reject root substitution, undeclared mounts, incompatible access, stale selected generations and
conflicting writers. Retain read-only image roots, private mappings, resource limits, lifetime holds,
generation draining, stop/recovery and uncertainty protections. Engine sockets, manager credentials,
store/configuration and unapproved host paths remain inaccessible. Host-native execution must remain
honestly labelled; selecting Podman cannot silently fall back to a host process.

Preserve the user's ownership, permissions and existing file bytes except for authorized work.
The owned volume's `:U` behavior must not recursively change ownership of a project folder. No silent
relabel, privileged fallback or broad mount is an acceptable substitute for a working supported mapping.
If host prerequisites cannot provide the selected access, refuse with a useful setup explanation.
Removing/reapplying an environment, stopping a worker, deleting a registration and history collection
must preserve the attached directory. Existing owned-volume preservation policies and protected
installation behavior remain intact. This is a new tested attachment boundary, not a blanket exception
to current mount verification.

## Preserve the remaining sprint boundaries

This correction owns projects and execution on their owning daemon. Phase 04 adds methods using the
same project resolution and permission rules. Phase 05 retains directional execution-host connections
and remote recovery: an explicit remote tool needs its own authorized working location or transferred
input/output artifacts. It does not inherit the originating host's absolute path or obtain shared
storage by connection. Do not add remote checkout, automatic synchronization, network mounts, new
provider families, inference, a second scheduler or another environment registry in this correction.

## Verify the real project journey

Use fresh named temporary stores and disposable folders with controlled Rust adapters. The actual
daemon owns registration, resolution, execution and recovery. Browser actions must not seed project
records or manually orchestrate workflow progress. Establish these outcomes:

- Attach a folder with existing source and create a second project in a permitted parent. Create two
  workflows under one project and one under the other. Switch daemon/project/workflow, close the
  client and restart/reconnect from a fresh client; grouping, settings and real files persist.
- A model with selected tools creates a small Rust project, edits a source file and builds/tests it
  in the selected project. Run equivalent work in two distinct folders using the same supported
  environment binding. Inspect actual files and results; a text response claiming creation is not
  evidence. Controlled model responses prove tool plumbing; a configured real model is separate
  quality evidence and requires existing authorization.
- Host and Podman paths reach the intended folder. Edits are visible from the host, and rebuilding
  the environment preserves them. Verify UID/GID, modes and existing content. Native file tools and
  read-only container attachments deny writes; read-only tasks refuse unconfined mutation-capable
  host processes. A removed environment or registration preserves the folder. Exact owned-volume
  and protected-installation regressions still pass.
- Reject outside roots, traversal, symlink/root replacement, hidden project discovery, incompatible
  fixed process directories, permission loss, stale settings and environment generations. Race an
  environment/settings edit against submission/start; accepted provenance must remain explainable.
  Explicit empty tools/access, partial overrides and title-only edits after defaults change retain
  their intended meaning. A model's managed worker tool preserves resource holds through uncertainty.
- Competing workflows through host and container tools cannot bypass shared or alias path claims.
  Interrupt an entered writer; restart retains uncertainty and blocks conflicting writes. Separate
  prepared worktrees permit the intended parallel alternative without automatic Git operations.
- Interrupt create before/after the directory effect and before/after durable acceptance. Exact retry
  never creates another folder, changed-key reuse conflicts, and unresolved ownership remains visible
  until explicit reconciliation. Capacity refusal leaves existing operations recoverable.
- Open an existing 02d store; discover older workflows, associate one deliberately, and replay its
  original commands before and after settings changes. Accepted tasks/history remain unchanged.
  New tasks resolve current reviewed defaults through browser, CLI and model admission alike.
- Keyboard and narrow-screen users complete project setup and task creation without protocol forms.
  Every created task clearly shows its model/agent, project and effective execution environment.

Run complete affected owner suites, not only new filtered cases: blueprint/control/authority,
runtime durable tasks, persistence/redb task workflows and application state, protocol/client/CLI,
daemon control plane, capability host, local-process execution and managed-linux recipe/lifecycle/
mechanism suites. Run affected direct/peer/protected-resource regressions for shared contract changes.
Use the commands owned by [workflow](../../workflow.md#focused-suites) and actual package manifests;
record discovery and executed counts. Generate/check TypeScript and public APIs where changed, run
scoped warning-denying Clippy, maintained example checks and the actual CLI/daemon journey. Complete
frontend locked install, check, lint, unit, build and production browser suites.

Physical Podman evidence uses only already authorized disposable resources, local approved images
and available tools. No paid pulls, privilege changes or retained installations may be used to obtain
a pass. If unavailable, implement and test the production attachment/refusal mechanisms, retain the
exact physical qualification gap and keep dependent product acceptance open; do not present a mocked
mount assertion as a physical pass. Prompt 07 retains that gap and the full integrated gate.

Update canonical owners, affected guides/reference/examples, current source mapping and evidence to
match implemented behavior. Run documentation contracts and diff checks. Make coherent tested local
commits with explicit staging; preserve history and do not push. Leave a runnable review environment
and invite the user to choose a project, start useful work and locate its actual files. Record only
their observed/reported actions and judgments in the handoff. Stop before 04; automated checks do not
complete their review.
