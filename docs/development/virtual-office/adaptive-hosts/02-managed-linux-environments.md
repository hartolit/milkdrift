# 02 — A complete managed Linux working setup

## Assignment and dependency

After 01 is accepted, implement the full managed-resource lifecycle and its first real Linux
mechanism. Read `handoffs/00.md` and `handoffs/01.md`, [the sprint README](README.md),
[the discussion](discussed-direction.md), and [AGENTS.md](../../../../AGENTS.md). Apply
[implementation practice](../../practices/implementation.md),
[documentation practice](../../practices/documentation.md), and the
[verification policy](../../workflow.md).

You own a useful prepared agent environment, persistent services, ownership-aware maintenance and
removal, and their common direct/workflow API. This is not an installer script followed by manual
administration, an empty resource schema, or a process adapter that abandons background children.
The environment must be usable through the independent host delivered in 01.

Use the single [Slotbook specification](../../../guides/adaptive-method-example.md) for working
files, tools, staging, persistent data and later deployment. The protected verifier/effect gate
belongs to 03; do not invent a separate application or claim that protection from setup alone.
[ADR 0039](../../../decisions/0039-managed-resource-ownership.md) owns the adopted lifecycle rules.

## Starting owners and design

Read the current host registry/materialization/execution owners from 01, authority/resource selectors,
persistence/application/artifact/execution ports, redb lifecycle/integrity/backup paths, daemon
configuration and client/API/CLI, and the local-process profile and cleanup contract. At the baseline,
ADRs 0009/0021 distinguish invocation-owned trusted children from isolation; ADRs 0022/0024 explain
non-run state and execution retention. Do not force installed services into either task lifetime or
compacted RPC observations.

Use a narrow managed-resource owner and one adopted Linux adapter. The owner handles approved
configuration, intended transitions, permissions, active use, and inspection. Rootless Podman and
systemd/Quadlet implement the supported platform effects and supervision. Reuse appropriate existing
mechanisms instead of writing a package solver, process supervisor, generic deployment language,
or environment-specific workflow scheduler.

Read the installed-version documentation before generating unit definitions. The authoritative
starting references are [Quadlet](https://docs.podman.io/en/stable/markdown/podman-systemd.unit.5.html)
and [Podman API security](https://docs.podman.io/en/stable/markdown/podman-system-service.1.html#security).
Do not blindly copy options from a newer manual or treat raw Quadlet text as validated configuration.

## Required implementation

### 1. Durable installation and resource state

Define typed, versioned state for an installation and its owned working areas, service/configuration
generations, platform identities, data-preservation rules, published capabilities, active users,
and pending changes. Separate desired/approved state, latest observations, and durable transition
evidence. Operation receipts link to resources; they are not the mutable resource inventory.

Implement prepare/apply, inspect, start/stop, update, and remove. Expected resource versions and exact
request identity guard mutations. Within one store, acceptance of a lifecycle change, relevant use
reservations, and durable intent must be coherent transactions. Persist exact intended resource
identity before creation or replacement. Preserve evidence required for recovery and later removal
even after old execution observation detail is compacted.

An identical reapply preserves source files, useful outputs, documentation, database state, selected
models, and other retained data. It must not restart a healthy service or discard an agent's scratch
changes merely because an installer ran again. Distinguish unchanged setup, drift, pending update,
and an incompatible requested change. Do not call undocumented mutable tools an immutable maintained
generation; expose their provenance and promotion requirements honestly.

Distinguish owned resources, shared prerequisites, and attached external services in storage and API.
An external attachment cannot be stopped or deleted by installation removal. Adoption is a separate
explicit verified decision, not a name match. Installation deletion must not remove shared images,
volumes, toolchains, credentials, or models still needed elsewhere. Decide preservation at creation
and allow deliberate authorized changes; do not guess what data is valuable during cleanup.

### 2. One lifecycle shared by all callers

Expose typed operations through capability invocation and administrative client/API/CLI calls,
routed to the same semantic owner. Apply permission and resource-state checks below transports and
in-process adapters. Users and workflows must not encounter different defaults, drift handling,
use holds, or cleanup rules for the same request.

Creation/update requests identify approved recipes and bounded typed configuration. Validate their
resulting mounts, paths, devices, network exposure, secrets, and resource limits. Reject unexpected
unit directives, engine arguments, path traversal, shell/unit interpolation, or grant expansion.
A friendly operation name does not make arbitrary raw configuration safe. Use a mechanism-specific
adapter internally without leaking unrestricted engine administration as a worker capability.

Keep model services configurable external endpoints. An owned server is managed by the installation;
a model request is an invocation of it, not its lifetime. Existing native trusted profiles and
attached providers continue to operate outside the container setup.

### 3. Resource-use coordination

Integrate use holds with actual accepted/executing operations from 01, including local workflow
attempts whose authoritative record remains in the runtime. Hold the exact resource generation
before maintenance can invalidate accepted work. Rebuild live holds before opening admission on
restart, and release them only under the owning execution's evidence or an explicit resolution.

Allow one mutating operation per working area initially. This must not block unrelated model calls,
other services, or separate worktrees. Account for readers when deleting/replacing their resources.
Acquire multiple resource holds coherently with a defined order/refusal strategy so conflicting
operations cannot deadlock. Bound any queue or wait.

Implement ADR 0039's distinction between generation lifetime holds and exclusive editing claims.
An accepted parent may retain lifetime protection while transferring mutation to its exact
authorized child association. Record suspension/physical quiescence before the guarded resource
transaction transfers the claim; parent writes must refuse until proven child quiescence and
authorized reacquisition. A shared actor/workspace name is not lineage, and a live writable mount
cannot be suspended by a flag alone. Refuse unsupported suspension before child entry.

02 owns transactional handoff/return, claim-generation conflicts, physical stop evidence, restart
and cancellation behavior, and authorized blocker inspection/resolution, using accepted execution
relationships from 01. 04 supplies the published invocation/run association and integrated one-worker
test; no publication stub is needed here. Carry these facts in `handoffs/02.md`.

Updates/removal close new affected admission and refuse busy state or wait under an explicit bounded
policy. Queued accepted work must not be silently invalidated. A timeout, socket close, expired lease,
or a missing response is not proof that entered work stopped using a resource. Provide inspection
of blockers and an authorized disruption/resolution path; retain the old operation's uncertainty
when resource reclamation does not establish its outcome.

Generation identity includes the actual deployed implementation/configuration. Drain before replacing
a service behind its endpoint. When two model versions cannot coexist, support a declared interruption,
not an assumed spare GPU. Publish only verified replacement generations. A failed update must remain
observable and must not advertise the candidate as healthy or rewrite accepted old generation facts.

### 4. Recover interrupted platform changes

Implement recovery at every boundary: durable intent before platform call; platform action performed
but response lost; observed success before result commit; replacement partly activated; removal
partly complete; capability publication interrupted. Inspect recorded identities/configuration and
platform ownership rather than adopting a matching name or PID. A stale callback cannot advance a
newer transition. Resource recreation cannot inherit old evidence solely by reusing a path.

Recovery may finish recording a verified result, resume a specifically safe remaining step, or
compensate when authorized. Otherwise retain a useful partial/uncertain state and refuse conflicting
changes. Preserve data and readable diagnostics. Do not promise database/platform atomicity or
rollback of irreversible data changes. Service recovery restores declared service state; it does
not manufacture results for an earlier model or process invocation.

Extend integrity, offline inspection, backup, and restoration to these new records and owned durable
configuration. Record what installation data is outside a store backup and how it is preserved.
A restored host must not accidentally recreate or seize live platform resources still owned by
another installation. Existing restored-generation execution guards remain effective.

### 5. Real Linux adapter and manager protection

Implement rootless Podman plus systemd/Quadlet integration through a production-composed adapter,
including supported prerequisites, configuration generation, verification, service observation,
activation/draining, restart, and removal. Systemd remains the service supervisor. Ordinary host
shutdown leaves persistent services according to policy; temporary task containers remain owned and
cleaned up by their invocation lifecycle. Bound helper commands, output, timeouts, and cancellation.
Do not detach untracked host children or reconnect using only a recycled PID.

Document and enforce which identity owns the manager and which paths a worker receives. Manager
state, secrets, executable, service definitions, engine/systemd sockets, and policy configuration
must not be writable/reachable by an untrusted worker. Use explicit scoped host APIs for permitted
service changes. Verify actual mounts and access rather than declaring rootless operation sufficient.
Keep useful tool installation possible inside the authorized working boundary, without access to
the supervisor or host account outside it.

Prefer a cohesive installation over many accounts/containers, but introduce a real enforced identity
boundary when needed for the protection claimed. A trusted native process with the manager's host
privileges remains trusted; it is not a sandboxed worker. Do not advertise containment of arbitrary
native code under that same identity. Failure to establish required protections must refuse, never
fall back to privileged containers, full-home mounts, raw sockets, or unrestricted host execution.

Enforce and test declared filesystem, network, device, memory, CPU, and process constraints where
advertised. A configured number or capability label is not proof of OS enforcement. Allow broader
network/tool use only as an explicit profile choice; do not claim destination isolation that the
selected network mechanism cannot enforce. Keep all platform details out of core semantic contracts.

### 6. A maintained setup and operator path

Ship one useful maintained Linux recipe: normal working files and documentation, Git/Rust and basic
build tooling, an agent-capable execution path, and an owned llama-server configuration. Support
operator-supplied exact model/server inputs and an attached endpoint alternative. Basic bootstrap
must not depend on a model. Pin maintained inputs and record mutable/unknown facts honestly; do not
resolve floating image/tool aliases silently for an already accepted generation.

Provide an inspectable repeatable bootstrap command plus preview, verification, diagnosis, and
removal through the product's normal path. An auxiliary bootstrap script may install OS prerequisites
and the product service; lifecycle semantics after startup belong to the production resource owner.
Confine explicit elevation to bootstrap. Never silently overwrite a user's existing account, SSH,
NetBird, firewall, units, data root, model server, or database. Use generated bounded configuration
rather than asking the operator to manually edit many files.

Detect the real Linux/systemd/user-session/cgroup/rootless prerequisites and available storage and
memory. The UM790 has user-reported 64 GB shared memory and a large iGPU reservation; do not count
those as separate additive memory pools. Do not change firmware reservations. Record actual device
and inference backend; CPU success is not GPU proof. GPU enablement is an explicit supported profile
with real verification, not a CUDA assumption. Other platforms retain working portable behavior and
truthfully refuse the Linux-only setup mechanism.

## Acceptance tests

Use deterministic platform fault injection through the same adapter contract and actual Linux
mechanism tests. Both are necessary; a mock container state map cannot establish isolation.

- Reapply the same setup after modifying retained working files; data and valid service identity
  survive. Install/use a tool in the authorized workspace; an unrelated model call still proceeds.
- Start an owned service, complete the start operation, disconnect the client, restart Milkdrift,
  and reconnect without creating another service. Test declared supervisor behavior independently
  of Milkdrift's availability. A real machine reboot belongs in physical qualification.
- Crash at each recorded platform boundary, including success-before-result-commit; retain one owned
  resource or explicit uncertainty, with inspectable repair. Inject mismatched identity, conflicting
  versions, and stale callbacks; refuse unsafe adoption or state advancement.
- Race mutation/read work with update/removal; respect per-resource holds across restart. Exercise
  pre-entry cancellation, entered uncertainty, bounded busy behavior, and authorized resolution.
- Prove the resource portion of the shared parent/child test: during child mutation, parent and
  unrelated conflicting writes refuse; retained lifetime protection prevents removal/replacement;
  other resources progress. Proven child completion returns parent editing without simultaneous
  writers or leaked holds. Interrupt at handoff and during child use; exact lineage and claims
  survive, and uncertain use keeps writes/deletion blocked. 04 composes this with a single worker
  and a published child; 06 reruns that integrated case. Timeout/acknowledgement is not stop evidence.
- Fail an update's verification, test declared interruption when generations cannot coexist, and
  prove old accepted work is not silently served by replacement bytes.
- Compact operation detail, then inspect and remove its still-owned installation correctly. Backup/
  restore must preserve resource relationships and refuse unsafe activation of duplicated ownership.
- Attempt manager-file, socket, credential, unit-path, traversal/symlink, privilege, and unapproved
  network/device access from a worker. Assert actual denial for claimed protections. Include a
  positive permitted tool/service change through the scoped API.
- Remove the disposable setup with a preservation plan while an attached service and an unrelated
  sentinel installation remain intact. No global prune or broad recursive deletion is acceptable.

Extend appropriate persistence/redb, host, daemon, process/shared-adapter conformance, client/CLI,
repository-contract, and platform suites. Every new adapter must be used by a production composition
root and pass a common contract plus mechanism-specific tests. Run the full gate. When Linux/systemd
is unavailable, finish the implementation and deterministic tests, retain a runnable real-host test
lane and explicit pending qualification; do not report a mock as completed physical acceptance.

## Completion and handoff

Stop only when prepare/use/inspect/update/recover/remove is a complete product path for the maintained
setup. A deeper ownership or cleanup defect is part of this assignment, not optional hardening for
06. Update architecture/operations/status, exact examples and CLI parsing tests, platform support,
retention/backup limits, and API docs. Protected publication policy is added in 03; this assignment
must already expose precise immutable candidates and resource operations it can govern.

Write `handoffs/02.md` with public commands, recipe/protection facts, retention behavior, exact
transaction and recovery rules, changed contracts, actual mechanism-test evidence, and unexecuted
physical checks. Do not declare resource management complete based solely on installing Podman.
