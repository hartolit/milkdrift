# 0041 — Published methods bind a starting revision and recoverable service invocation

- Status: accepted direction; implementation assigned to adaptive-hosts 04
- Date: 2026-09-18
- Extends: [0013](0013-immutable-proposal-revisions.md), [0022](0022-redb-owned-daemon-application-state.md), [0027](0027-controller-final-entry-reservations.md), [0038](0038-independent-host-execution.md), [0040](0040-protected-adaptive-methods.md)
- Refines: [0019](0019-frozen-execution-authority.md) for explicit published service calls; ordinary child authority inheritance remains unchanged

## Version and owner

A published method is an exact starting blueprint plus a governing agreement, adaptation policy,
public interface, documentation, execution constraints, and explicit service-execution policy.
Legal per-invocation adaptation can yield different actual revision lineages. New promotion changes
future selection; accepted calls retain their version, agreement, starting revision and history.
Retirement closes new admission without deleting accepted definitions or links.

Control owns publication validation, promotion/retirement and orchestration of the linked run.
Persistence owns immutable publication records and narrow invocation/run association actions, with
redb transactions. Definitions remain in the blueprint/revision owner. Host registry advertises the
version and consumes a host-owned durable-continuation port implemented by control; control already
depends on host, so host must not import control to call the method. Daemon injects that implementation
only in its workflow-enabled role. Execution-only hosts can consume remote publications but cannot
advertise a locally implemented method. No CLI subprocess wrapper or second graph store is introduced.

## Invoke-only service authority

Publication records bind a configured service principal and exact grant/policy revisions, not the
publisher's ambient identity. The public operation names allowed input mappings, target/resource
sets, delegated internal operations, required evidence, output disclosure and budgets. Authority
separately checks the caller's exact invocation and permitted inputs, then derives the internal basis
from that explicit service relationship. It records both the caller/delegation and service identity.
It does not compute a broad union of their grants or require callers to own raw internal privileges.

For example, an invoke-only caller may select the published deployment of Slotbook to its allowed
test target. The service identity may verify and deploy there using a host-held secret; the caller
cannot read the secret, choose an arbitrary path/target/callback, edit the method or administer the
host. The input mapping admits only declared product parameters and authorized exact artifacts.
Internal tasks remain within the service grant, publication constraints and inherited prohibitions,
agreement and limits. A delegated parent must be allowed to use that service relationship; the
operation cannot erase its resource restrictions or governing obligations. Capability expansion is
explicitly bounded service execution, never silent widening of an ordinary subworkflow's grant.

Invocation, public-result reads, internal inspection, method editing, agreement changes, publication
and resource administration remain separate permissions through the same command/read plane.
Delegated credentials retain 0038's origin/account binding across direct and peer routes. A copied
public operation name cannot reclassify a worker as independent. Check current revocation before
future entry and disclosure while preserving accepted authority facts for replay.

## Acceptance to internal run

Use a **stable command association** because runtime already owns its transaction. Before any
internal creation, the authoritative public operation record saves the exact publication, canonical
inputs, service basis, allowance relationship, reserved child run identity and canonical create/start
command identities. For a local workflow caller this is a runtime-owned pending effect/link fact;
for a direct or incoming peer caller it is part of the serving execution record. There is no added
standalone execution journal for the local attempt. Planned child identity denotes real work to be
created, not a fabricated workflow for ordinary direct process/model calls.

Control submits those exact commands to runtime. Runtime atomically creates the run, inputs,
agreement/authority/account bindings and creation receipt; starting it is a separately replayable
command under its existing owner. The association is then advanced from planned to linked using
the exact runtime receipt. A conflicting pre-existing run refuses; it is never adopted by name.
Acceptance acknowledges the durable planned association, not successful internal creation.

Crash before creation replays the saved create command. Crash after creation or start but before
association advancement looks up/replays the same receipts and completes that association. Crash
after completion recovers outputs from the same run. No case allocates a replacement run or reselects
a new published version. If a now-revoked service cannot start, retain a refused/unresolved invocation
as appropriate; do not invent a successful child. Linkage, pending cancellations, allowance transfers
and resource use survive detail archival as long as their obligations do, and compact summaries
retain permanent exact replay/conflict identity.

## Nonblocking continuation and resource handoff

Once the internal run can proceed, the host returns a durable pending outcome and releases its
execution thread and transient entry permit. The acceptance retains a durable generation/version
pin, result observation cursor and resource lifetime holds independently of those worker slots.
Runtime's existing scheduler owns internal tasks; bounded maintenance pages advance observations and
public results. A waiting call does not count against the execution capacity its child requires.
Outstanding invocations, nesting depth and retained observations still have independent bounds.
Refuse recursive/circular publication chains by exact accepted ancestry and depth before child
creation; no unbounded thread per call or second scheduler is permitted.

For a parent and child editing the same managed working area, worker release is only half the rule.
Use [0039's explicit handoff](0039-managed-resource-ownership.md#lifetime-protection-versus-mutation):
bind suspended parent, accepted child association, resource generation and inherited authority;
prove the old writer quiescent; atomically transfer the editing claim; retain lifetime protection.
The child enters only after linkage and handoff are durable. Parent writes remain refused until
all child writers are proven stopped and a new parent claim is authorized. Exact child identity,
claim generations and interruption evidence stay inspectable after restart. An active parent
process with a writable mount cannot be made safe merely by releasing its Milkdrift worker slot.

Cancellation records one exact public request and a stable linked runtime cancellation command.
Acknowledgement reports acceptance of that request, not child termination. Parent timeout or
disconnect neither duplicates nor proves termination of the child. Restart resumes bounded observation
or preserves uncertainty and resource holds. Public success requires the internal accepted output and
agreement satisfaction; failed result publication does not rerun an already completed method.

## Accounts and settlement

Persistence's existing account owner owns the delegation/reservation relation. Before internal work
enters, reserve its enforceable envelope from any applicable caller account. On the same host,
ordinary children still inherit their exact account; service calls additionally bind the constrained
service identity and a sub-allowance to the parent reservation. Child use consumes that reservation's
allowance and settles once, rather than charging both an aggregate call and every child again to the
same account. Service-wide ceilings can independently constrain the same work; overlapping references
to one account must not double-charge it. Across hosts, 0038's acknowledged transfer/settlement owns
the relationship instead of a shared counter. This extends existing account transitions rather
than creating a publication budget ledger.

Entry, artifact publication, cancellation, late settlement, retries and restart retain exact transfer
identities. Reserve supported bounds before entry; unknown metering refuses a hard bound or retains
unresolved allowance. Elapsed time or a new internal run never resets a cumulative account. Models
with explicitly unbilled policy still meter tokens and invocations; arbitrary process-internal
model calls remain unqualified unless actually mediated. A public result reports what is known.

## Compatibility, evidence and alternatives

04 introduces versioned publication and link forms and reviews changed authority, runtime event,
account, serving record, configuration and client/peer DTO families against 01–03's actual readers.
Use the existing exact-current storage refusal/preservation policy; no migration or numeric future
version is invented here. Accepted versions remain exact within supported generations. Old ordinary
run history is never relabeled as a published invocation. Implement the new relationship across
all readers, integrity scans, backup, conformance, clients and recovery before advertising it.

Tests must observe one real internal run across loss before/after create/start/link, exact replay
after archival, narrow invocation beside denied administration, and positive adaptation beside
effect-bypass refusal. With a single execution worker and the same working area, the child must
progress while parent/conflicting writes refuse; unrelated resources must progress. After proven
child completion the parent regains editing without duplicate work or a leaked hold. Repeat with
interruption during handoff and active child use, cancellation and missing stop evidence. 02 owns
resource transition evidence, 04 owns this integrated test, and 06 reruns it through product binaries.

A frozen graph per version would unnecessarily forbid contracted adaptation. Ambient publisher
rights create a confused deputy. Blocking on the child consumes its own capacity, and a separate
local journal duplicates runtime truth. Reconsider these choices only for a concrete unsupported
service composition with equivalent exact linkage, scope, accounting and quiescence evidence.
