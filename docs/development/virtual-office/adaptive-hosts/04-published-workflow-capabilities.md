# 04 — Publish adaptable workflows as dependable capabilities

## Assignment and dependency

After 03 is accepted, make its governed adaptable method callable as a versioned capability through
normal Milkdrift clients and peers. Read accepted handoffs 00–03, [the sprint README](README.md),
[the discussion](discussed-direction.md), and [AGENTS.md](../../../../AGENTS.md). Apply
[implementation practice](../../practices/implementation.md),
[documentation practice](../../practices/documentation.md), and the
[verification policy](../../workflow.md).

The caller selects a published version, submits its inputs, and receives durable acceptance and
observable results without copying or controlling the internal workflow. The publishing host owns
one linked internal run and its resources. Permitted per-run adaptation remains possible under the
selected agreement. This is not a shell wrapper that starts a CLI process, a new orchestration
service, or a synchronous wrapper that deadlocks waiting for its own workers.

## Existing owners to inspect

Trace the current capability descriptor/registry/generation lifecycle, direct/peer acceptance and
prepared entry from 01, ordinary workflow start/control/proposal/recovery, agreement and effect
protection from 03, controller account reservation/settlement, artifact transfer/inspection, and
client/API/CLI authoring. Read current subworkflow execution and peer tests as distinct existing
cases: a published workflow is not automatically equivalent to a nested local subworkflow or a
remote model call.

Use explicit ownership decisions from 00. Keep workflow definitions and internal run truth with
their current owners; capability publication must not become an alternative graph store, runtime,
credential store, or event journal. If current adapter/worker interfaces cannot represent durable
pending workflow-backed work without blocking, repair that shared boundary completely rather than
adding a special untracked thread or a fake terminal event.

## Required implementation

### 1. Exact published versions

Implement durable publication of a validated method: exact starting blueprint revision, public
input/output meaning, governing agreement, permitted adaptation policy, documentation, execution
constraints, and the service authority/limits needed to run it. Register a real capability descriptor
through the ordinary host. Validation must prove that its referenced owners and required supported
behavior exist; a publication record without an invocable production path is incomplete.

A caller accepts an exact published version/generation. The internal run may adopt permitted new
revisions under the version's adaptation rules. Record that actual lineage; do not silently replace
the starting definition or accepted obligations with a newer library version. Changing the reusable
method creates another published generation/version even when the public interface schema stays
compatible. Keep interface compatibility distinct from implementation identity.

Implement create/publish, inspect, list/discover, retire/drain, and replace/promote behavior with
exact permissions and expected-version conflicts. Withdrawal blocks new calls while preserving
accepted invocations and their definitions/evidence. Existing accepted work must remain accountable
after catalog renewal, restart, retirement, or a newer publication. Unsupported unavailable
implementations produce an explicit disposition, not fallback to another version.

Only workflow-enabled hosts can own/invoke the internal implementation as a published provider.
Execution-only hosts can serve ordinary operations and consume remote capabilities; they must not
silently construct a scheduler because a configured publication requires one. Explain role errors
at configuration/admission rather than advertising unusable capabilities.

### 2. One recoverable internal run per accepted invocation

Bind an accepted public invocation to its exact internal run durably. Within a shared store use the
appropriate owning transaction, or a reviewed stable command/receipt association when runtime
acceptance has its own transaction. Handle a crash before child creation, after child creation but
before linkage/result commit, during adaptation, and after terminal outcome. Recovery must discover
or complete the exact association; it must not start another run because the client missed a reply.

Local and remote callers use the same publication owner. A local workflow keeps its own attempt
history and the linked internal run keeps its actual workflow history; do not add a redundant
standalone ledger as a second authority for that local attempt. A direct/remote serving owner retains
the accepted public operation. The linking fact does not authorize either owner to rewrite the other.

Distinguish acceptance, internal running/waiting state, internal terminal outcome, agreement
satisfaction, published result, and persistent deployed resource. A terminal internal run that failed
its agreement cannot yield a successful published result. Failure to publish or record results
must preserve truthful uncertainty. Exact replay and archived summaries retain enough linkage to
avoid duplicate work even after detailed observations have been compacted.

### 3. Continuation without worker starvation

Implement pending workflow-backed operations through owned durable continuation/observation. Do not
hold a scarce execution thread while waiting for internal tasks that need that same pool. Keep
accepted invocation/generation/resource ownership distinct from a thread slot. Use bounded queues,
paged observations, and owned lifecycle handles; no unbounded per-call background task farm.

Prove behavior with one execution worker and a workflow-backed capability that performs ordinary
process/model work. It must progress, report, cancel, and recover. Bound nesting, outstanding calls,
recursive publications, and cumulative usage. Detect or refuse unsupported self-calls/call cycles
rather than hanging or recursively creating unlimited runs. This must remain one workflow engine,
not an environment/publication-specific scheduler.

Cancellation has an exact public request and a linked internal control action. Propagate it through
normal ownership with separate acknowledgement and terminal evidence. Caller disconnect or timeout
does not itself cancel or duplicate remote work. Revocation constrains future entry and disclosure
without rewriting accepted effects. Shutdown and restart recover the link, remaining holds, and
observations; do not announce clean completion while internal work is unowned.

### 4. Constrained service authority and inherited accounting

Implement the useful service boundary: an invoke-only user may request the published deployment
without acquiring production credentials, permission to edit the graph, or unrestricted access to
every internal capability. Publication explicitly binds a service-execution identity and immutable
policy that permits the narrowly described operation on its allowed resources. This is a reviewed
service authority relationship, not an accidental union of publisher and caller grants.

Check the caller's right to invoke the exact operation and supply its particular inputs. Enforce
public input/target constraints, governing obligations, service grant limits, current revocation,
and any delegated caller limits on internal work. The internal execution cannot accept arbitrary
paths, targets, credentials, callbacks, context, or proposed graph changes that turn it into an
administrative proxy. Do not allow a workflow-owned worker to launder its restricted work through
a supposedly independent direct call.

Preserve applicable cumulative accounts across local/remote published calls, descendant creation,
retries, revisions, and restart. Define parent reservation, accepted internal allowance, and settlement
identity. Charge actual attributable work once to an account, while separately enforcing legitimate
service-wide ceilings; do not add the same observation twice or let starting a service run reset an
allowance. Reserve a conservative supported envelope before work enters, retain unresolved use on
loss, and refuse unsupported bounded-account guarantees rather than treating unknown as zero.
Keep per-invocation authority distinct from resource admission.

For local unbilled models, monetary spend can be non-applicable while tokens, invocations, artifacts,
and process limits still apply. Do not infer “free” from a loopback URL or require cloud billing to
test the product. Process-internal external calls remain outside direct-model accounting unless
actually mediated and measured; document and constrain that limitation for the advertised contract.

### 5. Public outputs and authorized shared administration

Map public inputs to declared internal inputs and internal accepted outputs to the advertised result.
Authorize and bound artifact transfer and context selection. Public output cannot leak internal
workspace state, verifier secrets, or unrelated artifacts. Caller-supplied artifact references
need actual read authority and validated bytes, not trust by checksum alone.

Expose publication discovery/invocation/status/cancel and authorized internal inspection/editing
through the same client/API/CLI. Keep invocation, result reading, internal inspection, blueprint
editing, agreement change, publication, and resource administration separately grantable. An
invoke-only caller can inspect its public receipt and permitted outputs, not arbitrary internal
history. An authorized colleague can address the owning host and inspect/edit through ordinary
control, without copying the database or using a special privileged API.

Keep multi-host identities, results, and errors attributable to their actual owner. Host selection
must not silently switch credentials or destination after acceptance. A GUI is out of scope, but
all the behavior it would need must be reachable through the existing supported command/query path.

## Product scenario and verification

Publish the adaptable deployment method from 03. Invoke it directly and from an outer exploratory
workflow, locally and across the existing peer transport. The method must execute its verification,
perform a permitted repair when needed, and call the same protected deployment operation. It may
use an execution-only host for build/model tools; the publication's internal run remains owned by
the workflow-enabled host.

Required cases include:

- Two callers invoke the same published version with different inputs; their runs can adapt
  differently under the same policy without shared mutable state or changed promises.
- Lose acceptance and internal-start replies, kill/reopen the relevant owners, and replay the exact
  request after archival: retain one internal run and no duplicate deployment.
- Change request bytes under the same key, substitute a publication version/target, or reuse another
  caller's receipt: conflict or denied disclosure, not another authorized operation.
- Publish a newer generation and retire the old one during active work: accepted calls keep their
  exact starting method/agreement and legal continuation; new selection follows explicit policy.
- Invoke with one worker slot and nested ordinary work: no worker starvation. Test nesting/cycle
  limits, queue saturation, cancellation, partial failure, and bounded shutdown.
- Invoke-only caller successfully deploys within the public contract but cannot inspect protected
  internals, edit/republish, use production secrets, or choose an unapproved target. An authorized
  editor succeeds through the ordinary control path without changing another accepted call.
- Induce the adaptation/effect bypasses from 03 through the public wrapper and direct lower-level
  calls; wrapper publication must not weaken them.
- Share a finite parent budget across internal calls; force exhaustion, restart, uncertain response,
  duplicate settlement, and a new child/revision. No allowance reset or double charge.
- Fail public result publication after the internal run settles; retain its exact state and public
  uncertainty without rerunning the method. Preserve artifact sensitivity and current authorization.

Use actual product binaries for the scenario and deterministic model/process fixtures for fault
reproducibility. Assert external side effects and internal run identities through public inspection
and independent counters, not only private struct equality. Extend relevant runtime/control/authority/
peer/client/daemon/storage/shared-adapter suites and run the full gate. Update fixtures, schema
versions/read policies, examples, CLI parsing, architecture, and operations in the same change.

## Completion and handoff

The stop condition is a real versioned workflow-backed capability, not just a publication registry.
It must be callable with narrow public permission, recoverable, cancellation-aware, budget-correct,
nonblocking, and faithful to its governing agreement. Fix any underlying scheduling/ownership issue
needed to achieve that outcome now, across all affected consumers.

Write `handoffs/04.md` with publication and invocation ownership, linkage/recovery transaction rules,
service authority and account semantics, public/private inspection behavior, tested commands, exact
versions, removed alternatives, and evidence. The next assignment learns and promotes candidate
methods using this implemented publication boundary; it must not repair a stub left here.
