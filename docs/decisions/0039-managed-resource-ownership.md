# 0039 — Managed resources retain lifetime protection and transferable editing ownership

- Status: accepted direction; resource implementation in adaptive-hosts 02; physical qualification pending; published child integration to 04
- Date: 2026-09-18
- Extends: [0010](0010-host-owned-materialization.md), [0022](0022-redb-owned-daemon-application-state.md), [0024](0024-peer-execution-hot-retention-and-tombstones.md)
- Preserves: trusted-process limits in [0021](0021-byte-pinned-trusted-host-processes.md)

## Context and owner

An installation survives the invocation that created it. Its configuration, physical identity and
removal obligations cannot live only in compactable execution observations. A parent also needs
to protect a working area while delegating edits to a child; retaining an exclusive writer while
waiting would prevent that child from progressing even if a worker thread were available.

Capability-host owns a narrow managed-resource module: approved recipe/configuration, lifecycle
decisions, use admission, intended transitions, recovery and inspection. Persistence defines typed
resource/use/change records and transactional actions; redb owns their physical commits. The Linux
adapter implements platform effects and observation. Runtime supplies exact local attempt/link
facts to these ports; it does not implement containers. Daemon composes the same owner for admin
commands, direct invocations and workflows. No new scheduler or crate per resource noun is needed.

Rootless Podman and systemd/Quadlet are the first mechanism. Quadlet generates systemd units and
supports user services; systemd supervises the service independently of its creating invocation.
See the [Quadlet manual](https://docs.podman.io/en/stable/markdown/podman-systemd.unit.5.html).
The [Podman service API](https://docs.podman.io/en/stable/markdown/podman-system-service.1.html#security)
grants the service user's full engine access; it is not a scoped worker interface. Keep its socket,
manager files, credentials, units and configuration outside worker access. Verify installed versions
and actual OS enforcement in 02; this choice establishes no isolation or hardware qualification.

## Durable lifecycle

An installation binds an owner, approved configuration revision, exact resource generations and
platform identities, owned versus attached/shared classification, preservation choices, capability
generations, observations and pending changes. Mutable files are working data, not proof of a
reproducible maintained generation. Verified changes publish a new generation; old accepted work
cannot silently reach replacement bytes at the same URL. Reapply with identical approved input is
idempotent and preserves working files and a healthy service. Drift is visible, not overwritten.

Each change carries canonical request identity, expected resource version, approved configuration
digest, intended physical identity, transition generation, and a bounded step/evidence record.
The resource owner plans; persistence atomically accepts the guarded change, closes affected
admission and records intent before platform effects. Platform observation and outcome recording
are later transactions guarded by that exact transition generation. No transaction spans redb,
systemd and the filesystem. Multi-resource acquisition is one same-store all-or-none transaction
in stable identity order, refusing conflicts or using a bounded wait; never hold a partial set
while awaiting another resource.

Three required traces determine recovery:

- **Creation succeeds, result commit fails:** the saved intent names the resource before creation.
  Recovery inspects its exact platform identity, ownership marker, approved configuration and service
  state. Matching evidence permits recording completion without another create. A name/PID match
  alone does not. Ambiguity remains partial/uncertain; compensation requires separate authority.
- **Update races with accepted use:** acceptance holds the exact generation; maintenance closes new
  affected admission using the expected version, then refuses busy or waits within its bound. It
  cannot invalidate an accepted hold. Services that cannot coexist use an explicitly drained
  interruption, not substitution beneath old work. Stale results cannot advance the newer change.
- **Removal detail is compacted:** resource removal retains a tombstone with identity, terminal
  generation, ownership proof, preserved-data disposition, change identity and evidence needed for
  replay/conflict. Live resources retain their full removal inventory. Execution-detail archival
  neither deletes these facts nor core artifacts. Attached services and shared prerequisites remain
  outside deletion ownership. Restored copies cannot activate duplicate live ownership silently.

## Lifetime protection versus mutation

A lifetime hold prevents removal/replacement of an exact resource generation while accepted work
depends on it, including read use. An editing claim identifies the sole active mutator. These are
different facts in the same resource owner. Holds bind an authoritative local attempt or serving
acceptance; acquire them in that owner's acceptance/entry transaction before admitting use.
Existing journal/account transactions gain resource actions instead of a duplicate local ledger.
Across hosts, durable acknowledged acceptance supplies the local use obligation; there is no
distributed atomic transaction. Unknown work retains protection after worker lease expiry.

Use **explicit handoff**, not actor/workspace-name reentrancy, for nested edits. The accepted
parent/child link from [0041](0041-published-method-invocation.md), exact resource generation,
authority inheritance and expected claim generation authorize a transfer. Parent lifetime protection
remains; mutation passes through these guarded states:

1. The parent reaches a durable suspension boundary and releases its physical writer. Record the
   pending exact child association and transfer intent; refuse further parent entry/writes. A parent
   process holding an unrestricted writable mount cannot safely suspend by assertion: its writer
   must exit and be observed stopped, or an enforcing adapter must revoke access and prove quiescence.
   Unsupported suspension refuses the nested editing call before child mutation.
2. After that proof, one resource transaction compares the parent claim, generation and accepted
   link, replaces the parent editing claim with the exact child's claim, and retains lifetime holds
   for both dependencies. The child can start only after this commit and linkage validation. No
   writer exists in an interrupted transfer gap; a second caller cannot adopt the pending claim.
3. Child terminal workflow state alone is insufficient. After durable adapter evidence proves all
   child writers stopped and outputs settled, the owner transaction releases the child claim/hold
   and returns editing eligibility to the suspended parent. The parent rechecks current authority,
   generation, cancellation and maintenance state before reacquiring a new claim and resuming. A
   cancelled parent does not resume; its hold is released only after child use is resolved.

An exact accepted link can be reserved before the internal run is created, using 0041's stable
command association. The handoff transaction refers to that association, never a guessed child
name. On one store it atomically updates resource claims and handoff state. Runtime commands own
parent suspension and child creation separately with exact idempotent links; recovery may complete
the association but cannot grant mutation while either proof is missing. On another host, the
resource-owning host validates acknowledged linked acceptance and remains the sole claim owner.

Rebuild use and transfer obligations with admission closed on restart. Before transfer commit the
parent is suspended; after commit the child owns mutation; after proven child quiescence a guarded
return is replayable. Missing evidence at any point blocks writes/removal. Timeout, cancellation
acknowledgement, lost connection or lease expiry does not establish quiescence. There is never more
than one active mutator, but other resources and unrelated model calls can proceed.

Expose blockers, lineage, exact claims, last evidence and pending transition through the existing
authorized command/read plane. Extend its retained-work resolution approach with resource-specific
inspection and resolution in 02; the implemented typed operations share the resource owner. An authorized operator
may supply validated stop/absence evidence, resume the exact safe step, or explicitly fence/disrupt
the owned resource and record the deviation. A risk acknowledgement without physical fencing cannot
enable another writer. Resolution can preserve an invocation's unknown outcome while proving that
its resource use ended. Offline inspection/preservation remains available if startup is blocked.

## Compatibility, acceptance and alternatives

New resource records and journal actions need versioned readers, integrity/backup coverage and
hand-reviewed fixtures in 02; 04 adds exact published-child linkage. The implementation uses managed schema 2, physical redb schema 14/internal document format 19,
and daemon configuration 12. Older stores/configurations refuse before mutation; no migration is supplied. Preserve supported history and refuse incompatible
stores before mutation as in [0038](0038-independent-host-execution.md).

02 tests transfer transitions, conflicting writers, uncertain use and fault recovery using real
accepted execution relationships; 04 implements the integrated single-worker published parent/child
case, and 06 reruns it. During child progress, parent/unrelated conflicting writes and removal must
refuse, work on another resource must progress, and proven child completion must permit parent
resumption without a duplicate or leaked hold. Interrupt both the transfer and active-child stages.
This is a required design test, not a finding that today's implementation already deadlocks.

Task receipts alone cannot own persistent resources. A shared reentrant lock cannot prove lineage
or physical quiescence. One operation per host would hide the problem by disabling useful concurrency.
Reconsider the initial one-mutator-per-area rule only with a concrete consumer and equivalent
generation, isolation, recovery and deletion evidence. External supervision remains the mechanism;
Milkdrift does not acquire a service supervisor or generic installation language.

## Implemented mechanism and format boundary

The adapter accepts strict workload-independent Linux recipes, preloaded exact images and typed
server/model inputs. Podman 5.4..6.x is the accepted mechanism range, with physical evidence on
6.1.2. The installed Quadlet generator validates owned definitions; an exact systemd drop-in replaces
cleanup by name with cleanup by the service-created container ID. Actual enforcement is checked
during apply; deterministic fault maps do not qualify it. See
[operations](../operations/managed-linux.md) for the physical lane.

Recipe schema 2 and mechanism `linux-quadlet-v4` replace the initial recipe's application-family
and coupled-budget assumptions. Worker and owned model receive separate typed container limits;
there is no aggregate memory reservation. Preflight sums simultaneous demand and accounts for the
saved current service's anonymous resident memory on reapply/update. The semantic owner supplies
that generation to diagnosis; a candidate cannot claim an existing allocation by matching a name.
An authorized prepare observation returns `unprepared` with bounded diagnostics when qualification
fails. This adds a response state to the current managed response contract without changing saved
requests, receipts or inventory meanings. Clients upgrade with the workspace.

Owned model identity is an explicit single alias, used by arguments, verification and the existing
provider profile. Exact image/model bytes remain separate provenance. The recipe reuses provider
`EndpointLimits`; operator-selected worker and service deadlines replace workload-specific hard
ceilings. Linux/OCI numeric representations and bounded input/output remain enforced. Application
content and tool environment belong to the approved image and ordinary worker initialization. The
maintained Slotbook image binds its brief and preserves edited files on repeated initialization.
There is no recipe interpreter, raw engine-argument escape hatch, or second provisioning owner.

Older recipe/mechanism generations explicitly refuse, with no silent inventory rewrite or adoption.
Their matching binary retains the preservation/removal path and offline inventory remains
inspectable. Daemon schema 12 still requires `systemd_directory` alongside `quadlet_directory`;
its shape did not change in this correction. The portable managed request/inventory formats and
redb physical/document formats remain unchanged. Worker claims use rootless auto user namespaces,
one owned working mount and explicit cgroup/network/device restrictions.

Managed inventory, receipts, step evidence and use/link indexes are part of the existing redb store.
No local journal event schema changes were needed: acquisition consumes exact scheduled snapshots,
entry decisions, terminal facts and attached-subworkflow association events already owned by runtime.
A return grants a new physical entry claim only to a still-live parent under its current inherited
basis. Explicit resolution commits fencing before external cleanup and preserves the old outcome.
The new portable request/inventory documents use version 1; accepted exact configuration is retained
independently of the approved recipe source files and serving-detail archival.
