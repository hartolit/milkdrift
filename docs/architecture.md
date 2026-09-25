# Architecture

Milkdrift separates a reusable method, the history of performing it, and authority to change what
happens next. In the current workflow composition, a client submits intent to the daemon; runtime
accepts facts durably and asks external capabilities to perform work. Independent hosting
also admits direct operations without a workflow. Clients inspect the owning service's
projections instead of opening storage or reconstructing adapter behavior themselves.

Offline `storage-admin` is a separate daemon executable path under OS file-owner authority.
Redb owns source locking, private inspection copies and complete stopped-generation copies;
the application composes redacted diagnostics with runtime's pure projection/context checks.
It constructs no runtime service or external capabilities. Ordinary store opening refuses the
inspection-only marker on backups/restores before any database writes.

This document explains which component owns each part of that operation and the invariants that
connect them. The terminology and package map below support the detailed lifecycle sections.
Read [vision](product/vision.md) for intent, [status](product/status.md) for exact current versions
and qualification, and [the source-learning route](README.md#learning-the-implementation) to trace
the executable system. Development methods belong in the [practices](development/practices/README.md).

## Terminology

- **Blueprint:** reusable method expressed as a declarative workflow or subworkflow package.
  **Workflow:** identity with revision lineage. **Revision:** immutable semantic snapshot with exact parents.
- **Node:** definition-time unit. **Node execution:** one runtime occurrence. **Attempt:** one
  invocation under an exact capability selection. **Run:** durable execution pinned to a revision
  lineage. **Edge:** explicit control and/or typed data dependency.
- **Host invocation:** one accepted direct or delegated operation. **Origin:** its direct caller or
  workflow relationship, independent of transport. Local workflow attempts retain their runtime owner.
- **Requirement:** what a task needs. **Descriptor:** immutable advertisement of one capability
  generation. **Observation:** mutable health/load/availability or a reported execution fact.
- **Author:** bounded revision provenance. **Actor:** authenticated principal. **Grant:** immutable
  scoped authority revision. **Controller:** an actor allowed to influence future work.
- **Command:** validated intent with authority, idempotency, and optimistic expectations.
  **Event:** immutable accepted fact. **Projection:** derived operational state.
  **Effect:** interaction with an external capability or infrastructure.
- **Workspace:** scoped logical state. **Artifact:** immutable content reference with digest,
  media type, size, sensitivity, producer, causal inputs, and retention class.
  **Context manifest:** exact selected and omitted task evidence.
- **Reconciliation:** prospective application of a revision to live work. **Prompt sequence:**
  bounded import/template compiled into ordinary revisions. **Layout:** presentation state outside
  semantic identity. **Peer:** remote execution host with its own accepted invocation records.

- **Installation:** durable approved setup and owned resources. **Managed working area:** ordinary
  mutable files with a resource generation, lifetime holds and one editing owner; distinct from
  logical workspace values and temporary materialization. **Attachment:** access to a resource
  owned elsewhere, without permission to delete it.

Agreements and published methods bind adaptation to an accepted contract:

- **Agreement:** immutable obligations, effect prerequisites and adaptation limits accepted for a
  scope. **Method adaptation:** a prospective revision inside those limits, not a change to them.
- **Published method:** a callable version binding an exact starting blueprint, agreement,
  adaptation policy and constrained service identity. **Promotion:** selection for future calls,
  distinct from repairing a run or approving a changed agreement.

## Owners and dependency direction

The physical workspace below is the current complete package map. Names in the first column correspond to
directories; every package is named `milkdrift-` plus the final directory component, except the CLI
executable, which is `milkdrift`. Private children organize each owner's implementation.

| Package directory | Owned responsibility and principal modules |
| --- | --- |
| `crates/contracts` | Shared canonical/bounded JSON, validated-string, digest-lexical, UTF-8 truncation, and validating-deserialization mechanics. Domains own meaning and wire shapes. |
| `crates/capability` | Identities, requirements, descriptors, pure matching, resolved snapshots, invocation/observation/cancellation and managed-resource request contracts (`descriptor`, `invocation`, `document`, `managed`). |
| `crates/blueprint` | Immutable graph, structured definitions, task/context policy, validation, revisions, mutation (`model`, `validation`, `revision`, `mutation`, `context`). |
| `crates/workspace` | Scope lineage, values, artifact metadata, provenance, and workspace budgets. |
| `crates/authority` | Actors, grants, resource selectors, decisions, frozen execution basis, pure evaluation, and secret references (`model`, `evaluator`, `document`, `secret`). |
| `crates/model` | Provider-neutral task/response and exact causal-manifest contracts (`task`, `context`, `document`). |
| `crates/persistence` | Durable documents and narrow journal/revision/workspace/artifact/application/peer/clock ports; controller account validation and transitions. |
| `crates/runtime` | Commands, scheduling, final entry/reporting, structured work, projection, recovery, reconciliation, and causal discovery/selection (`engine`, `projection`, `context`). |
| `crates/capability-host` | Live generations, selection/permits, prepared adapter entry, direct/peer serving lifecycle, bounded workers, secrets/materialization ports, and `RuntimeStore` bridge (`registry`, `serving`, `worker`, `materialization`, `managed`). |
| `crates/control` | Proposals, deterministic risk, grants/presets, controller lifecycle, application orchestration, and ordinary workflow-control adapter (`service`, `policy`, `controller`, `adapter`). |
| `crates/prompt-sequence` | Strict JSON/Markdown imports, ordinary blueprint compilation, stage association, and prospective remediation; no executor or storage. |
| `crates/control-protocol` | External command/read/layout DTOs, codecs, negotiation, authenticated cursors; no HTTP/runtime/storage types. |
| `crates/control-client` | Typed authenticated HTTP, bounded safe-query retries and artifact ranges, exact-cursor SSE reconnect. |
| `crates/peer-protocol` | Transport-neutral session/catalog/execution/cancellation/artifact contracts and strict codecs. |
| `adapters/local-process` | Profile validation and byte identity; direct argv, preparation, streams, monitoring, outputs, and platform process ownership (`config`, `process`). |
| `adapters/managed-linux` | Strict workload-independent Linux recipes, rootless Podman effects, generated user Quadlets, owned temporary workers and model-service use adapters. |
| `adapters/model-provider` | Endpoint policy, feature negotiation, bounded HTTP/SSE, independent OpenAI-compatible and Anthropic mappings, artifact publication. |
| `adapters/local-secret` | Explicit environment/restricted-file references; no enumeration or retained secret values. |
| `adapters/peer-http` | Authentication, configured transport, catalogs, remote capability adapters, and artifact transfer framing. Capability-host owns serving lifecycle. |
| `adapters/redb-store` | Physical schema/transactions, journal/indexes, snapshots, accounts, application/peer retention, artifacts, and administrative integrity scans. |
| `apps/daemon` | Configuration compilation, authentication, single owner queue, command/read adaptation, HTTP/SSE, startup/maintenance/shutdown (`config`, `auth`, `host`, `http`). |
| `apps/cli` | Arguments and presentation over the control client; bounded input/output/session/streaming owners and command-family routing. |
| `tools/evidence` | Unpublished development leaf: actual-binary harness, mutation runner, benchmarks, operational and external evidence. |
| `examples/adaptive-slotbook` | Independently deployed Rust application and explicit defective fixture. Product packages do not depend on this example. |

Dependencies point toward stable semantics. Capability knows nothing about blueprint; blueprint
uses pure requirements and schema identities. Persistence consumes immutable domain contracts and
owns no runtime decisions. Runtime and redb are sibling consumers of persistence; runtime's redb
dependency is development-only. Capability-host implements runtime's `TaskExecutor`; adapters
implement the host-owned port without opening redb or deciding run state. Control consumes the
semantic, authority, persistence, runtime, model, and host boundaries and delegates to their owners.

```text
semantic contracts ← persistence ports ← runtime ← capability host ← adapters
        ↑                    ↑              ↑              ↑           ↑
        └────────────── control/import ──────┘              └── daemon ─┘
control protocol ← control client ← CLI                         ↑
        └───────────────────────────────────────────────────────┘
```

The diagram shows dependency layers, not every Cargo edge. Daemon explicitly composes concrete
storage, secret, process/model/peer adapters. The CLI consumes control-client, control-protocol,
capability and serving wire documents from peer-protocol, plus prompt-sequence for local authoring.
It neither constructs runtime services nor opens storage. HTTP, database, OS, provider, and async types stay outside
semantic contracts. No UI or local inference package exists. Exact manifest boundaries are checked
by `tools/evidence/tests/repository_contracts.rs`.

Canonical capability-owned identities (`PeerId`, `SchemaId`, `ExtensionKey`, `BoundedJson`,
`TrustZone`) are imported directly; consuming domains do not re-export alternative owners.
[Public API policy](reference/public-api-policy.md) governs exports and test-only features.

Transport-independent serving acceptance, preparation, entry and recovery live in capability-host;
local workflow history stays in runtime. Further accepted dependency changes preserve that inward
direction. Host resource policy belongs in a narrow module
with typed persistence actions and an adopted Linux mechanism adapter. Blueprint owns agreement
structure; authority owns verifier policy and service delegation decisions, and control owns publication and
proposal orchestration. Host consumes a continuation port implemented by control, avoiding a
host-to-control dependency cycle. Shared producer meaning stays in workspace, with one artifact
store. These moves are required by concrete direct/resource/publication consumers; no new crate
is required just to name a concept. ADRs [0038](decisions/0038-independent-host-execution.md),
[0039](decisions/0039-managed-resource-ownership.md), [0040](decisions/0040-protected-adaptive-methods.md)
and [0041](decisions/0041-published-method-invocation.md) own rationale and compatibility decisions.

Daemon startup composes common storage, clock, authority, registry and serving workers in both
roles. Only workflow-enabled hosts construct runtime, control and effect workers. Recovery closes
new entry until these owners are ready; execution-only startup refuses outstanding workflow
obligations and preserves closed history. The installation identity is durable and cannot change
with a configuration edit. Authenticated direct clients and delegated peers share serving acceptance
and reporting, while local workflow attempts retain their atomic runtime journal/account transition.

Adapter preparation freezes validated inputs before external entry. Serving and runtime owners
then recheck authority and their own lease, cancellation and allowance facts before committing
entry. A consumed prepared call cannot be reused. Direct selection contains only explicitly supplied
inline/artifact inputs and does not manufacture workflow context. Host-invocation and client-input
artifact owners have no run namespace. Peer imports preserve foreign causes as authenticated claims,
and controlled remote outputs charge the originating reservation through ordinary publication.

## Definitions and prospective control

A revision is created by one complete versioned mutation batch against genesis or an exact
optimistic base. The private candidate must validate before publication. Identity includes
semantic content and exact parents, excluding layout, time, and map/JSON object ordering.
Multiple parents mean a deliberately resolved merge, never automatic conflict invention.

Runs record append-only accepted events. Reconciliation preserves completed, started,
effect-dispatched, and otherwise committed executions under their original revision and
provenance. Only uncommitted future work may be redirected. Incompatible interfaces, required work
removal, stale sequence/base/digest/plan guards, authority changes, or uncertain effects produce
typed refusal/conflict/remediation. The closed reconciliation matrix owns both classification and
allowable actions; a new definition cannot reinterpret old evidence.

Proposals are bounded, duplicate-safe, canonical untrusted documents binding workflow, exact base,
optional run/sequence, mutation batch, provenance, evidence, rationale, risk, application policy,
optional run action, and stop condition. Large reasoning stays in artifacts. Only decoded structured
model output can become a proposal; prose and returned tool calls do not execute control actions.

Control privately validates the candidate, computes authority delta and deterministic risk, stores
the revision, and delegates acceptance to runtime. It never appends run events itself. Current low-risk
auto-apply requires explicit policy and exact apply authority and is limited to future pure/read-only
work. Terminal/started work, side effects, profile/trust expansion, subworkflow/interface changes,
cancellation, and elevated changes require recorded approval. Approval links exact proposal,
revision/effect, approver, and policy. Preset names expand into grants rather than runtime roles.

The structural adaptive-method boundary refines that coarse approval rule inside explicit governed
scopes. A run binds an immutable agreement separately from the revision region an agent may edit.
Blueprint validates preserved responsibilities and interfaces; control classifies the actual delta;
runtime applies it prospectively under the retained authority/account. Useful repair, investigation
and dependency edits within that region can be preauthorized. The same grant cannot weaken the
agreement, verifier, target, completion requirements or its own limits. Outside such scopes the
existing approval policy remains. Agreement changes require a distinct authorized decision and
cannot relabel old failure as compliance. This checks protected structure and effect prerequisites,
not equivalence of arbitrary programs. Authority owns the finite verifier policy, workspace owns
its exact candidate observations, and capability-host requires private journal evidence at managed
publication entry under ADR 0040. An `AgreementAccepted` event retains the originating run,
revision and agreement digest; structured children inherit it unchanged. A governed child pin is
protected even from a separate direct adoption command. A new agreement is accepted by a separate
run, never by relabelling the original scope.

## One command and external-effect path

Consider a client starting a run. The reply confirms the accepted command; task completion arrives
later through durable adapter observations. Two connected paths preserve that distinction:

```text
client request
    |
    v
authentication → authority → durable command result → reply
                                  |
                                  v
                              scheduler
                                  |
                                  v
                           capability entry
                                  |
                                  v
                        observations → read models
```

If the reply is lost, exact request replay recovers the command result. If an external operation
loses its result after entry, the runtime may retain an uncertain attempt. Command replay does
not turn that missing external evidence into a success or authorize another invocation.

Each externally initiated mutation binds actor, exact grant revision/digest, command identity,
canonical complete request, and optimistic guards. Authentication supplies server-owned actor
facts; command JSON cannot claim them. Runtime acceptance or denial-without-events commits the
canonical authority decision and result atomically. Exact redelivery returns that decision/result
without reevaluation; changed canonical facts conflict.

Start freezes an `ExecutionAuthorityBasis`: actor, grant and policy identities/versions/digests,
workflow/run/initiating revision lineage, decision provenance, and revocation generation. Children
inherit that basis. Later revisions may narrow it, never replace or enlarge it. All reachable task,
reducer, and nested-workflow requirements are checked against the frozen envelope before start or
adoption without needing a live provider.

That is the current ordinary run/subworkflow rule. The adopted published-service boundary binds
both the caller relationship and an explicitly constrained service identity under ADR 0041. It
does not implicitly widen ordinary children or reset their inherited restrictions and accounts.

The host evaluates every semantic candidate against authority before mutable health or capacity.
Stable selection uses the exact requirement, explicit priority, capability identity, and revision.
Resolution, exact-generation claim, and immediately-before-entry evaluation retain canonical
decisions. Their requests bind descriptor category/operation/profile/trust/locality/peer,
side effects/idempotency, adapter filesystem/network/secret needs, and budgets. Execution and
cancellation never fall back to another generation. Revocation denies future entry and releases
leases without rewriting already-entered work.

An adapter must declare immutable authority requirements and explicit start, drain, and shutdown
behavior. Start finishes before registration is published. Health preserves the caller's boundary
time. Cancellation acknowledgements bind invocation and request sequence. Registry locks are not
held over adapter code; fixed caller-owned workers use bounded queues, claim pages, panic
containment, and deadline-driven shutdown. Every adapter runs the shared conformance suite.

`TaskExecutor` has one incremental durable reporter path. Adapters submit observations; runtime
owns sequence acceptance, terminal uniqueness, heartbeat lease extension, and workflow transitions.
A durable terminal observation outranks a later worker failure. Missing terminal evidence after
entry remains uncertain. Retry eligibility follows recorded side-effect and idempotency facts;
neither cancellation requests nor connection closure prove that an external effect stopped.
Worker/system receipts are private runtime paths, not alternate external authority.

### Direct and remotely served operations

The shared path is authorize → exact-generation preparation → revalidation → durable entry
and reservations → adapter entry → durable observations. Preparation freezes authorized data without
external effects. It is shared mechanism, not a shared second journal. Runtime's current local
entry/account transaction remains the only authority for a local attempt. A generalized serving
execution owner persists direct and incoming remote acceptance, claims, entry and cancellation.
An origin workflow and a serving operation have linked records because they own different facts.

A direct client supplies explicit bounded inputs and receives a real host invocation identity.
Workflow delegation carries its actual owner/run/revision/execution/attempt and governing selection.
Authentication binds delegation, grant and account; a delegated credential cannot relabel its work
as direct to avoid limits. Replay is scoped by host, authenticated caller realm/principal and exact
request. Current disclosure authorization remains required even for retained results. After possible
entry, missing evidence preserves uncertainty. Capability-host owns this serving lifecycle;
[ADR 0038](decisions/0038-independent-host-execution.md) explains why it shares preparation with
local workflow execution while retaining a separate durable owner.

## Scoped authority and disclosure

All local/peer commands, information-bearing reads, pages, and subscriptions use the same pure
evaluator. Grants constrain workflow/run, operation, capability identity/category/operation/profile,
execution trust/locality, paths/access, network, secrets, artifact identity/sensitivity, layout,
peer, workspace, diagnostic class, budgets, validity, and revocation. Allow dimensions use explicit
`Any` or bounded nonempty `Only` beneath whole-scope `DenyAll`; empty collections cannot grant
wildcards. Shared-layout revisions are explicit; actor-owned layout vocabulary is reserved.

For an unnamed task implementation, prospective runtime validation can probe the ordinary
authorized resolver and narrow that identity to a current registered, granted implementation. Other
requirement dimensions remain subject to the full envelope check. The probe creates no attempt
or permit. Transient health and permit capacity affect scheduling and entry, not prospective
authority; those boundaries repeat selection and authorization. Publication uses the same
rule, allowing one reusable definition to run in separately granted workspaces without giving
each service authority over every workspace.

Collections are filtered before hidden identities are fetched or projected. Artifact metadata and
content are independently authorized against stored sensitivity. Readiness discloses less than
health. Consequential mutations, protected releases, and peer administration retain bounded
decision provenance without credentials or payloads. Authentication and grants remain separate
even for valid tokens, peer relationships, descriptor labels, or model role claims.

Bearer digests are compared in constant time. Explicit environment/file references are reread for
rotation through the same secret resolver used by adapters and peers. The resolver bounds values
and never enumerates ambient environment. [Authority configuration](operations/authority.md) owns
selector syntax, path grammar, dangerous-scope acknowledgements, and restart/revocation procedure.

External cursors bind feed position, actor, exact grant, decision, query/filter digest, and a
credential-derived MAC. They cannot cross credential rotation, actor, grant replacement, or query
scope. Streams authenticate and authorize establishment and every bounded poll; revocation stops
future disclosure distinctly from transport failure. External historical DTOs are separately
versioned projections, never serialization of internal events, redb rows, or provider payloads.
[Control API](reference/control-api.md) owns wire details.

## Structured execution

Control/data edges are acyclic. Branch uses a safe condition AST to select a declared arm. Fork
creates named isolated child scopes. Its join waits for all, any successful, or a satisfiable
quorum with explicit failure/cancellation policy; reducer is a separate declared data operation.
Repeat invokes a pinned acyclic body with a hard iteration maximum, optional tighter time/cost
limits, and terminal limit policy. Wait/timer and authenticated correlated signal waits are durable.
Subworkflow pins a revision and interface rather than copying mutable definitions. Explicit
success/failure/cancellation terminals cannot complete a scope with unaccounted owned work.

Prompt-sequence compilation emits fresh/explicitly continued coding tasks, distinct verification
and result-acceptance tasks, branches over accepted results, review/signal holds, and ordinary
prospective remediation. `milkdrift-control` owns the fixed purpose checks; the installed control
adapter reads exact evidence through the daemon's authorized artifact port and publishes an
immutable decision. Runtime continues to own ordinary task and branch execution. The compiler owns
import bounds, canonical digests, identifiers, and template policy only. Repository ownership,
authority, scheduling, and effects stay with existing owners.

## Context and artifacts

Context answers what evidence one task should receive. A task revision requests sources and
budgets; runtime selects from causally visible work and records the selection before dispatch.
The capability host then loads the selected bytes. Keeping selection separate from loading lets
an inspector explain what the attempt received even after newer work or a revision exists.

Branches isolate mutable workspace state. Cross-branch transfer requires declared data edges,
artifacts, joins, reducers, or explicit imports. An authorized persistent host repository is an
explicit sequential-process choice; parallel work requires separate worktrees/scopes and merge
capabilities. It does not imply a Git implementation or sandbox.

Task revisions own context policy and output semantic roles. Runtime discovers metadata through a
bounded recent journal tail frozen at one sequence plus compact projection anchors at exact
historical terminal sequences. It joins governing revisions, scope lineage, workspace/artifact
metadata, and frozen authority. Sources include inputs, bounded ancestors, exact executions, roles,
failures/decisions, explicit evidence, join results, and imported subworkflow outputs. Lifetime
settled history alone cannot exhaust discovery; there is no whole-history chronological fallback.

Historical ancestry follows each execution's governing revision. A join exposes only its declared
result to descendants of that exact join; sibling failures and private output cannot cross by
association. Subworkflow imports follow their exact durable parent link. Omission records must not
reveal protected identities or sizes. Ordering is causal depth, semantic kind, node, execution, and
canonical reference bytes. Scan/depth/event-summary/item/artifact/per-item/byte/manifest/unit
bounds apply deterministically. Optional losses retain policy/budget/authority/missing/corrupt/
unsupported/superseded/isolation reasons; fail-closed policy requires required losses to fail before
dispatch, including after selection stops. Scope and authority facts redact omission metadata
independently of the reported reason. Selection policy version 2 records these corrected rules.

The canonical manifest binds run/revision/execution/attempt and policy digest. Each selection binds
content digest/size, semantic tags, governing revision, scope/sequence/time, producer actor,
capability/generation/profile/peer/invocation, causal evidence, sensitivity, authority, and reason.
Omissions, totals, budget, and a domain-separated digest complete the record. Its restricted
artifact is committed and journal-published before scheduling can reach an adapter; invocation
carries the compact reference. A retry rebinds the prior selection to its new attempt without
rescanning later history. Before reuse, runtime refuses older omissions that cannot establish
required-evidence or disclosure safety; recovered leases face the same check with admission closed.
Historical bytes remain readable and unchanged. [ADR 0031](decisions/0031-context-enforcement-and-retained-evidence.md)
owns this compatibility boundary. A different selection requires a distinct manifest/attempt.

The host materializes only selected non-direct content after authorization and verifies digest,
size, and media facts. Model requests receive the manifest as system context and delimited
untrusted evidence; negotiation includes injected features. Unsupported roles/images/binary parts
fail before HTTP. Runtime compares a model task's session declaration with its inline or
artifact-backed request before claiming work, including retries and recovered leases. Agreement
does not supply an unsupported provider protocol. Processes explicitly map reserved manifest/context
inputs through existing input-file policy; no ambient global context file appears. Outputs retain
manifest/input provenance.

For direct calls, the serving owner freezes a distinct explicit selection from supplied
inputs and authorized references; it discovers no workflow ancestors. Host materialization verifies
that selection and the bytes, including empty selection where valid. Workflow causal manifests and
continuation retain their stronger run/attempt rules. Direct fresh model calls cannot bypass those
rules by submitting workflow context under another origin. Workspace/persistence producer and
accounting contracts identify actual host invocations, imports and workflow attempts in one
artifact store. Public input uploads and peer transfers use its bounded publication, retention,
integrity and authorized read paths.

Artifact publication uses bounded resumable chunks, exact offsets, digest/size checks, and atomic
metadata/accounting acceptance after content publication. Read authority, integrity verification,
and ranges remain independent. Abort/cleanup release owned reservations; replay and content
deduplication do not duplicate logical charges. Explicit retention may expire bytes while keeping
safe metadata and integrity evidence; compaction never silently deletes artifact content or outputs.

## Managed resources and editing ownership

Capability-host's `managed` resource owner
retains approved recipe/configuration, exact platform identity, generation, ownership/preservation
policy, use and pending changes. Persistence/redb atomically record guarded intent and affected
admission before a platform action. A later transaction records observed completion against the
same transition generation. Rootless Podman and systemd/Quadlet supply Linux effects and service
supervision; a successful service restart is not evidence that a lost model request completed.
Native trusted execution and externally attached services remain usable.

Resource state outlives the creating operation. After create succeeds but result recording fails,
recovery inspects the saved intended identity/configuration and records verified completion or
uncertainty without creating a replacement. Updates/removal close affected admission and respect
exact generation holds. Compaction retains ownership/removal facts and preserved-data disposition
independently of execution detail. Attached services are not deletion targets. No transaction spans
redb, the filesystem and systemd; no general rollback is promised for irreversible external changes.

A lifetime hold prevents invalidating the generation required by accepted work. An editing claim
permits one mutator in a working area. A waiting parent keeps its lifetime hold while explicitly
handing the editing claim to its exact accepted child. The resource transaction checks generation,
parent/child association, inherited authority and expected claim. The parent must first suspend
writes with evidence of physical quiescence; releasing a thread or acknowledging cancellation is
insufficient for a live process holding a writable mount. Child entry waits for the durable transfer.

After proven child quiescence, the owner settles its use and makes the parent eligible to reacquire
editing under current checks. Parent/conflicting writes and removal remain refused while use is
uncertain, including after restart. Other resources can progress. Authorized blocker inspection
and resolution extend the existing command/read plane; a disruption records risk and requires
physical fencing before reuse, without pretending to establish the old operation's result.
[ADR 0039](decisions/0039-managed-resource-ownership.md) owns the precise handoff and recovery rules.
Local `NodeScheduled` acceptance and direct/peer serving acceptance acquire the descriptor's exact
`org.milkdrift/managed-resources` dependencies in their existing redb transactions. The resource
index projects those facts; it does not schedule an independent execution. Final-entry evidence
permits physical entry under the current claim. A terminal event releases unentered reservations,
but entered work needs adapter stop evidence. Accepted subworkflow journal links identify the exact
parent logical execution and inherited authority. Published links instead identify the caller's
exact saved association and its configured service identity. The wrapper has no physical writer:
its entry transaction records `NoExternalEntry` quiescence before any child takes editing. Physical
children still require the enforcing adapter's exact stop/fencing evidence.

The current maintenance policy refuses busy state immediately. Successful acceptance commits a
bounded transition and closes admission; each platform result is separately guarded by transition
identity and step. The immutable acceptance receipt never becomes the mutable inventory. Startup
resumes pending steps, retains explicitly uncertain steps, and rebuilds registry projections without
creating replacement identities. Linux recipe generation includes exact image/model/configuration
inputs. The Linux recipe separates worker and owned-service container budgets, supplies one model
alias and reuses provider endpoint limits. Application initialization lives in an approved image or
ordinary worker operations; the Slotbook example has no privileged production path. Preparation
receives the saved current generation for headroom accounting and returns authorized diagnostics
without effects. [Managed operations](operations/managed-linux.md) owns the usable path, ownership boundary,
retained disk state, backup exclusions and pending physical enforcement qualification.

## Agreements at effects and published methods

Protected managed publication requires verifier evidence bound to immutable candidate/build
bytes, material configuration, target generation, agreement/policy, verifier identity and validity.
Authority establishes verifier trust separately from content integrity. The resource operation
checks these prerequisites after preparation at consequential entry for every caller, including
direct/raw update paths. Repair workers cannot write served content or use an unguarded socket,
credential or native tool to bypass the operation. The existing result-acceptance capability retains
its finite documented meaning; it is not by itself this effect protection.

Control publishes exact starting methods and adaptation policies through the normal registry. A
published version binds a service principal/grant and input/target narrowing policy, allowing a
caller to invoke deployment without owning its internal administrative grant union. Caller authority,
inherited restrictions, agreement and budgets remain applicable; caller-supplied paths or context
cannot turn the service into an arbitrary proxy. Internal inspection/editing, publication and public
invocation remain separately granted operations.

The accepted operation retains a planned child identity and canonical create/start command association
before internal creation. Control submits those exact runtime commands; their receipts prove linkage
after interruption. A local caller stores the pending link in runtime history; a direct/remote caller
uses its serving record. No duplicate local attempt journal is added. Once arranged, the call yields
its worker capacity and continues through bounded durable observation while retaining version and
resource lifetime pins. Runtime remains the only scheduler. Exact ancestry/depth bounds refuse
unsupported recursion. Shared-area children use the editing transfer above as well as releasing
worker capacity. Cancellation, internal completion, agreement satisfaction, result publication and
surviving deployed service remain separate facts. [ADR 0041](decisions/0041-published-method-invocation.md)
owns linkage, service authority and account settlement.

The configured `runtime.publication_services` map names each capability's immutable service grant.
It requires workflow mode and explicit controller activation. Control validates supported internal
implementations, service create/start and invocation authority, the input contract, declared terminal
outputs, and conservative accounting bounds before publication. Registry capacity is reserved before
the inventory commit. Discovery projects public documentation and constraints without disclosing the
starting graph or service grant. Retirement preserves the inventory and accepted continuations;
restoration advertises an unavailable exact generation when its current service cannot execute it.
Before and after internal preparation, runtime rechecks the enclosing invocation's deadline and
local caller chain. Serving owns current client/peer checks through the host's weak owner reference;
the service grant alone cannot authorize further entry for a cancelled or revoked public call. Accepted ancestry
also carries each publication's depth ceiling, so a descendant cannot reset a stricter limit.

Each service run has a persistence-owned cumulative allowance. Ordinary descendants inherit it;
controllers cannot replace it. A calling workflow reserves the method's full internal envelope in
its own account before entry, then settles measured internal work and public copies once. Missing
usage retains that reservation. Direct/peer serving accounts apply the same finite per-call envelope.
These are per-invocation budgets, alongside service authority and host capacity; they are not a
service-wide lifetime spending account. [Published methods](guides/published-methods.md) explains
configuration, operator commands, disclosure, and the supported limits.

Learning composes ordinary control proposals, runtime evaluations, artifacts and publication.
`control::learning` owns strict selection/declaration requests and the deterministic comparison.
The daemon checks authenticated source reads, verifies actual model-output provenance and reads
the declared runtime/account/verifier evidence. Its ordinary application receipts retain each
decision; there is no lesson database or background learning scheduler. Editable workspace notes
become task knowledge only through an explicit immutable artifact selection. Source pages freeze
the public timeline projection; private contents need their own permitted artifact reads.

Before proposal generation, an independent declaration fixes paired inputs, exact worker/target
generations, checks, thresholds and budgets. Missing or unexamined evidence stays inconclusive.
Only separate publication authority, or an exact operator policy rechecked against current grants,
can promote an eligible method. Old accepted calls keep their revision and agreement. Each product
variation owns mutable files, inputs, verification and usage. Recipe activation records a separate
generation and never changes a workflow publication implicitly. [Learning methods](guides/learning-methods.md)
explains these operations; [ADR 0042](decisions/0042-selected-evidence-and-method-evaluation.md)
owns their boundary and [status](product/status.md) records executed evidence.

## History, compaction, and recovery

Long-running workflows need complete history without keeping every settled attempt in the active
scheduler state. The journal retains accepted facts; the projection keeps what current work still
needs. Compaction changes the latter, so historical inspection must follow retained identities
back to journal pages rather than interpreting a compact view as the entire run.

| State | Authority and bound |
| --- | --- |
| Journal | Complete immutable event history and exact command results. |
| Active projection | Current operational obligations and compact terminal frontiers. |
| Snapshot | Optional bounded, verified recovery checkpoint. |
| Historical read model | Stable-cursor paged reconstruction of accepted facts. |

Projection folding retains full occurrences while eligible, dispatched, retrying, uncertain,
cancelling, awaiting successor scans, owning structured work, or carrying unconsumed import or
reconciliation obligations. Once closed, each current scope/node frontier retains at most one
terminal summary: governing revision, creation/terminal boundaries, attempt identity/count,
conservative side effects, selected route, and live output/provenance references.

Settled progress, old attempts/leases, consumed signals, fired timers, superseded frontiers and
revision pins, closed recovery/reconciliation records, and consumed branch/repeat/subworkflow
ownership retire to the journal. Stable identities, sequence anchors, and history digests locate
older evidence. Fixed workflow shape and bounded concurrency do not grow active state merely with
elapsed iterations. Shape, live branches, unresolved safety obligations, outputs/artifacts/context,
and configured workspace bounds can still increase it; this is no universal constant-memory claim.

Journal transactions atomically append checksummed events with runtime receipts, workspace/account
changes, and recovery indexes. Separate narrow application ports own external receipts, layouts,
proposal discovery, and bounded security audit. A receipt exists in exactly one hot or cold tier;
oldest-first bounded archival, new receipt insertion, and same-store layout/proposal effects commit
atomically. Cold storage retains the identical canonical request/result for replay and conflict.
If runtime acceptance precedes external receipt commit, recovery replays the stable internal
command and commits the missing receipt; it does not redo the effect.

`RuntimeService::open_closed` checks physical compatibility without admitting work. Startup recovers
active runtime/application owners before adapter registration, peer recovery, worker startup, and
admission. It does not scan all terminal history or hash all artifact bytes. Active corruption fails
startup closed; unrelated terminal corruption is found on read or explicit scrub.

An explicitly selected recovery composition permanently restricts a closed runtime to existing
authorized recovery commands. Control selects safe-restart proposal plans and requires approval;
runtime commits prospective cancellation and revision pins through its ordinary command owner.
The daemon serves authenticated controls and receipt maintenance without adapters, peers,
execution materializations or scheduling. A fresh normal startup must validate all active state
before execution. [ADR 0035](decisions/0035-authorized-recovery-controls.md) owns this boundary.

`StorageAdmin::scan_integrity` performs bounded resumable historical verification under one read
transaction per page. Physical phases and cursor policy cover primary records, derived indexes,
receipts/layouts/proposals/audit, artifacts, and controller accounts. Artifact-content verification
is opt-in and bound to the continuation. Health samples are not full integrity proof.

Snapshots have independent persistence envelope and runtime payload versions. Strict padded
standard Base64 carries payload bytes. A domain-separated, length-framed checksum binds metadata
and decoded payload, and the selected append transaction records an exact payload commitment in
the history chain. Envelope checksum, event-prefix digest, and append-time commitment must agree
before a snapshot is verified. Lexical bounds precede tree allocation. Unsupported or invalid
optional snapshots are discarded for authoritative journal replay; no lifetime ID set belongs in
a checkpoint.

## Controller resource accounting

The control lifecycle parses immutable digest-bound controller policy in an ordinary pinned repeat
wrapper. Policy binds identity, wrapper/body/interface, all cumulative ceilings, checkpoints,
fail-closed unknown-usage behavior, stop behavior, currency, labels, and provenance. Unknown policy,
digest mismatch, contradictory wrapper, zero resource limits, or legacy metadata-only patterns fail.
Monetary allowance may be zero; an absent currency requires zero and forbids billed admission. A
controller cannot modify its own policy; another authorized actor needs a new reconciled revision.

Persistence owns account declaration/state, reservations, optimistic revision digest, and immutable
run binding. Its six ceilings are cost, input units, output units, artifact bytes, process entries,
and model entries. Input/output are logical model tokens, including generated reasoning, rather than
bytes or cache-evaluation work. A bounded envelope must explicitly declare `model_tokens` to enter
this account; omitted or incompatible units cannot be combined. Process adapters declare these
dimensions non-applicable. Redb changes totals only in existing journal/artifact transactions. Descendants inherit
the exact account; nested policies in an already bound descendant are refused. Final entry
checks authority, prepares local request data under the exact generation permit, rechecks authority,
then atomically commits entry intent and account admission before external work. Frozen descriptor
category determines process/model entry counts.

After local preparation, entry refreshes unrelated sibling journal changes and verifies that its
own attempt, lease, execution, authority, revision pin, and cancellation boundary still match.
This preserves the prepared handle across sibling writes made during preparation. The entry/account
transaction still checks the current run head and account revision; a later conflict retries within
the existing bound, and a changed or expired ticket is refused.

The daemon installs that one lifecycle before recovery when `controller_activation = "enabled"`
is explicitly configured. The default is disabled; the feature-gated development `qualification`
mode uses the same installation. Active account bindings require installation during recovery;
marked revisions also require account establishment before any external task entry.
Child creation in a marked revision also requires an account, established earlier or atomically
in the same transaction. An unmarked child placed before activation cannot escape that account.
Authorized run/controller reads project this account with committed and remaining totals. They
do not maintain another ledger, and a descendant read requires inspection of the account origin.

A parent's pinned child revision binds the immutable child creation event. Authorized prospective
reconciliation may change the child's current revision while the original creation pin, inputs,
workspace, inherited authority, and account binding remain enforced.

Unit, cost, and artifact obligations reserve before entry. Unknown bounds, currency mismatch,
blocked state, overflow, or ceiling violation produces durable denial. Terminal evidence settles
only authoritative use; missing bounded observations retain remainders and block future admission.
Uncertainty retains every remainder; over-envelope use blocks the account. Logical artifact
publication charges its exact reservation or run binding in the metadata transaction. Immutable
account revisions replay each change from its predecessor and exact transition/publication source.

The model adapter owns operator billing and request counting. Its supported text contract computes
the envelope after injecting context and retains it with the exact prepared body. Unbilled charge
is non-applicable; token units still reserve and settle. Explicit text tariffs reserve conservative
charges and calculate observed accounting from final tokens, retaining raw provider amounts
separately. Frozen profile generation facts prevent later edits from repricing an attempt. The
runtime and account do not implement tokenizers, price catalogues or local inference.

Lifecycle assessment consumes settled totals plus outstanding reservations; it separately owns
cycles, revisions, proposals, elapsed time, failures, rejections, and depth. Repeat's structural
guard is one iteration higher so assessment records the reached ceiling before excess child
creation. Proposal dimensions are checked before revision storage; approval/application reassess
cumulative policy. Checkpoints use ordinary durable repeat-continuation decisions with revocation
and limit rechecks. Reached bounds record dimension/current/limit or unknown usage and fail without
provider retry. [Status](product/status.md) owns the production activation gate and evidence.

## Adapter trust and peer durability

Process profiles bind digest/size, optional deployment revision, safe path digests, platform/file
observations, full profile and execution-policy digests, trust class, and ownership facts. Bounded
registration verification precedes descriptor creation. Health and immediately-pre-spawn checks
re-resolve roots, identity, metadata, and bytes; mismatch makes that generation sticky-unavailable
until explicit new registration. Safe Rust leaves a minimized verification-to-entry race; no
atomic open-handle execution guarantee is claimed.

Direct argv substitutions remain single OS arguments. Children begin with cleared environment and
receive only allowlisted names/resolved secrets. Secret-bearing profiles cannot stream text.
Canonical roots, isolated materialization, bounded regular files, traversal/symlink/hardlink
refusal, and declared output imports mediate access. `TrustedHostProcess` still has daemon-account
privileges; `SandboxedProcess` requires a distinct enforcing adapter. On Unix, immediate-child exit
with a live owned group initiates bounded group teardown even if descendants retain output pipes.
One invocation owner keeps the spawned child, started I/O workers, and cancellation registration
together. Reporting/setup failure or unwinding disconnects the stream channel and requests forced
termination and interrupts nonblocking local pipe operations before joining workers; registration
and the outer host permit outlive that cleanup. One monotonic deadline covers group observation
and final pipe draining. Missing EOF remains uncertain even after local joins or parent exit;
it does not authorize successful output publication or automatic unsafe retry.
The original reporting failure still reaches runtime without invented terminal evidence.
[Process operations](guides/local-process.md) owns platform and profile details.

Model adapters reject unadvertised roles, parts, tools, schemas, reasoning, streaming, sessions, and
oversized encoded requests before entry. Output/tool calls remain artifacts, not automatic tool
execution. The two provider mappings preserve their own response/stream semantics and truthful
usage, cancellation, idempotency, and side-effect limits.
Runtime context assembly resolves explicit model continuation from exact prior manifest/response
artifacts and journal anchors. It freezes ordered messages and provenance in a versioned companion
selected by the ordinary manifest; adapters only translate that prepared selection. No conversation
ledger or provider session pointer is added to hot projections. Current authority and source evidence
are rechecked after local preparation before every entry, including recovery and retry.
[ADR 0036](decisions/0036-explicit-model-continuation.md)
defines the supported boundary and compatibility choices.
Local preparation owns the exact encoded request and ephemeral headers under the host's generation
permit. Runtime rechecks authority afterward, refreshes unrelated journal writes while validating
the exact prepared ticket, and commits against that checked run head. A durable local refusal
precedes entry intent and creates no account reservation;
an intent without terminal proof remains uncertain across restart. The
[adapter guide](../adapters/model-provider/README.md#prepare-once-before-external-entry) explains
the distinction between local refusal, possible submission, complete response, and reporting loss.

Peers consume expiring authority-filtered catalogs. Exact remote identities/generations map to
collision-resistant local capability identities with typed peer, locality, trust, and catalog
provenance. The origin owns workflow truth; the serving daemon owns durable remote acceptance.
Every acceptance/query/page/cancel acknowledgement binds the caller's exact request or URL identity.

Task `PlacementRequirement` adds intersecting locality and exact-peer sets to the existing requirement
owner. Revision admission proves that envelope against the retained grant; matching, frozen schema-3
snapshots, and final entry enforce the actual host. Empty sets select nothing. No catalog or health
failure widens the task or moves entered work. Origin-side inspection keeps the request and selected
peer/catalog generation. [ADR 0037](decisions/0037-constrained-peer-placement.md) owns compatibility.

Serving output publication uses the accepted peer execution's remaining artifact allowance in the
ordinary core artifact store, with that execution as its external producer. Origin-side references
remain causal commitments to the accepted request, not invented local journal facts. The origin
imports observed outputs through execution-owned, authorized metadata/chunk transfers before
reporting their exact references. This path shares core retention and publication; input transfer
and repository/credential ownership remain explicit.

Serving acceptance atomically binds canonical idempotency, relationship/catalog/generation,
authority, capacity, and dispatch availability before responding. Fixed workers claim durable
leases and record entry separately; known-entered work is never automatically replaced. Identical
requests replay from hot records or tombstones; changed requests conflict for the store generation.
Hot observations are contiguous bounded rows with a rolling digest. Eligible terminal/uncertain
records archive oldest-first atomically, retaining identity, provenance, authority, cancellation,
accounting, observation count/digest, and final disposition; detailed observations and peer links
retire while core artifacts keep independent retention.

Connection closure is neither cancellation nor terminal proof. Post-entry missing restart evidence
becomes uncertainty. Pre-entry cancellation preserves acknowledgement-completion recovery even
when terminal observation committed just before clock/storage failure. Inbound/outbound artifacts
use core publication/read ports. [Peer protocol](reference/peer-protocol.md) owns wire/retention
details; [peer operations](operations/peers.md) owns operator connectivity and quotas.

## Daemon lifecycle and compatibility

Configuration selects workflow-enabled or execution-only composition. Execution-only startup uses
the common owners without constructing runtime, control or workflow workers. Role removal refuses
nonterminal runs, active leases and unresolved controller accounts; closed history stays intact for
offline inspection. Health reports the role, and absent workflow operations return an explicit
unavailable result.

The [daemon](../apps/daemon/README.md) connects these owners into one process. Startup establishes
what can safely continue before accepting new work; shutdown keeps storage available until workers
have finished their final reports. Operators arrange configuration, credentials, external services,
and backup through [daemon operations](operations/daemon.md).

Strict bounded duplicate/unknown-field-rejecting TOML compiles once into normalized immutable owner
plans. Enabled peer mode requires identity; internal owners receive only their section. One bounded
synchronous owner queue serializes runtime, control, storage, clock, and registry operations.
Typed one-shot closures capture response channels; HTTP owns sockets/framing, not semantic state.
Queue saturation has a typed overload response. External work runs on fixed workers without a
global lock. Weak peer-facing store handles and managed-resource handles prevent idle routers from
extending storage ownership. The daemon owner retains the managed lifecycle until its workers
finish, then releases it when joined; an idle HTTP connection cannot block the next store open.

One fallible daemon clock advances a durable high-water fact before authorization, expiry,
scheduling, or timestamps; artifact acceptance advances it in the same transaction. Failure or
rollback refuses work and records redacted health evidence. Restart cannot forget a later observed
instant. Elapsed time during downtime still trusts the OS clock. Post-entry peer clock/store loss
retries the exact release/uncertainty transition without another adapter invocation.
Fresh samples are taken only after acquiring the redb write transaction, so concurrent artifact or
receipt publication cannot overtake an earlier sample and manufacture a rollback. The configured
store clock supplies all fresh durable observations; caller-owned timestamps retain exact
watermark validation through the persistence port.

One mutex owns coherent operational health and feed generation. Queue guards release occupancy on
dequeue/drop. Owner-request panic closes ordinary admission; queued requests recheck readiness.
The owner remains available for final clock/persistence/shutdown work, skips normal maintenance,
and cannot claim clean shutdown. These are local admission facts, never workflow history.

Shutdown closes external admission, drains peers/runtime, disconnects registries, and applies the
configured drain/cancel/retain deadline. The owner continues servicing final worker writes while
peer/effect workers join, then joins itself and releases storage. Unresolved identities remain
truthful. [Daemon operations](operations/daemon.md) owns exact startup, backup, and shutdown procedure.

Writers emit one current canonical version per family. Readers accept only versions whose meaning
is unambiguous; unknown core variants, malformed identities, invalid derived fields, duplicates,
and unsupported versions fail. Bounded DNS-namespaced extensions are the explicit unknown-field
mechanism. Semantic/canonical-byte changes require version review and hand-reviewed fixtures;
moving code does not. Current read behavior is tabulated in [status](product/status.md).

Storage physical/internal versions are exact-current with no migration. Legacy sidecars cannot
be ignored or imported as competing authority. Layout has an independent digest and optimistic
generation keyed by workflow/revision; it contains no semantic edges, tasks, prompts, secrets, or
requirements and changes neither revision identity nor runtime history. Rust source compatibility
follows current consumers under the public API policy; durable and wire compatibility remain
independently owned contracts.
