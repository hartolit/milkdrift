# Milkdrift: adaptable methods, dependable capabilities, independent hosts

Discussion synthesis and implementation direction, 2026-09-17.

This document carries the user's intent and the conclusions of the host and adaptive-workflow
discussions into the next sprint. It is not a transcript, an assessment of the agents who took
part, or a claim that the proposed behavior already exists. The user has now requested the sprint;
[00](00-first-execution-prompt.md) must adopt the direction in its proper canonical owners before
implementation. Keep enduring intent in vision, ownership in architecture, compatibility decisions
in ADRs, verified behavior in status, and unfinished work in the roadmap. Do not paste this entire
document into each of them.

Baseline reviewed: `855f8ecbb1007baa2a91006384fa322af40ebef9`. The source index at the end pins the
observations below. The earlier independent-host document and documentation patch are inputs to
this synthesis, not prerequisites that must be applied. This synthesis incorporates the later
clarification about evolving methods and supersedes incompatible interpretations of the earlier
proposal. No prior patch application, runtime test, or physical host experiment is assumed.

## 1. The product we are building

Milkdrift should help people and agents develop methods of work, adapt those methods while work
is running, and turn useful experience into reusable blueprints. It should also provide a practical
place to perform that work: prepared tools, normal working files, services, permissions, recovery,
and other machines' capabilities.

The host is supporting infrastructure. The product is not merely a container manager or a remote
command launcher. Conversely, execution hosting must be independently useful: a person should not
need to create a workflow merely to invoke an installed tool or administer an owned service.

The user's two ambitions are compatible:

- An exploratory workflow can develop quickly, reconsider its plan, learn, and continue through
  many revisions. It can propose changes to the goal when its authority permits that discussion.
- A dependable published workflow can adapt its method to the problem while remaining accountable
  to the responsibilities and caller-visible conditions under which it was invoked.

These are different scopes of freedom in the same system, not two workflow engines. A dependable
method may delegate open-ended investigation. An exploratory product workflow may call a narrowly
specified deployment method. Neither invocation silently changes the permissions or obligations
of the called scope.

The intended operator experience is one coherent application across separately owned hosts. The
UM790 can supply a toolchain, model service, and working areas. A desktop can coordinate work using
those capabilities. A workflow-enabled host can publish a complete deployment method. Authorized
editors can inspect and improve that method; invoke-only users receive its supported operation and
results. A later GUI presents these owners without merging their databases or giving every user
universal authority.

## 2. Blueprints preserve a method, not a recorded command sequence

A blueprint is a reusable definition of how to pursue an outcome. Its important building blocks
express responsibilities, decisions, required evidence, and permitted adaptation. A particular
revision is an immutable executable definition. A run records what actually happened under one or
more prospectively adopted revisions.

For an application-development method, requirements, implementation, verification, and review are
not valuable merely as labels in a graph. Requirements should resolve enough uncertainty to guide
implementation. Verification should apply the declared checks to the actual candidate. A review
may reveal that the method needs further investigation or that the assignment needs an explicit
change. Those responsibilities can persist while the necessary tasks differ between applications.

A branch, an already-declared repeat, or an agent choosing an investigative technique can vary a
run without changing its definition. Adding a genuinely new stage, dependency, or task contract
creates a new revision. Applying it must preserve past execution facts and affect only permitted
future work. Existing branches, bounded repeats, pinned subworkflows, and prospective revision
adoption are relevant foundations; they do not alone define which obligations an adaptation must
preserve. [S2][S3]

Do not introduce a global `static`/`dynamic` switch. Record what a particular scope may change:
its internal plan, tool selection within grants, approved configuration, or proposed requirements.
A capability's maturity or evaluation evidence is also not an authority level. Calling a method
“stable” cannot grant it production access; calling it “experimental” cannot excuse unrecorded
effects, unlimited spending, or permission escalation.

Continuous improvement needs useful preauthorization. Requiring another human approval for every
permitted repair or internal rearrangement would defeat the product. At the same time, approval
of a broad goal must not authorize arbitrary changes to acceptance conditions. The implementation
must allow scoped adaptation while reserving changes to the governing agreement for a distinct,
properly authorized decision.

## 3. Distinguish the agreement, the method, and the evidence

A dependable invocation needs an identifiable agreement: its input/output meaning, relevant
process and result obligations, effect prerequisites, adaptation limits, and failure or escalation
behavior. Its starting blueprint and actual adopted revisions explain the method used to satisfy
that agreement. Its evidence explains what was checked, against which candidate and environment,
by which authorized verifier, with what limitations.

Process obligations matter as well as output shape. A caller might require a separate review,
a particular verification family, an approval before production exposure, or a retained deployment
receipt. Returning correctly shaped JSON does not establish that these steps happened. A required
review cannot be silently deleted because another agent considers its answer sufficient.

The agreement must not be editable by the same delegated change that adapts the method. An agent
may repair an application or propose additional investigation. It must not make a failing candidate
acceptable by deleting the check, replacing its verifier with itself, narrowing the tested content,
changing the target, or redefining successful completion. Changed requirements need an explicit
new agreement or a narrowly recorded exception authorized by their owner; they must not be reported
as compliance with the original requirements.

Do not attempt to prove that arbitrary graph programs are semantically equivalent. Use explicit
protected responsibilities, allowed edit scopes, verified input/output boundaries, and effect-time
checks. Choose the smallest representation that enforces these rules throughout current consumers.
Preserve ordinary workflow composition rather than adding a competing obligation scheduler.

The current risk classifier is a coarse approval mechanism, not proof that all important obligations
survive arbitrary changes. The current result-acceptance capability separates a completed task from
an accepted result, but its documented checks have limited meanings and depend on the selected
verifier. Neither fact should be promoted into a general correctness guarantee. [S4][S5]

## 4. The decisive example: repair is allowed, bypass is not

Use one small application and a controlled deployment target to make the direction concrete.
The application's first candidate fails a declared check. An agent investigates the failure, adds
or revises permitted repair work, creates a changed candidate, and obtains new applicable evidence.
The verified candidate can then be published. The failed candidate and its failed evidence remain
part of the history.

The same scenario must reject an attempt to remove the required verification, lower its standard,
substitute an untrusted verifier, use evidence for a different candidate, or call a lower-level
publication endpoint without the prerequisites. A graph that still contains a verification node
is not protective when the worker can publish around it with unrestricted credentials.

The operation that performs the protected effect must enforce its own requirements for every
caller: CLI, agent, local workflow, or remote workflow. Bind applicable evidence to the artifact
actually deployed, the policy and verifier identity, relevant configuration, target, validity
conditions, and permitted use. A checksum is content identity, not proof that the producer was
trusted or that the content satisfies the requirement. Read permission is not publication authority.

Evidence must cover the state the effect will use. A source commit alone does not identify an edited
working tree, and verified source does not automatically identify a later rebuilt binary. Prefer
immutable deployment candidates and capture the relevant configuration with them. Revalidate or
reject when a material dependency changes. Avoid turning a time-of-check/time-of-use race into an
implicit approval of newer bytes.

The host does not need the entire workflow graph to enforce a protected operation. It needs the
operation's prerequisites and trustworthy applicable evidence, issued through the authorized owner.
Model prose, arbitrary booleans, and caller-authored “verified” labels do not supply that proof.
Administrative escape hatches may exist under separately granted authority, but cannot be available
to the supposedly constrained repair worker or mislabeled as ordinary compliant publication.

Passing a finite security check supports that check's stated claim, not “the application has no
possible flaws.” The sprint's example must name what its verifier establishes and must not treat
passing the demonstration as a general product-security certification.

## 5. Three distinct kinds of improvement

**Adapt this invocation.** The agent addresses the problem in the current product or deployment
within the accepted adaptation policy. The run retains its actual revisions and evidence. Other
callers do not inherit the change automatically.

**Improve the reusable blueprint.** An authorized agent reviews relevant history, proposes a better
method, and evaluates it on declared tasks or product variations. One successful trace is evidence,
not a proof that every step was necessary or that the method generalizes. Evaluation should include
failures, counterexamples, declared criteria, and separate test inputs rather than selecting only
the run that inspired the proposal.

**Change or publish the callable version.** An authorized publisher selects the revised method,
its obligations, and adaptation policy for future callers. Publication is distinct from creating
a proposal or achieving a successful run. Existing callers retain the exact version they accepted.
Retirement can stop new admission without erasing accepted work or rewriting its promises.

These actions may all be automated within explicit grants and policy. They do not inherently
require three people. They do require distinct authority and records, so permission to repair one
application does not accidentally become permission to change everyone's deployment standard.

Exploration can produce a reusable product-line method. For example, evidence may show that missing
planning caused conflicting features and expensive rework. A proposed blueprint can add requirements
clarification before implementation and a defined response to contradictions. Separate runs can
then generate variants using explicit product parameters. Inputs, mutable files, acceptance results,
and costs belong to their respective runs. Shared immutable lessons do not justify shared writable
checkouts or cross-branch data leakage.

Retiring or reconsidering a reusable method is also ordinary controlled work. The product must not
freeze a method forever merely because it was once published. Improvements become explicit new
versions; permitted adaptation remains available inside each accepted invocation.

## 6. What reproducibility means

Use precise claims rather than one all-purpose word:

| Property | Meaning |
| --- | --- |
| Inspectable history | Recover what was selected, attempted, changed, and accepted in a particular run. |
| Reusable method | Apply a selected blueprint and its adaptation rules to another input or variant. |
| Consistent acceptance | The agreed standards govern results; standards are not rewritten to excuse failures. |
| Reproducible setup | Recorded recipes, versions, and configuration can reconstruct the declared tool environment within its stated limits. |
| Reproducible execution or output | Enough inputs and external conditions are controlled to repeat the relevant behavior or bytes; this requires separate evidence. |

Identical acceptance rules do not imply identical generated output. A saved timeline does not
reconstruct a mutable external service. A recipe does not restore a database's contents. A fixed
model identifier does not necessarily freeze all server behavior. Record what is pinned, observed,
mutable, or unknown; refuse a guarantee the selected implementation cannot provide.

Agents may install and experiment in their working area. To publish a maintained toolchain or
service generation, the changes must become explicit recipe/configuration inputs, verified before
activation. Source files, documents, datasets, and database content remain working data under their
own preservation policy. Undocumented scratch changes are permitted only as scratch changes; they
must not be represented as a reproducible maintained environment.

This setup discipline supports the workflow discipline but does not replace it. Tool reproducibility
explains the circumstances of execution. The selected agreement, blueprint revisions, and evidence
explain the method and the reasons a result was accepted.

## 7. Independent hosting and explicit ownership

| Owner | Responsibility |
| --- | --- |
| Execution host | Authenticate and authorize operations, host exact capability generations, execute or durably arrange work, retain acceptance and observations, and recover. |
| Workflow engine | Own definitions, scheduling, causal selection, prospective changes, workflow obligations, approvals, cumulative accounts, and run history. |
| Managed-resource owner | Own approved setup/configuration, resource identity, active use, lifecycle changes, verification, and removal policy. |
| Platform supervisor | Maintain the owned service process and report actual state under its installed configuration. |
| Published-method owner | Bind a callable version to its agreement and starting method; durably associate accepted calls with internal runs through existing owners. |
| Client or future GUI | Submit commands and display authorized state without opening databases or acquiring implicit administrative privilege. |

Use one daemon executable with execution-only and workflow-enabled configurations initially. Both
compose the same hosting machinery. Execution-only startup needs authentication, authority, storage,
artifacts, execution recovery, and any installed resource management. It must not construct the
workflow runtime, workflow control service, or workflow workers simply to serve tools. A full host
adds the workflow role; it is not a second product or a daemon-per-environment arrangement.

Role changes cannot abandon work. Disabling workflow ownership against a store with live or unresolved
workflow obligations must refuse with useful diagnostics. Closed history must remain intact and
inspectable through a supported owner. Unsupported routes should explain that the role is absent,
not pretend that protected stored records never existed. Do not claim a small memory footprint
without measurement. The goal is independent operation, not an arbitrary crate count or immediate
removal of every compiled workflow dependency.

Current normal startup creates `RuntimeService` and `ControlService`; adapter execution context
requires workflow coordinates. Independent hosting is therefore a cross-cutting implementation,
not a presentation switch. [S6][S7]

## 8. Direct operations, peer transport, and shared execution

An operation name identifies what can be done. An invocation identifies one request to do it. A
direct invocation needs exact caller, request, inputs, capability generation, permissions, limits,
and outcome facts without invented workflow coordinates. A workflow-originated invocation also
carries its actual workflow owner, run, revision, node, execution, and attempt relationship.

Origin and transport are separate. A peer can carry either kind of work. A CLI is not required to
pretend to be a peer daemon. A caller-supplied origin tag does not grant authority. Delegated worker
credentials cannot reclassify controlled work as independent work to escape budgets or restrictions.
Namespaces must prevent different callers or peers from colliding on execution identifiers.

Generalize the prepared-entry boundary: authorize, acquire the exact generation, prepare bounded
inputs without external effects, revalidate authority and current prerequisites, durably record
entry and applicable reservations, then enter the adapter. Preparation, refusal, possible external
entry, terminal evidence, and lost reporting remain distinct. The current direct execution helpers
assign substantial obligations to their callers; exposing one through HTTP is not sufficient. [S8]

Share these mechanisms without creating competing histories. Local workflow attempts keep their
runtime-owned journal and final-entry/account transaction. Standalone and serving-side remote work
use a generalized durable execution owner based on existing peer acceptance and recovery. A remote
workflow's originating host keeps workflow truth while the serving host retains its own accepted
operation. Their records link; neither becomes a replica of the other's database.

Peer persistence is durable. Active records are not compacted; archived summaries preserve identity
and outcome/uncertainty. That foundation can be reused without using peer receipts as a resource
inventory. Resource state and required lifecycle evidence have their own retention obligations. [S9]

Exact replay must survive disconnect and archival. Changed bytes under the same scoped key conflict.
Replaying acceptance does not bypass current authorization to disclose results. A missing response
after possible entry is uncertainty, not permission to repeat an arbitrary effect. Cancellation
acknowledgement is not proof of termination. Pre-entry continuation requires revalidation; entered
work is never replayed merely because a lease expired.

## 9. Inputs, artifacts, and cumulative authority

Retain workflow causal selection, exact-attempt binding, branch visibility, required-input handling,
omission redaction, and continuation checks. Direct selection is a separate explicit meaning:
only supplied or referenced inputs that the host authorizes, bounds, and freezes for that invocation.
It does not discover workflow history or search other working areas implicitly. Initial direct model
support uses fresh requests and explicit inputs; existing workflow continuation retains its own
supported behavior. [S10]

Adapters consume validated selection and verify bytes. A missing, mismatched, or unsupported selection
must fail before the forbidden read or effect. Do not make context optional to avoid changing its
owner. Generalize artifact producer, materialization, and accounting identity to represent host
invocations honestly; do not create fake runs for direct output. Continue using the ordinary
artifact store and secret-reference mechanism rather than new parallel stores.

Permission to invoke a capability is separate from its resource budget and protected-effect
requirements. Descendants inherit applicable authority, obligations, and accounting. They cannot
obtain broader rights by changing node type, connecting a peer, spawning a new run, or calling a
direct endpoint. Host-scoped and per-call limits also apply to truly independent calls.

Cross-host accounting needs an explicit reservation and settlement relationship, not a shared
mutable global counter. Reserve an enforceable allowance at the caller, enforce the accepted
allowance where work occurs, and settle authenticated observations without double charging the
same use. Outstanding or uncertain use cannot be released because of timeout alone. An unknown
usage dimension must stay unknown or cause refusal when the selected contract requires a bound.
Existing inability to measure arbitrary agent-internal model calls must not be hidden by the new
interfaces. Do not require paid cloud inference to demonstrate local operation.

## 10. Managed installations are durable resources

An installation records approved recipe/configuration revisions, actual implementation identity,
owned resources, observed state, published generations, active use, data-retention choices, and
pending lifecycle changes. The invocation that installed a service can finish while the service
continues indefinitely. Completing or compacting that invocation must not erase the service's
ownership, operating instructions, or removal path.

Provide prepare/apply, inspect, start/stop, update, and remove behavior through the same owner for
direct clients and workflows. Reapplying an unchanged setup preserves useful work and avoids needless
recreation. Changes require expected versions and exact approval where applicable. Generated
capability generations become available only after verification; an old generation cannot secretly
point to a changed service behind the same URL.

Distinguish managed resources from attached external services. Removing an attachment removes local
connection state, not the external process or its data. Adoption of an existing resource is explicit
and verified, never inferred from a matching name. Shared prerequisites and caches are not casually
owned by whichever installation used them last. Destructive cleanup requires a concrete ownership
and preservation plan.

A finite lifecycle controller is necessary; another workflow language is not. Reuse maintained
recipes and an external supervisor. Milkdrift supplies the common authorization, version check,
resource-use coordination, recovery, inspection, and removal inventory that ad hoc scripts alone
would otherwise duplicate across callers. If a mechanism already supplies those guarantees, adapt
it rather than implementing it twice.

## 11. Resource use, interrupted changes, and truthful recovery

Reserve the exact required resources before allowing an accepted operation to be invalidated by
maintenance. Keep holds linked to the authoritative execution record. On one store, coordinate
acceptance or entry and required holds transactionally. Across hosts, use acknowledged durable
acceptance, not a pretend distributed transaction.

For the first implementation, permit one mutating operation per managed working area. This is not
one operation per environment or host: an unrelated model request can proceed concurrently, and
separate worktrees can be used for parallel variants. Include read use when removal or replacement
would invalidate it. A maintenance request closes new affected admission and either refuses busy
state or waits under a declared bound. Avoid hold-order deadlocks when more than one resource is
required.

Unknown work cannot hold the system hostage without explanation, but timeout is not a deletion
license. Provide inspection and an explicit authorized resolution/disruption path. A decision to
reclaim a resource at known risk remains distinct from evidence that an old invocation succeeded
or safely terminated. Rebuild use obligations before opening admission after restart.

Record intended resource identity and change before creating a container, replacing configuration,
or removing data. Inspect that exact identity after interruption. A matching name or PID is not
sufficient ownership proof. A crash after platform success and before result commit must not create
a duplicate resource. A stale callback cannot advance a newer change. Recovery can finish verified
recording, carry out a specifically safe remaining step, compensate when authorized, or preserve
partial/uncertain state.

There is no atomic transaction covering redb, systemd, the filesystem, and arbitrary external APIs.
Define and test each interruption boundary honestly. Data migrations or irreversible external
changes may prevent rollback even when restoring an old executable is possible. Never claim full
rollback solely because a container image can be changed back.

Services and requests recover differently. A healthy service after reboot does not prove the result
of a model call that lost its reply. A disconnected client does not stop a shared service. Normal
host shutdown follows declared persistent-service policy while task-owned children follow their
execution cleanup policy.

## 12. The first Linux working setup

Use rootless Podman and systemd/Quadlet behind the managed-resource boundary. Quadlet generates
systemd services from definitions and supports user units. Milkdrift owns approved configuration
and resource records; systemd owns supervision. The raw Podman API provides the service account's
full container-engine authority, so it must not be exposed as a scoped worker operation. [S11][S12]

The installation must establish actual separation. Identify manager-owned database, credentials,
configuration, executable, service definitions, control sockets, and writable roots. Workers get
only the declared mounts, network/device access, and delegated API credentials. They cannot modify
the manager or replace the controls enforcing their grant. Trusted host execution under the manager
account remains trusted with that account; grants cannot contain arbitrary code sharing its OS
privileges. [S13]

Prefer one understandable installation and worker environment over a proliferation of accounts or
containers. That preference cannot substitute for enforcement. Use distinct service or worker
identities when the claimed boundary needs them, and explain their purpose. Bootstrap elevation is
separate from routine worker authority. Never solve an isolation failure with silent privileged
containers, unrestricted mounts, host networking, or a host-shell fallback.

Prepare useful build tools, ordinary source files, a documentation entry point, and an owned
llama-server service. Keep inference external and configurable. Also attach an existing endpoint
and exercise a trusted native capability to prove that managed containers are optional execution
choices. Basic installation must work without a model; the model-driven demonstration follows it.

The operator's first machine is a UM790 Pro with 64 GB of shared memory, with a reported large iGPU
reservation. Agents must discover actual available memory, storage, OS/session support, and devices
rather than add system memory and iGPU memory as separate capacity. Model selection is a local
operator choice, not a fabricated fixed recommendation. Record exact model/server/configuration
and observed backend; a successful CPU request is not GPU qualification. Do not change firmware
memory allocation or tune unrelated system settings as part of bootstrap.

Provide an inspectable, repeatable setup command and deliberate removal procedure. Preserve chosen
source, outputs, documentation, databases, credentials, and models according to policy. Removing a
local secret reference does not revoke it at a remote provider. Reapplying a recipe must not overwrite
agent work or erase experimental changes without an explicit policy decision.

## 13. Published workflows are callable methods

A published version identifies its starting blueprint, input/output contract, protected obligations,
adaptation policy, and relevant execution constraints. This corrects an earlier overly compressed
statement that it simply binds an internal revision. The starting revision is exact, but permitted
run-specific adaptation can produce different actual revision histories for different invocations.
The allowed adaptation is part of what the caller selected, not an unannounced moving target.

One accepted invocation maps durably to one internal run on a workflow-enabled host. Lost replies,
reconnection, restart, or withdrawal of a catalog entry must not create another run. Creation and
linkage require an atomic same-store transition where possible or a deliberately recoverable
idempotent command association. Invocation acknowledgement, internal completion, accepted result,
and surviving deployed service remain distinct facts.

The published method's owner controls its internal implementation and credentials. The caller
must have permission to invoke the exact public operation, but need not receive production secrets
or the unrestricted union of every internal capability. An explicit service-execution policy must
bind the permitted caller inputs, internal identity, delegation limits, obligations, and budgets.
This is constrained service authority, not accidental privilege inherited from the publisher or
from a peer name. Untrusted caller-supplied paths, targets, callbacks, and context cannot turn the
published method into a general administrative proxy.

Expose inspection and editing to properly authorized colleagues through the same client surfaces.
Invoking, viewing internal evidence, editing the method, changing obligations, publishing a version,
and administering its tools are separately grantable. A shared interface or organization label
does not override host authorization.

Waiting for an internal run must not occupy the entire worker pool needed to execute that run.
Use the existing durable scheduling and observation owners to arrange continuation; do not create
one unbounded thread per invocation or another orchestration engine. Bound nested calls and detect
unsupported recursive/circular compositions. Cancellation propagates with its own acknowledgement
and evidence; stopping the caller is not proof of stopping remote work.

## 14. Knowledge and learning remain accountable data

Each working setup should have a discoverable entry point describing purpose, usage, decisions,
limitations, and relevant learned knowledge. Agents can update it and propose method improvements.
Preserve source, scope, applicability, supersession, and approval/promotion history using existing
workspace, artifact, context, and proposal facilities where their meanings fit.

A mutable documentation entry point can identify the latest useful knowledge, while a run freezes
the exact versions it selected. A later edit must not change what a completed task appears to have
known. Retrieved text is evidence, not permission or privileged instruction. Cross-workspace learning
requires explicit authorized source selection; a global knowledge search must not bypass branch
visibility or artifact sensitivity.

The sprint must implement one complete learning path rather than promise general intelligence:
select authorized run evidence, produce a structured candidate blueprint, evaluate it under stated
criteria on distinct inputs, retain the results, and promote or reject through ordinary control.
Use a real model for the operational demonstration, deterministic fixtures for reproducible tests,
and record that neither establishes broad model-quality guarantees. Fine-tuning, a new memory
engine, or a giant generic lesson schema is not required.

## 15. What remains and what changes from earlier proposals

Retain independent hosts, optional managed isolation, one hosting implementation, durable serving-side
acceptance, owned resources, external inference, shared administration, and the complete CLI-first
UM790 lifecycle. Retain the distinction between resource state and operation history. Retain exact
workflow truth without a second authoritative local attempt ledger.

Replace the idea that the host alone defines the next product milestone. The sprint must also
establish adaptable methods, protected responsibilities, versioned publication, and a concrete
learning/reuse path. Replace a fixed internal graph interpretation of published versions with an
exact starting method plus permitted adaptation. Replace universal “reproducibility” claims with
the distinct properties above. Reject global static/dynamic modes and unlimited rogue authority.

The critiques produced useful implementation requirements, not proof that the project needs a
rewrite: durable resource records cannot be replaced by execution receipts; standalone inputs need
a valid owner; service supervision needs an actual mechanism; every changed contract must be adopted
fully. Likewise, an existing workflow engine is not the only possible owner of safe lifecycle
transitions. Authorization and effect prerequisites belong below all caller types.

Use abstractions when they consolidate shared meaning, not to make the diagram look complete. A
crate must enforce a real semantic, contract, adapter, dependency, or consumer boundary. Existing
accidental boundaries may be changed when necessary; do not preserve them at the expense of one
coherent implementation, and do not undertake unrelated restructuring under this sprint's name.

## 16. Evidence that will settle the design

The [sprint assignments](README.md#assignments-and-current-state) own execution order. Together they
must establish the following observations through product behavior:

1. A direct client and a desktop workflow use the same execution-only host, with real caller/origin
   records, bounded model inputs, useful process work, and no invented local workflow.
2. Exact accepted work survives response loss and restart without duplicate entry; denied reads,
   revocation, forged provenance, stale versions, and exceeded allowances refuse safely.
3. A protected Linux setup supports repeated application, useful work, independent service recovery,
   versioned update, active-use coordination, and ownership-aware removal beside an external service.
4. An agent can make a useful authorized repair and adopt a prospective revision. It cannot weaken
   the accepted obligations, forge a verifier result, reuse stale evidence, or publish around them.
5. A versioned workflow capability creates one recoverable internal run per accepted call, permits
   authorized internal editing without granting it to invoke-only clients, and retains budgets and
   effect requirements across hosts and nested calls.
6. Run evidence produces a proposed, evaluated method improvement and independent product variations;
   promotion affects future selection without rewriting older accepted versions or histories.
7. The whole scenario works on the real authorized Linux/UM790 setup, with actual local inference,
   explicit evidence limits, reproducible commands, and no left-behind disposable resources.

These are acceptance requirements, not reports of tests run while drafting this document. Missing
hardware access must be reported as a qualification gap, not simulated by a success label. Basic
implementation, deterministic refusal/recovery tests, and operator procedures remain required even
when a particular machine is unavailable to the implementing agent.

## 17. Source basis and limits

Repository links are pinned to the reviewed baseline. They establish current owners and limitations,
not that the new direction is already implemented. Agents must reread the current checkout before
editing. External manuals were consulted on 2026-09-17; verify the installed platform's version
before using any option. Design rules above are sprint decisions or user requirements, not claims
that a reference mandates one architecture.

- **S1 — Product owners:** [vision](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/product/vision.md), [architecture](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/architecture.md), [status](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/product/status.md), [roadmap](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/product/roadmap.md).
- **S2 — Existing blueprint meaning:** [blueprint README](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/crates/blueprint/README.md).
- **S3 — Prospective changes:** [ADR 0005](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/decisions/0005-prospective-revision-reconciliation.md).
- **S4 — Current risk classification:** [policy.rs](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/crates/control/src/policy.rs).
- **S5 — Acceptance semantics and limits:** [result-acceptance guide](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/guides/result-acceptance.md).
- **S6 — Current daemon composition:** [startup.rs](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/apps/daemon/src/host/startup.rs).
- **S7 — Adapter execution and publication identity:** [adapter.rs](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/crates/capability-host/src/adapter.rs).
- **S8 — Entry and serving owners:** [capability entry](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/crates/capability-host/src/registry/execution.rs), [peer worker](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/adapters/peer-http/src/service/worker.rs).
- **S9 — Durable non-run state:** [ADR 0024](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/decisions/0024-peer-execution-hot-retention-and-tombstones.md), [ADR 0022](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/decisions/0022-redb-owned-daemon-application-state.md).
- **S10 — Context and model preparation:** [ADR 0031](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/decisions/0031-context-enforcement-and-retained-evidence.md), [model-provider guide](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/adapters/model-provider/README.md).
- **S11 — Initial supervision mechanism:** [Podman Quadlet manual](https://docs.podman.io/en/stable/markdown/podman-systemd.unit.5.html).
- **S12 — Container API authority:** [Podman system-service security](https://docs.podman.io/en/stable/markdown/podman-system-service.1.html#security).
- **S13 — Actual process trust:** [ADR 0021](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/decisions/0021-byte-pinned-trusted-host-processes.md).
- **S14 — Shared authority and inheritance:** [ADR 0019](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/decisions/0019-frozen-execution-authority.md), [ADR 0020](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/decisions/0020-one-authorized-control-and-read-plane.md).
- **S15 — Existing discussion:** [managed execution and connected hosts](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/development/virtual-office/whiteboard/discussions/managed-execution-and-connected-hosts.md).
- **S16 — Assignment and explanation rules:** [virtual office](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/development/virtual-office/README.md), [implementation practice](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/development/practices/implementation.md), [documentation practice](https://github.com/hartolit/milkdrift/blob/855f8ecbb1007baa2a91006384fa322af40ebef9/docs/development/practices/documentation.md).
