# Execution, authority and resources — independent reconstruction

Faris, 2026-10-10. This is supporting analysis for the existing [system dossier](system-dossier.md),
not another status owner. Source inspected at `ff818e5b47ec7724a239fb0aa9b234880bca4c22`.
No Cargo, platform operation, model call, schema change or production implementation was performed.
The recommendations below are modeled designs, not executed alternatives. The independent position
was recorded before reading any fresh reviewer's conclusions. The original selected program was
treated as provisional.

## Independent position

The useful product is a system in which an actor can call a tool directly, reuse a method under
their own authority, or invoke a restricted service implemented by a private method. The same
person must be able to understand a stopped call and recover its evidence without learning the
database layout. These are distinct promises; neither geography nor the word “method” determines
which authority relation applies. [U07](../intent-source-excerpts.md#u07--reuse-and-remote-execution-preserve-the-surrounding-doubt)
does not authorize making publication the prerequisite for all remote work.

The present execution design earns substantial retention, but not wholesale approval. Its strongest
features are exact generation selection, preparation before durable entry, distinct accepted and
observed facts, real direct origins, bounded continuation without an occupied worker, and explicit
resource quiescence. Its weakest ownership boundary in this review is managed-use policy inside
the redb adapter: database-specific code decides inherited lineage, claim transfer, cancellation and
parent resumption. Another concrete avoidable burden is constructing typed capability descriptors
by serializing and editing their JSON shape. Direct and peer admission also repeat the same fresh
admission rules before converging on the same store action.

I recommend a complete correction of those three boundaries, keeping runtime, serving, resource and
account facts separately owned. A single invocation service is a credible competitor, worked below;
it does not eliminate the difficult publication and physical-stop facts, and would enlarge every
ordinary local workflow's recovery protocol. Moving publication wholesale to runtime or host is
also less coherent than retaining a narrowly defined workflow-service composition in control.
This conclusion concerns the examined execution cluster. It does not approve unexplored provider
semantics, every bound, every authorization request producer, or the whole product program.

## Coverage audit

The old [dossier](system-dossier.md#concept-coverage-and-challenge) and
[behavior evidence](behavior-evidence.md) traced useful outcomes and selected call paths. Their
coverage rows did not establish comparative design review of these twelve packages. The old direct,
peer and publication observations are evidence of behavior under their recorded conditions; they
are not experiments comparing storage owners. “Prior depth” below classifies what the retained
packet demonstrates, not the effort an earlier agent may privately have spent.

All twelve manifests were read and current dependency consumers searched. The listed consumers
are important production consumers, not a generated complete inventory. Test/evidence consumers
do not independently justify a public API or package. Deep means source, failure paths, relevant
test assertions and an alternative were examined; mapped means a narrower source/consumer review.

| Workspace package | Purpose, neighbors and important consumers | Prior depth demonstrated | Fresh depth and limits |
| --- | --- | --- | --- |
| `milkdrift-capability` | Portable requirements, descriptors, exact selections, invocation events and managed request vocabulary. Used by blueprint/workspace/authority, protocol, host and adapters; contains no live executor. | Mapped generic invocation/selection; no substantial comparison of descriptor construction. | Deep on descriptor production, managed bindings and invocation identity; retain semantic crate. Narrow typed extension construction should replace JSON object surgery. All feature-matching dimensions and bounds were not independently rederived. |
| `milkdrift-capability-host` | Registry generations/permits, adapter preparation, materialization, direct/peer serving and managed lifecycle. Runtime consumes `TaskExecutor`; adapters consume its ports; daemon composes it. | Direct/peer lifecycle traced and selected tests observed; broad retention asserted. | Deep on serving acceptance/entry/recovery, publication continuation and resource owner. Compare unified invocation ownership below. Registry lock/health internals and every worker overload path mapped rather than newly stress-tested. |
| `milkdrift-authority` | Pure grant evaluation and immutable accepted execution basis. Runtime, control, host and daemon supply authenticated facts; local-secret uses secret-reference/value types. | Scoped authority retained, largely from documented semantics and selected public refusal tests. | Read evaluator and selector/contract tests. One exact grant plus explicit service relationship is justified; the evaluator cannot establish the truth of caller-supplied resource facts. No complete audit of every producer of `AuthorityRequest`. |
| `milkdrift-persistence` | Durable domain documents, storage ports and pure cumulative-account transitions. Runtime/control/host and redb consume them. | Journals/accounts named and preservation required, without substantial ownership alternative. | Deep on account action/transition, serving records, publication association and resource records; compare extra ledger vs atomic owner facts. Revision/snapshot codecs and every event-family reader only mapped. |
| `milkdrift-peer-protocol` | Authenticated serving caller/origin, delegation, acceptance, observations and artifact transfer wire contracts. Host, peer-http, persistence, client/CLI consume it. | Remote trace; transport-neutral contract inferred. | Deep on caller/origin separation and exact request conversion; package name has become narrower than its direct-client contract. A rename alone would not fix ownership. Complete hostile decoder/negotiation audit not performed. |
| `milkdrift-model` | Provider-neutral model request/response and causal context-manifest contract used by runtime, control and model-provider. | External inference/context retained. | Request/response and context ownership mapped; provider mapping compared below. Workflow context living here is a coupling to coordinate with the context reviewer, not evidence that all model types belong in host. No independent reconstruction of every context selector. |
| `milkdrift-local-process` | Trusted byte-pinned argv execution, materialization, stream/process lifetime and stop observations. Daemon and evidence instantiate it through host. | Existing process qualification cited. | Read preparation, monitor/lifecycle and selected public tests. Distinct OS process owner justified against a generic HTTP/job wrapper; all Windows/Unix platform branches were not exercised. |
| `milkdrift-model-provider` | Endpoint policy, independent provider encodings, bounded network response and usage publication. Daemon and managed-linux consume the same adapter. | Existing endpoint/conformance evidence cited. | Read adapter preparation, HTTP boundary, accounting shape and test assertions; retain provider mappings as external mechanisms. No new provider, live-server or reasoning-mode compatibility claim. |
| `milkdrift-managed-linux` | Rootless Podman/systemd effects, approved recipes, physical fencing and provider reuse for managed models. Host owns lifecycle policy; daemon chooses platform. | Resource lifecycle plus selected lost-stop tests observed; physical evidence inherited. | Deep on ownership separation and model wrapper; lifecycle/physical tests reviewed, not run. Linux enforcement, active reboot and hardware claims remain those of their original evidence. |
| `milkdrift-peer-http` | Peer HTTP/authentication/catalog projection and remote adapter that submits/observes the exact remote acceptance. Daemon consumes registry and transport. | Direct/peer path traced. | Deep on descriptor remapping, remote request/delegation, output recovery. Remote descriptor conversion has legitimate scope meaning. No third-host direct relay or general network deployment proof. |
| `milkdrift-redb-store` | Same-store transactions, physical readers/indexes/retention and administrative preservation. Runtime/control/host use persistence ports; daemon supplies one store. | Recovery/retention behavior mainly inherited from existing tests. | Deep on managed acquisition/transfer and publication links; sampled account/application transaction tests. Significant semantic rules are embedded in mechanism code. Complete integrity scanner, power-loss and all tombstone families remain outside this bounded review. |
| `milkdrift-local-secret` | Exact environment/restricted-file credential lookup without caching or enumeration. Daemon supplies mapping; process/model adapters use `SecretResolver`. | Source family named; design substantially inherited. | Full small implementation and tests read. Retain distinct mechanism crate: it isolates ambient OS access and has a real production consumer. No new non-Unix ACL or filesystem-race assurance. |

The [manifest](../../../../../Cargo.toml) and [architecture](../../../../architecture.md#owners-and-dependency-direction)
own the complete package map. `control`, `runtime`, `daemon`, workspace/context, application
configuration, evaluation and client packages are neighboring source traces here, not package-wide
approval; the coordinator's coverage record must join their reviewers explicitly.

## Need, protection and mechanism

| Valued outcome | Necessary rule | Current mechanism and judgment |
| --- | --- | --- |
| Call a process/model without authoring a workflow. | A real caller, selected inputs, bounded allowance, durable request identity and effect outcome. | `DirectInvocationDraft/Request` → `PeerService::invoke_client` → serving record. The record is valuable; a fabricated `RunId` is not needed. The peer-prefixed Rust names do not create another semantic owner. |
| A workflow can wait for a remote tool. | Origin scheduling and remote accepted effect remain separately knowable through a lost network reply. | Runtime attempt plus B's serving acceptance. These are not duplicate execution history: A knows its dispatch/observations; B knows its own acceptance/entry. |
| Invoke a private reusable method without gaining its internals' privilege. | Explicit input/output disclosure and service authority, separately from caller invocation authority. | `PublishedMethod`, `PublishedServiceIdentity`, `PublishedInvocationPlan`, real internal run. Retain this relationship; do not label every reusable or remote method a publication. |
| Retry a command after losing its reply. | Exact canonical request must recover the same accepted result, including after detail archival. | Runtime, serving and managed receipts bind their respective owner transactions. Sharing canonical machinery is useful; replacing all with one receipt without cross-owner atomicity is not. Application-to-runtime receipt duplication needs the separate application trace; it is not approved by this comparison. |
| A tool installation survives the work that created it. | Resource identity, ownership and removal obligations outlive compactable invocation observations. | `InstallationRecord`, approved setup, pending change and removal disposition. Retain durable inventory apart from invocation log. |
| Parent work lets a child edit its files and later continues. | Lifetime protection differs from permission to mutate; physical writer must stop before claim transfer. | `ManagedUse`, editing list, claim epoch, `QuiescenceEvidence`. Retain these facts; move pure transition policy out of redb-specific functions. |
| A revoked/expired grant stops future entry while preserving history. | Frozen accepted basis plus current evaluation at each effect/disclosure boundary. | `ExecutionAuthorityBasis`, `AuthorityDecisionSnapshot`, final checks before/after preparation. A historical decision is evidence, not evergreen permission. |
| A new child or restart does not reset a budget. | Reserve before entry; settle once from observed usage; retain unknown remainder. | `ControllerAccountState` pure transitions plus atomic journal/serving integration. “Controller” naming is historical: published internal work also uses these accounts. Renaming cannot replace their rule. |
| An endpoint response was observed but not recorded successfully. | Response observation, durable terminal and physical stop are different facts. | `AdapterFailureKind::ResponseObservedFailure`, reporter failure propagation and uncertainty. A generic success/error wrapper would erase useful distinctions. |

The origins are partly established. [ADR 0038](../../../../decisions/0038-independent-host-execution.md)
describes the concrete prior coupling: direct work required workflow coordinates, peer output used
a run-shaped namespace, and peer entry preceded preparation. [ADR 0039](../../../../decisions/0039-managed-resource-ownership.md)
explains why parent lifetime holds and child editing had to diverge. [ADR 0041](../../../../decisions/0041-published-method-invocation.md)
records private service authority, stable child association and nonblocking continuation. These
explain design pressure; they do not prove present optimality. History also shows later corrections:
`ea8e21a` moved direct preparation from CLI to serving, `b446b74` added public publication preparation,
and `d5130e8` preserved queued calls during retirement. There is no evidence here that every original
bound or required agreement came from an observed user failure rather than the original exercise.

## One demanding case, with independent expected outcomes

Use two daemon owners A and B and two caller modes. B has a private method revision R implementing
`repair.inspect-and-build`, an approved service identity S, a managed installation with working
area W at generation 7, and one execution worker. Caller H may invoke the service and read its
declared result; H cannot inspect R, obtain its secret, administer B or choose arbitrary paths.
The workflow mode is a real run on A with account Q; the direct mode is the standalone client
calling B's authenticated direct endpoint. A direct call to A that relays to B is deliberately not
assumed: the present remote adapter requires workflow provenance.

Both modes use the same declared input artifact and exact published version. The private internal
run has two steps: a child edits W, then another step uses its result. Lose the reply after accepted
association, then lose physical stop proof after the first child's entry. Revoke the caller or
service grant, restart B, replay the original request, and attempt editing/maintenance. Later supply
authorized physical fencing evidence. These are fixed task inputs for the comparison, not executed
new tests.

Expected outcomes are independent of representation:

1. Exactly the originally accepted service version and internal child identity survive reply loss;
   no replacement work or broader grant is chosen.
2. Direct calling creates only the method's real internal run, not a synthetic outer workflow.
   Workflow calling retains A's real scheduling and account facts.
3. One worker is sufficient for the internal child: waiting on its result does not occupy that
   worker. B's private internals/secret remain undisclosed to H.
4. The lost stop proof retains W's lifetime hold and child's editing exclusion after cancellation
   and restart. A terminal workflow report by itself cannot authorize a new writer.
5. Grant change denies further governed entry and current prohibited disclosure. It does not
   delete the acceptance, reset Q, infer that the child stopped, or retrospectively change R.
6. An authorized resource operator can establish physical quiescence and settle the old ownership
   without manufacturing successful work. A revoked parent cannot resume editing under its old
   authority. Exact replay still conflicts on changed bytes.

### Current design: the complete path

Direct: daemon authenticates H; host resolves the client basis and catalog in
[`serving/direct.rs`](../../../../../crates/capability-host/src/serving/direct.rs).
`invoke_client` looks up the actor-scoped request before fresh catalog checks, binds replay to its
original grant, checks current inspection permission, then calls `accept_serving`. Workflow:
A's runtime resolves a peer capability; [`RemoteCapabilityAdapter`](../../../../../adapters/peer-http/src/remote.rs)
retains B's exact descriptor, strips B's managed binding from A's local projection, and sends a
targeted delegation with workflow origin and reservation. B's `invoke` follows the same durable
`admit_peer_execution` boundary. A and B retain different accepted facts; importing B's W binding
as A's inventory would be incorrect even if names match.

B acquires and prepares the exact generation. [`PublishedWorkflowService::prepare`](../../../../../crates/control/src/published.rs)
validates declared inputs and service relationship, creates a deterministic child identity, and
constructs exact create/start/cancel commands without executing them. The serving entry transaction
saves the plan and resource/account facts, records `NoExternalEntry` for the publication wrapper,
and moves to `AwaitingWorkflow`. A local workflow calling a local publication instead records the
same plan in its runtime-owned pending effect; it does not add a second local serving row.

[`continue_published`](../../../../../crates/capability-host/src/serving/published.rs) advances the
saved plan in bounded pages after releasing the execution worker. Control calls
[`arrange_published_run`](../../../../../crates/runtime/src/engine/effects/published.rs): replayable
create, bind/account, then start transactions recover the same real run. These transactions cannot
simply be deleted because a single Rust service owns their caller: acceptance can commit before
the child exists, and cancellation can arrive between creation and binding. Pending association is
an obligation, not a second current revision.

At each internal adapter entry runtime rechecks the enclosing plan, current caller authority,
deadline/cancellation and service basis, before and after preparation. At a serving ancestor it
asks the serving owner; a missing owner fails closed. In the resource transaction, an exact linked
child can take editing only from the wrapper's proven quiescent claim. A physically entered child
needs `PhysicalStop`, not wrapper `NoExternalEntry`, before return. The current resource owner
retains one suspended-child relationship per use.

On restart, B restores retained publication generations and pending calls; the same plan drives
the same commands and account. Lost stop evidence keeps the child claim and parent suspension.
Continuation withholds public completion while the parent use is suspended, even if the internal
workflow has a terminal outcome. Public outputs copy only declared accepted fields through the
artifact/account owner. Public cancellation acceptance never claims that the physical writer ended.

The path is substantively implemented. The exact combination above is not newly executed here;
its components have tests discussed below. Existing tests do not justify presenting the combination
as a fresh real multi-host or physical-stop qualification.

### Complete smaller correction

Keep those durable boundaries and wire identities. Change three concrete implementation seams:

1. In host's private serving module, introduce one `FreshServingAdmission` construction path after
   caller-specific authentication and replay/disclosure handling. It checks drain, deadline,
   catalog selection/operation and exact descriptor, calls the appropriate current authority
   evaluator, then supplies the existing `PeerAdmission`. Direct and peer public methods retain
   their distinct credential/replay access rules and call that path. Delete duplicated fresh
   admission branches; do not move authentication to persistence or merge distinct rate scopes.
2. Put managed-use transition meaning in `persistence::managed`, next to `ManagedUse` and the
   transactional port, as pure functions over typed borrowed current facts. Redb collects exact
   owner facts inside its write transaction and applies the resulting transition in that same
   transaction. Move policy from `managed/execution.rs::{acquire,linked,parent_authorized,
   parent_cancelled,pending_publication}` and `managed/uses.rs::{transfer,quiesce,release}`;
   retain redb table/index access, receipt guards and commit faults there. The fact input must
   contain actual accepted association/basis/cancellation and matching source identities, not
   caller-supplied `authorized: true` or `child_is_valid: true` Booleans. This is the same ownership
   pattern already used by the pure account transitions, not a new service or durable journal.
3. Add a capability-owned checked descriptor reconstruction method for replacing extensions
   while preserving the other immutable fields. Migrate managed model and published descriptor
   decoration away from `serde_json::to_value` → `get_mut("extensions")` → `from_value`. Keep
   typed extension payload serialization at its real wire boundary; remove dependence on the
   descriptor's serialized object layout. Peer remapping still deliberately changes identity,
   locality, operation visibility and resource scope through `DescriptorBuilder`.

The demanding case has exactly the same persisted events, accepted identities and user choices.
The material difference is who defines the rule: one admission path and one managed transition
owner can be tested from the expected outcomes before any database effects. No new distributed
transaction or projection synchronization is introduced. All producers/consumers adopt the changed
Rust interfaces together; this is not a second execution path hidden behind an adapter.

The intended Rust surface is a `managed::transition` module with a closed `ManagedUseAction`
for the current acquire/entry/no-entry/quiesce/handoff/return/release meanings, and a bounded
`ManagedLineageEvidence` input that borrows the exact local accepted events and authority basis or
the exact serving record/publication plan. Its `apply` function returns the revised
`InstallationRecord` and changed use identities, or a refusal. Redb derives its physical index
updates from that result. The pure function creates no durable evidence on its own.
Redb obtains that evidence and checks revision/claim guards in the same write transaction; an
unrelated caller cannot construct a convincing linkage merely by passing an allowed decision.
Keep the result private to this storage-port boundary and do not expose these actions as another
public daemon command family. Existing `ManagedResourceStore` methods remain the product's atomic
operations. Exact function factoring is an implementation choice; semantic owner, evidence source,
atomic guard and deletion of the old decision bodies are settled by this proposal.

### Strongest ownership alternative: one invocation owner

A credible alternative makes a host invocation service the sole owner of every external attempt,
including local workflow attempts. Its durable `InvocationRecord` would include source
`Direct | WorkflowAttempt`, exact selection/inputs, acceptance, prepared-entry commitment,
cancellation, observations, resource-use references and account reservation. Runtime would own
eligibility, revision, node completion and causal selection only. Serving peer authorization would
be a source/transport claim, not another local execution state machine. Publication would be an
invocation whose implementation is a real child run. Physical installations and grants would still
have separate owners.

For the demanding case, direct B acceptance writes that record; A writes its own local invocation
for the peer call and B writes a remote accepted record, since separate hosts cannot share a local
ledger. B's publication record saves the same child association before child creation. Its
continuation releases its worker and schedules the child. Every internal process on B is now
another invocation row. Lost stop evidence lives in that row plus the independent resource claim;
revocation is checked by the invocation owner before new entry and runtime before prospective
control. Restart scans accepted invocation records and resumes the same child/observation work.
So far, all six expected outcomes are achievable.

The hard part is local atomicity. Runtime must commit “attempt exists and may enter,” account
reservation, resource acquisition and the invocation acceptance together. The strongest version
therefore adds an invocation action to `AtomicRunCommitRequest`; redb applies it in the same write
transaction. An outbox variant could also be correct, but adds an extra pending state and recovery
worker to every local attempt. Neither may let runtime claim a dispatched attempt whose invocation
record never existed. The invocation owner would later append a durable report reference; runtime
must consume it exactly once and save its observation cursor/result in the same local transaction
as node progress. A terminal invocation is not yet a completed node if output acceptance or
structured cancellation has not committed.

This replaces the two local entry/report implementations with one. It also creates an obligatory
association and report-consumption protocol between runtime and invocation owner for every local
process/model call. It does not remove A/B records, installation inventory, account transactions,
publication child creation or grant evaluation. Its durable event names could preserve raw
observation meaning without duplicate authority, but “one table” would not make the workflow's
derived completion or physical stop interchangeable.

If selected, delete runtime's owning attempt-entry/report paths and local publication continuation,
retain only immutable dispatch/consumption references, migrate snapshots and source readers, and
make the invocation service the only adapter reporter. A temporary local bridge cannot remain
another valid owner. Migration would require a stopped supported store conversion and exact
mapping of every active/uncertain attempt and archived request; changing only new runs leaves two
permanent ownership models. That is substantial cost, but cost alone is not the reason to reject it.

The reason to prefer the smaller correction is semantic concentration: workflow entry is already
atomic with the workflow/account decision, whereas direct/peer serving is already independent.
The unified service saves repeated transition plumbing while introducing a new completion boundary
in the predominant local workflow path. The inspected repetition is not yet evidence that those
two lifecycle policies are diverging. The recommended pure transition/admission changes remove
specific duplication without introducing that boundary. Reconsider if independent durable
invocation inspection/control becomes a required product operation for all local attempts, or a
third accepted caller family cannot fit the current two owners. This is a conditional design
trigger, not authorization to wait for future implementers to decide the present architecture.

## Publication ownership and user burden

Putting all publication code in runtime looks attractive because the child is a run. It would make
runtime own capability registration, service configuration and public artifact copying, or require
new ports for all three. Putting it in host gives host workflow validation, command construction
and private terminal-result meaning, defeating independent execution-only composition. A standalone
publication service crate would add a package without an independent durable owner or required
external consumer. The current control composition is warranted: `PublishedWorkflowService` owns
the explicit service relationship, validation, registration, orchestration and disclosure; runtime
owns child facts, host owns invocation acceptance and adapter lifetime.

The continuation port is useful dependency inversion, not a redundant journal. Its weak reference
means it cannot keep a stopped workflow owner alive. `PreparedAdapterAction::Published` differs
from an external one-shot closure because no worker should wait synchronously for a run. Deleting
that distinction would deadlock or require another scheduler. Keep it closed and explicit; do not
generalize it into a universal plugin continuation framework on the evidence here.

There are nevertheless avoidable operator choices. The ordinary caller should choose the method,
inputs, owner and acceptable budget; a prepared request should supply catalog/digest/generation and
exact request identity. The publisher chooses the permitted input contract, disclosed outputs,
service relationship and ceilings; the daemon derives saved grant digest and governing agreement
from actual owners. The existing direct/publication preparation operations already move this work
server-side. Do not turn their internal fields into required GUI form fields. An authorized resource
operator, unlike an ordinary caller, must deliberately choose preservation, interruption and fencing.

The current `PublishedInput` supports finite choices and bounded artifacts. That constraint helps
prevent a caller choosing arbitrary paths/targets, but it is not a general theory that all private
methods have only those inputs. Likewise every publication currently requires a governing agreement.
Source and ADR establish deliberate implementation, not that an ordinary read-only reusable method
must incur the entire governed-adaptation authoring ceremony. For the selected demanding case the
service contract is justified; whether a simpler remote same-authority revision call is needed must
be resolved in the reuse design rather than silently widening this service route. Geography alone
does not justify a grant transition.

## Repeated rules and a future change

The meaningful conversion surfaces are not all waste:

- Remote descriptor mapping removes foreign managed ownership, restricts advertised operations and
  adds peer provenance. The mapping must remain even if Rust names/types are unified.
- Serving rewrites a caller-controlled invocation ID to a local acceptance-bound ID and rewrites
  reports back. This prevents distinct callers colliding in one adapter registry. Keep the mapping
  and test it; do not deduplicate on equal caller text.
- Workflow input selection and direct explicit inputs have different provenance promises. Both use
  the same materialization port, but a direct request must not manufacture causal history.
- Provider wire mappings convert a provider-neutral request to distinct external contracts and
  reject unsupported features. Folding them into one permissive JSON passthrough would transfer
  unsupported assumptions to workflow authors.
- Managed model wraps the ordinary `ModelEndpointAdapter` to add exact resource entry and stop
  ownership. It already reuses the provider mapping. Its wrapper is justified; its descriptor JSON
  object surgery is not.
- Pure account decisions being recomputed against a guarded database revision are verification of
  an atomic acceptance, not two balances. A budget shown by authority is per-request permission;
  cumulative settlement has a different owner and units. Neither should masquerade as the other.

Use a realistic future change to compare costs: one installation contains two distinct working
areas, and two children should edit disjoint areas concurrently while the parent retains both
lifetime holds. The independent rule would still forbid two writers to one area, forbid parent
editing of a delegated area, and require exact child stop before its area returns. Current
`ManagedUsePhase::Suspended { child, evidence }` permits one suspended-child relationship per use;
this is a concrete representation limitation, not proof that every installation needs concurrency.

Under the current design, changing it requires managed state/wire/read views, the redb acquisition,
linkage, transfer, return and integrity rules, host orchestration, Linux consumers of claims, and
publication completion's single `Suspended` check. The rule is interleaved with table access, so a
database refactor and product-policy change are difficult to review separately. A generic unified
invocation ledger still needs the same resource claim redesign and publication quiescence check;
one invocation row cannot express simultaneous ownership of different physical areas by itself.
It additionally must preserve runtime report-consumption behavior during the new child pattern.

With the recommended transition owner, the future meaning changes in one pure managed-use
transition module and its state contract; redb supplies the same exact owner facts and writes the
new result. Host/platform code still changes when the physical contract changes—there is no honest
zero-cost abstraction—but existing transaction and lineage oracles can test the new state without
redb-specific setup. A typed resource-owner query such as “has unsettled delegated editing” replaces
control's direct pattern match on a particular phase when that representation changes. This is a
specific direction for future extensibility, not authorization to implement multi-child editing now.

## Tests as evidence and as assumptions

The independent oracles for the demanding case are the six outcomes above. These existing tests
support parts of them; no tests were run in this review:

| Source/assertions examined | What it establishes in its fixture | What it does not decide; discriminating variation |
| --- | --- | --- |
| [Direct process public test](../../../../../apps/daemon/tests/control_plane/direct.rs), `direct_process_upload_replay_and_restart_have_no_workflow_records` | Production adapter appends input, exact request/restart does not add workflow history, caller artifact isolation and quota/refusal assertions. | Does not prove direct A→B relay or private-method authority. Keep “no synthetic outer run,” not “no run anywhere” for a real method implementation. |
| [Publication recovery](../../../../../crates/control/tests/control_service/published/recovery.rs), `each_create_bind_start_commit_boundary_recovers_one_child_after_reopen` | Before/after faults at each of three command boundaries produce one `RunCreated`, one binding and exact association; no extra process entry. | The number three is implementation detail. An alternative with one atomic bind/create must preserve one real child and exact outcomes, not imitate three faults to pass. |
| [Managed publication](../../../../../crates/control/tests/control_service/published/managed.rs), lost-stop and authorized-return tests | Fixture retains child claim/parent suspension across cancel/reopen, refuses unsafe parent entry, and can later settle with explicit resolution. | A scripted stop observation is not physical Linux proof. Vary stale claim, another child, same resource name at another host, grant revocation and disjoint resources. |
| [Final-entry test](../../../../../crates/control/tests/control_service/published/entry.rs) | Cancellation/deadline changes during child preparation prevent entry; managed refusal releases safe pre-entry holds. | It directly covers cancellation/expiry, not every grant-replacement producer. Add an exact-grant revocation variation with unchanged request bytes to the same boundary. |
| [Peer publication](../../../../../apps/daemon/tests/two_daemon_peer/published.rs) | Actual daemon compositions use ordinary peer adapter, one real remote internal run, linked serving source and accepted transferred output. | Operator-rich fixture does not independently prove an invoke-only caller cannot inspect private internals. Pair with narrow public-control refusal fixtures and a combined narrow case. |
| [Account transition tests](../../../../../crates/persistence/src/controller_account/tests.rs) and [nested tests](../../../../../crates/persistence/src/controller_account/tests/published.rs) | Exact limits, unknown usage retention, late settlement, zero dimensions and nested allowance without double charge are explicit assertions. | Arithmetic/property coverage does not show every adapter reports sound bounds. Test identical account reached through nested references and unknown remote settlement before concluding aggregate control. |
| [Managed lifecycle](../../../../../adapters/managed-linux/tests/lifecycle.rs) | Controlled platform faults at intent/result boundaries preserve exact setup/replay; deny authority before platform action; bounded concurrent replay has one driver. | In-memory platform observations do not qualify systemd/Podman or power loss. Native tests/evidence remain separate. |
| [Authority contracts](../../../../../crates/authority/tests/contracts.rs), selectors and workflow scopes | Equal actor/grant facts evaluate identically, containment/refusal and exact grant identity tested. | Truth of facts supplied by every caller is outside the evaluator. Vary caller/service grant independently; never test by giving both the same broad administrator grant. |

The implementation merits targeted observations, not another unchanged whole-suite run for planning.
Safe discriminators, if the coordinator authorizes an existing local fixture observation, are:
an invoke-only direct and workflow call to the same private service; revocation specifically during
preparation; a same-name foreign managed binding that never resolves locally; and fault after child
terminal but before physical stop/claim return. New comparative designs remain modeled until
implemented. A source-shape check counting journals cannot decide their semantics.

## Concrete adoption units and supported state

These are complete proposed units for the revised program, not production work performed now.

1. **Managed transition ownership.** Add the pure transition/fact-validation module to existing
   `persistence::managed`; migrate all redb managed acquisition, entry, no-entry, quiescence,
   handoff/return and release decisions in one unit. Keep host platform-driving and redb atomic
   evidence loading/receipts/indexes. Cross-check all actual consumers, including publication
   continuation, ordinary attached children, managed worker/model guards and administrative
   resolution. Delete moved decisions from redb. Retain current serialized records and errors;
   use unchanged golden bytes, current-writer reopened fixtures, stale/forged fact cases and
   before/after transaction faults. The intermediate product still serves existing direct,
   workflow and published operations. This extraction should not need a storage generation change.
2. **Serving admission convergence.** Introduce one private fresh-admission pipeline and migrate
   `invoke` and `invoke_client` together. Retain caller-specific authentication, replay disclosure
   and rate semantics. Delete duplicate deadline/catalog/selection checks and prove both caller
   families produce equivalent refusal for equivalent governing facts, while different principals
   cannot collide. Existing durable bytes, request digests and archived replay must stay exact.
3. **Typed descriptor decoration.** Add the narrow validated capability API; migrate publication
   and managed model construction, and inspect other descriptor producers for the same pattern.
   Remove JSON field-path mutation. Prove canonical descriptor bytes/digests are unchanged,
   malformed/oversize extension refusal remains, and remote mapping still removes foreign managed
   ownership. This is a Rust API correction, not permission for a new wire schema.
4. **Complete consumer evidence.** Exercise the demanding case through public direct and workflow
   routes with narrow distinct caller/service/operator grants, one worker, current supported
   redb reopen, lost reply, lost stop and retained allowance. Inspection should expose actionable
   accepted/uncertain/blocked facts without private method content. The normal executable gate
   belongs to implementation acceptance, with these focused fault cases in the owning phases.

For these corrections, current supported stores stay writable by the same semantic generation:
there is no reason to stop active/uncertain work merely for moving pure logic. Before claiming byte
compatibility, compare exact readers/writers and golden fixtures; do not infer it from unchanged
field names. Deploying a new binary still requires the existing orderly owner shutdown/reopen
procedure. A missing physical-stop proof remains held across that process. Older unsupported
pre-release generations retain their existing refusal/preservation treatment; this plan does not
invent their migration or destructive cleanup.

If the broader program selects a unified invocation service instead, unit 1 alone is insufficient.
It needs an explicit offline conversion: close admission; stop/fence or retain every external
obligation; copy the supported store under the existing admin owner; transform each accepted local
attempt into one stable invocation identity while preserving old receipt bytes and events; validate
links/accounts/resource claims; activate only the converted generation; retain the source for
inspection. Uncertain work cannot be rerun or silently drained by migration. The old runtime entry
writer must be removed, and rollback must not open the new generation with an old writer. Until
that conversion and its refusal semantics are specified and verified, the unified alternative is
not an implementable selected direction.

The proposed corrections preserve valuable behavior while changing actual responsibility and
construction. They also bound this review honestly: the twelve packages are covered at stated
depth, with complete execution/resource alternatives, not approved because neighboring tests pass.

Document checks performed for this contribution: all linked local files exist; the added-file
whitespace check reported no errors. The coordinator owns integrated documentation contracts.

## Faris cross-review of agent-directed work

This section was added after recording the independent execution position. It challenges Elin's
[agent-directed proposal](agent-directed-work.md), especially selection B, using the original
[U15/U16](../intent-source-excerpts.md#u15--adaptability-and-dependable-reuse) and additional source
inspection. It is attributed criticism, not Elin's agreement or a newly executed experiment.

### B is a useful primitive but an incomplete continuous-work decision

I accept the source finding that current evidence drivers supply orchestration missing from the
product. I also accept moving model-response authentication into control and preserving ordinary
proposal/reconciliation ownership. Those conclusions do not establish that a separately authorized
assistance round is the right highest-level product unit.

The selected B asks the human to initiate each criticism/answer round and grant a new allowance,
then expressly declines a project-wide budget. That is a real reduction in promise against the
user's wish for continuously evolving work and this investigation as a representative workload.
The reopening request says every action need not be autonomous; it does not establish that the
human should remain the only owner of the transition from criticism to reconsideration. Repeating
the same “reconsider,” allowance and target-selection ceremony is precisely a part of manual
shepherding Milkdrift originally aimed to remove. B can serve that action without serving the full
continuous process.

My dissent is therefore about **product selection**, not a request to reject the assistance
compiler. Prefer C when the accepted promise is “continue this bounded investigation and respond
to criticism until its stopping condition or human decision.” Reuse B's packet/model/admission
work as operations inside that journey. B by itself is a reasonable first usable intermediate
state only if the revised program retains the continuous session as committed scope, or the user
explicitly chooses the narrower product. The source cannot decide that product preference.

The strongest C is not necessarily a second scheduler. An ordinary retained root run can own
round occurrences, typed answer/criticism signals and the cumulative account. Its children carry
the actual model, evaluator and accepted target work. A read-only session projection can expose
“waiting for answer,” “candidate ready,” “critic changed the premise,” and “budget exhausted” from
their accepted facts. It need not copy all artifacts into a global conversation-memory store or
make a mutable plan table authoritative. Root cancellation and child uncertainty still use runtime
and resource owners. This version of C deserves comparison before citing the cost of a second
session lifecycle as a reason to select B.

There is concrete current support and a concrete remaining extension:

- [`RunCommand::DeliverSignal`](../../../../../crates/runtime/src/command.rs) carries typed,
  correlated, idempotent bounded payloads; [`command_planning/admission.rs`](../../../../../crates/runtime/src/engine/command_planning/admission.rs)
  retains consumption and `WaitSatisfied`. Answers need not live in a second mutable mailbox.
- [`extend_controller_actions`](../../../../../crates/runtime/src/engine/command_planning/commit.rs)
  derives `BindRun` actions from accepted `SubworkflowCreated` facts in the parent transaction.
  It rejects a conflicting nested account. [`structured/subworkflow.rs`](../../../../../crates/runtime/src/engine/structured/subworkflow.rs)
  recovers the same child and checks its creation revision, scope, inputs, inherited authority and
  agreement. Those are viable building blocks for cumulative session limits.
- [`RepeatConfig`](../../../../../crates/blueprint/src/model/structured.rs) and the current
  `Subworkflow` node pin a revision. They do not dynamically execute whatever revision a planner
  returns. A fixed controller wrapper alone therefore does **not** implement C. The proposal must
  choose either prospective adoption of an unstarted child-call region or a narrowly defined
  runtime-owned child association for a validated selected revision. Arbitrary control `CreateRun`
  followed by an application-side account label is not an acceptable substitute.

This is a foundational completion requirement: the revised program must name the selected child
operation, its acceptance event, account/authority inheritance, cancellation and reopen behavior.
It cannot be delegated to an implementation agent as “compose ordinary tasks.” If C uses a new
association, runtime must commit it and the inherited account together before child entry; control
may submit the operation but cannot append a child or account binding directly. If it uses ordinary
prospective adoption, it must identify the unstarted region and how the active coordinator's own
reports avoid making its proposal guard perpetually stale. A separate target child, with explicit
pause and exact guard, remains useful even when coordinator and target share an originating account.

### Both B and C need exact start and admission recovery

The assistance proposal says `StartAssistance` accepts a packet and inspection reconstructs its
run/proposal references. It does not yet specify the acceptance that permanently binds the packet,
selected revision, assistance run ID and exact create/start command identities before work begins.
Without it, a crash between creating the run and saving the outer response can leave an orphan
or cause a retry to allocate another run. An ordinary final application receipt is not by itself
proof that every preceding effect was discoverable before the receipt committed. The coordinator's
application trace must supply this exact association or the design must add it.

The complete resolution should mirror the justified part of publication recovery: immutable
accepted assistance intent binds the packet and deterministic run/create/start identities; replay
recovers those commands, never chooses another run. The association is carried by the existing
owning application/runtime acceptance, not a second mutable assistance journal. Inspect follows
it without relying on a client checkpoint. For C, an attached round's parent acceptance and account
binding must commit together; for standalone B, the command acceptance names its separate allowance.

The response-admission stage has a second loss boundary. [`ControlService::submit`](../../../../../crates/control/src/service.rs)
can save the candidate revision and request reconciliation before returning a result. Its
[`WorkflowControlAdapter`](../../../../../crates/control/src/adapter.rs) subsequently publishes
the result and reports terminal evidence. [`runtime recovery`](../../../../../crates/runtime/src/engine/recovery.rs)
classifies ordinary entered attempts by their retained side-effect/idempotency facts; it does not
magically infer an assistance candidate from those separate commits.

The new admission operation therefore needs a stable command/proposal identity derived from the
accepted assistance request and exact model response, preserved across admission retries. Its
recovery reads/replays that command and deterministic result publication. Losing admission output
must never restart the upstream model. If current authority no longer permits continuation or
disclosure, inspection must preserve the accepted/refused/uncertain distinction without leaking
the candidate. Tests must separately lose (a) start reply, (b) model terminal, (c) proposal commit
reply, and (d) admission artifact/report. A restart-after-model test alone does not establish all
four. This requirement applies to both B and C and is compatible with the retained owners.

### Ordinary-result evaluation: retain the idea, close its admission contract

The existing [`LearningDeclaration`](../../../../../crates/control/src/learning.rs) really does
require protected targets, verifier/check digests, separate managed resources and repair reduction.
It cannot evaluate an ordinary decision brief merely because both are called learning. A finite
`PairedResultJudgment` profile is a justified addition for the product's research/planning path;
keeping it separate from the protected profile is better than making all protected fields optional.
This recommendation is a new product contract, not evidence borrowed from Slotbook qualification.

Several foundational details remain in the proposed profile as written:

1. **Declaration order.** A pre-proposal declaration cannot contain the unknown candidate revision,
   and ordinary scheduling generates actual attempt identities. Reserve named evaluation slots
   and exact run/request identities first. Bind the candidate revision through its accepted
   candidate receipt before running candidate slots; bind actual attempts/output producers from
   the owning accepted journal afterward. Do not require the evaluator to predict scheduler IDs.
2. **Judgment ownership.** Decide whether a slot has one immutable judgment or supports amendments.
   Recommend one immutable accepted judgment per declaration/candidate/case/rubric/evaluator key;
   changed bytes conflict. A correction is an explicit superseding judgment, and a comparison
   pins the exact chosen receipt rather than “latest judgment.” Distinct command IDs must not
   permit selecting whichever of two conflicting verdicts makes the candidate win unnoticed.
3. **Isolation.** Immutable input equality and different run IDs do not prove independent writable
   state. For the first ordinary-result profile, allow read-only/model-only work or require exact
   separately owned working areas for effectful slots, using existing resource claims. A baseline
   process that edits the candidate's checkout invalidates the comparison even if both outputs
   have correct artifact digests. Native trusted tools retain their stated limitations.
4. **What independence means.** Separate evaluator authority from proposal authority, fix the rubric
   before proposal, and preserve the evidence each judge saw. Different actor strings alone do
   not establish independent reasoning or absence of shared model context. The product can verify
   the declared authority/context separation; a human judgment remains an attributed judgment.
5. **Interrupted orchestration.** `Compare` is a verifier of recorded slots, not their scheduler.
   B/C must name who creates and starts baseline/candidate runs and how their allowed budgets and
   exact inputs bind before entry. For C those are children of the session with finite allocated
   allowances; a separate authorized evaluator can provide hidden inputs through the ordinary
   scoped boundary. The session's proposer cannot gain those inputs merely by owning the session.

I support `ApproveLesson` as distinct from publication promotion: accepting guidance need not
create a callable service or invent an installation. It must bind the exact comparison and
guidance artifact, keep the comparison's finite scope visible, and remain a separate authorized
decision. Existing protected promotion keeps all verifier/target and publication-authority checks.
General evaluation orchestration belongs in control; daemon keeps authentication/wire adaptation,
runtime executes slots, and persistence keeps exact receipt/association facts.

For compatibility, “map v1 to the protected profile” must mean a read projection preserving original
canonical bytes and replay result. It must not deserialize old bytes into a new enum and rehash them
as a different accepted declaration. Keep the old request reader for exact historical replay before
new-admission rules are applied, or explicitly specify refusal without mutation and how original
results remain accessible. This is separate from graph-source migration; successful conversion of
blueprints does not establish preservation of learning receipts or their grant-bound disclosure.

### Cross-review outcome

B's control-owned packet and authenticated response admission should survive into either product.
I do not endorse B alone as having resolved the continuous-work ambition. Prefer an ordinary-root
form of C for bounded ongoing work, provided the revised architecture explicitly owns dynamic child
selection and the four loss boundaries above. The ordinary-result evaluation profile is a useful
direction with five concrete contract corrections, not an already complete reuse of current learning.
None of these observations requires another global invocation ledger or moving publication/resource
ownership. No fresh runtime/provider experiment was performed for this criticism.

### Reconsidered selection after U17/U18 and the complete C comparison

Faris-20261010, `/root/execution_faris`, after reading the user's exact
[U17/U18 directions](../intent-source-excerpts.md#u17--human-reviewed-knowledge-and-precise-evidence),
Elin's completed C and Rowan's [application reconstruction](application-reconstruction.md).
This changes the conditional root-run preference above. The earlier position remains visible so
that the objection and the evidence that changed it are not rewritten as prior consensus.

**Select C with a control-owned `SessionOwner`.** The user selected bounded automatic ongoing
work, not this owner. My architectural reason is the missing independent lifetime of accepted
next-work intent: a commitment must retain criticism and decide permitted future actions while
its current work run is running, paused, being reconciled or already terminal. The session
record owns that intent and its exact pending commands. Runtime still owns each run, its nodes,
attempts, waits, effects and historical results. This is a new bounded orchestration state
machine; describing it as having no orchestration or scheduling responsibility would obscure
its actual cost. It schedules whole authorized activities, while runtime schedules their execution.

The strongest alternative remains one ordinary root run. Its real advantages are an existing
durable identity, signals, inherited account, parent cancellation and fewer new storage contracts.
However, [`observe_child_terminal`](../../../../../crates/runtime/src/engine/structured/subworkflow.rs)
returns immediately for nonterminal children and imports outputs only from terminal evidence.
[`create_repeat_iteration`](../../../../../crates/runtime/src/engine/structured/repeat.rs) invokes
the controller lifecycle at activation/cycle entry; its human checkpoint follows a completed
true-condition frontier. This is not a callback when a live child reaches a pause or receives
criticism. An enclosing Parallel can keep a separate critic active, but does not itself expose the
new child's exact handle, nonterminal evidence contract or a durable join-free next-round state.

Delta's `CallTarget::AdmittedPlanResult` would correctly resolve a validated selected revision,
freeze its receipt/authority/interface at child acceptance and never resolve a different target
on retry. It does **not** close that nonterminal coordination gap. Full root-C additionally needs
an early child handle, scoped active-child observation, a coordinator continuation while the
child remains active, and cancellation/reconciliation rules for both branches. General asynchronous
child handles can be worthwhile in a future workflow language, but S28 does not independently
justify adding that new ordinary workflow semantic alongside dynamic call and the source migration.
Do not include dynamic Call merely to keep session accounting attached to a synthetic root.

Under selected C, the critic's declared output or authenticated `SubmitCriticism` references exact
evidence and target associations. The session can request an ordinary pause and observe the
resulting target frontier without waiting for the target to become terminal. It then prepares
a fresh, bounded planning packet. Criticism is an attributed claim; it does not directly edit
the method or satisfy an approval. Resuming, prospective adoption, retries and resolution of
uncertain work remain ordinary authorized operations. A revoked target-read permission can
block the packet even when the caller may still inspect the session's own accepted intent.

The selected boundary requires these concrete additions; they are part of C, not work for an
implementation agent to discover:

| Owner | Required accepted contract | What remains elsewhere |
| --- | --- | --- |
| `control::session` | Immutable policy, exact accepted decisions/pending commands, bounded phase projection and pure `plan_session_transition`; one-round assistance is a policy of this owner | No provider execution, node lifecycle, copied run status or physical-stop judgment |
| `authority` | Session identity scope and closed operations for create/read/criticism/answer/pause/resume/cancel/policy amendment, plus a frozen session actor/grant basis | Each target read, proposal, approval, run start and capability entry still evaluates its actual existing operation and resource facts |
| `persistence::session` | Bounded `SessionActionRecord`, exact `SessionExecutionAssociation`, accepted policy/admission generation and journal/receipt port | No second model transcript, artifact store or invocation ledger |
| `persistence::controller_account` with renamed new contract | Closed `ExecutionAccountOrigin::{ControllerOccurrence, PublishedInvocation, WorkSession}`, session establishment without a pretend run, unchanged reserve/settle/unknown-use algebra | Session rounds do not reset or privately estimate consumed allowance |
| runtime plus redb atomic commit | Exact association-backed run creation/account binding, normal run execution basis and current session admission fence before start/entry | Control cannot append arbitrary `BindRun`, task or attempt facts |
| daemon | Authentication/wire DTO translation, external exact receipts, bounded continuation pages and current authorized projections | Browser/evidence drivers no longer own stage/retry/proposal tracking |

The authority addition is necessary. Current
[`ExecutionAuthorityBasis`](../../../../../crates/authority/src/model/execution.rs) requires a
real workflow/root run/revision and derives from an accepted start decision. Do not weaken that
contract to encode an invented session run. `SessionAuthorityBasis` retains the creator's exact
actor/grant/policy basis and the accepted policy envelope. Every associated run obtains its own
normal start decision under that basis, narrowed by the session policy and checked against current
revocation. Association gives neither the observer nor the session actor implicit run/artifact
read, approval or publication permission. Versioned grant readers must not turn a missing session
scope in an old grant into newly granted control over sessions.

The account change is more than an enum rename. Current
[`ControllerAccountTransaction`](../../../../../crates/persistence/src/controller_account/transaction.rs)
requires establishment to bind the declaration's originating `RunId`. Current
[redb validation](../../../../../adapters/redb-store/src/controller_account/validation.rs)
matches inherited bindings exactly to `SubworkflowCreated` events. Preserve those old checks.
Add one typed session establishment path and one association-backed creation path: a session
creation transaction retains policy, external receipt, account and first action together;
runtime creation validates the exact accepted action/association and atomically writes run creation,
the session association provenance and its existing account binding. Start accepts the ordinary
run execution basis under its own current authority decision. A caller-supplied account ID or
boolean `is_child` cannot authorize this path. The unstarted run after a crash is recoverable
from those exact facts; no subsequent client checkpoint is needed.

Cancellation needs an admission fence, not merely eventual fan-out. Session pause/stop commits a
new closed admission generation before scheduling ordinary pauses/cancels. Each session-associated
start and external entry checks that current durable gate in its owning acceptance transaction;
otherwise `create -> session cancel -> delayed start` could admit fresh work. Descendants find
the originating session through their exact account/association chain. Accepted external work,
including remote service work, remains accepted and may remain uncertain. Resolution and terminal
settlement stay admissible under their separate authority; `Stopping` is not proof of physical stop.
An action selected before the fence but not entered remains pending/refused according to its exact
accepted command, never regenerated as a new request behind the operator's stop.

There is also a finite controller-composition limit. Current
[`ControllerLifecycleOwner::assess`](../../../../../crates/control/src/controller/lifecycle.rs)
accepts another origin only for a published invocation whose whole allowance fits the marked
controller's budget. Otherwise it refuses a conflicting originating declaration. Extend that
existing inherited-origin rule to `WorkSession` only when its whole allowance fits the marked
method's limits. Refuse a non-fitting local association before start; the existing published
service route can provide its declared bounded internal allowance while retaining the caller's
outer reservation. Ordinary session-authored methods need no controller marker merely to obtain
an account. Do not silently add hierarchical mutable budgets or reset the session account to run
such a method. Preserve the limitation visibly until a separate complete suballowance design earns
its cost. Independent cycle/proposal limits remain restrictions of the marked method.

The initial session policy fixes the originating budget. A separately authorized future
requirement change can narrow permitted work while retaining prior usage and reservations. A
budget increase would require an explicit account-amendment contract preserving those totals;
creating another allowance under the same label is not that contract. It is not needed to deliver
the bounded U18 journey and is not silently authorized by `ResumeWorkSession`.

### Application receipts and supported-state consequences

Rowan's receipt finding survives adversarial review. The daemon binds the exact external actor,
grant and canonical envelope to a response, including refusals and administrative snapshots.
Runtime semantic retry identity deliberately omits certain delivery/planning fields so its own
replanning can proceed. These are different promises, not duplicate authority over one fact.
Keep both. Exact deterministic IDs alone do not close partial commit gaps if recovery reconstructs
different command meaning; C retains complete prepared command material before invoking its owner.
After proposal acceptance and report loss, replay/query the same proposal and publish the same
derived result. Never regenerate the model response to repair a missing application receipt.

R-A05 also changes: `ListMethods` and `InspectMethod` are retained administrative snapshot commands,
not the invoke-only catalog. The proposed universal filter remedy was based on the wrong contract.
Retain those historical commands. Current management uses a new bounded per-record authorized
publication query with an opaque scoped cursor; invoke-only discovery continues through the
existing filtered capability catalog. The query projects `PublishedMethodStore`; it does not own
another publication list. No source-only pagination suspicion is an executed failure.

Adopt C and its account/authority/association readers together before permitting session-driven
entry. Keep exact old account/declaration/receipt bytes, original digest domains and replay readers;
project their origins without rewriting accepted records. New origins and admission fences require
a supported store generation that old writers refuse. Existing unrelated/uncertain runs are not
retrofitted into a new allowance. Preserve them with their own authority, liabilities and managed
holds; selected evidence may inform a new session under current read rights. Rollback must not
restore a pre-session database while effects accepted under the newer store survive. The canonical
region-source migration is separate: retaining old lowered plans does not by itself retain new
session associations, and old active plans need not be converted merely to observe their evidence.

### U17 corrects the earlier evaluation restrictions

The user's approved direction is broader than the proposed `PairedResultJudgment` executable
profile and than my earlier insistence on distinct proposer/evaluator authority. A knowledge
comparison may reference exact research inputs and outputs without inventing candidate runs or
executable method versions. Self-review is valid attributed evidence with that relationship
visible. Retrospective criteria are also valid, labeled as retrospective; neither deserves the
controlled/independent label merely because a receipt exists.

Retain shared exact evidence identity, judgment receipts, explicit supersession and separate
adoption authority. Add closed subject/basis distinctions for knowledge evidence versus executable
qualification, criteria timing and reviewer relationship. Require candidate/attempt slots and
effect isolation only where the claimed experiment actually executes those slots; do not impose
them on an architectural lesson. Required technical verification and protected publication checks
retain their existing force. Existing protected learning remains a supported profile. Knowledge
approval can make an exact scoped lesson selectable without turning it into a published method.

This completed comparison selects a new application owner for a demonstrated missing lifetime,
while continuing to reject a universal invocation ledger. The two conclusions are consistent:
the former owns new continuous intent; the latter would mostly relocate already owned execution
facts. No source inspection here qualifies the future crash/authority tests as already passing.

## Faris final counterexample review of the combined program

Faris-20261010, `/root/execution_faris`, 2026-10-10. I read the revised
[decision brief](../deliverables/decision-brief.md), [adoption plan](../deliverables/adoption-plan.md)
and proposed [P01](../deliverables/proposed-implementation/01-structured-source-and-shared-authoring.md),
[P04](../deliverables/proposed-implementation/04-ongoing-work-and-response-admission.md),
[P05](../deliverables/proposed-implementation/05-reuse-services-and-independent-owners.md),
[P06](../deliverables/proposed-implementation/06-adaptation-and-managed-work.md) and
[P07](../deliverables/proposed-implementation/07-knowledge-and-evaluated-reuse.md). This review
combines their contracts, rather than treating agreement between documents as behavioral proof.
The final selected name is `control::controller::commitment::WorkCommitmentOwner`, with
`WorkCommitment` and `ExecutionAccountOrigin::WorkCommitment`. The earlier session names above
record the evolution of the same candidate; they do not authorize a parallel `control::session`
implementation. I normalized the selected name in A1 and the execution deliverable.

### F-C1 — local cancellation cannot revoke an already accepted remote operation instantly

Counterexample: a commitment at A invokes a published method at B. B durably accepts, but has not
yet created/entered its private child. A closes the commitment gate, requests exact cancellation
and loses that request or its acknowledgment. B still has an accepted uncancelled operation, a
valid deadline and current caller/service rights. It may legitimately create and enter its child
after A's stop was accepted. A's durable gate cannot be read atomically inside B's transaction.

This follows concrete source. Runtime's
[`published_entry_allowed`](../../../../../crates/runtime/src/engine/effects/published.rs)
walks local publication ancestors, but hands a `Serving` boundary to the serving owner's policy.
The host's [publication gate](../../../../../crates/capability-host/src/serving/published.rs)
checks B's own exact association, serving phase/cancellation, deadlines and caller authority.
The [control continuation](../../../../../crates/control/src/published.rs) creates or cancels the
private child from those accepted facts. The [peer adapter](../../../../../adapters/peer-http/src/remote.rs)
returns an acknowledgment with neither acceptance nor terminal proof when cancel transport fails.
Nothing in these sources supplies distributed instantaneous commitment revocation.

Even locally, the fence linearizes durable entry admission, not the instant a process performs its
first physical effect. The [serving worker](../../../../../crates/capability-host/src/serving/worker.rs)
commits `mark_peer_entered` before invoking the prepared adapter. An effect admitted before gate
closure may physically begin afterward; it is already accepted work with the ordinary cancellation
and uncertainty obligations. The new gate must refuse fresh durable entry acceptance after closure,
not advertise absence of all later physical activity.

**Disposition:** scope the new gate to owner-local starts and entries, including locally reachable
publication ancestors even if the service has its own account. Checking only the immediate account
origin would miss that local published case. At the peer boundary retain ordinary exact outbound
cancellation and unknown usage until B's durable facts resolve them. B must refuse future child
entry once its own cancellation/deadline/current-rights boundary says so, without rewriting an
entry that already happened. A stopped coordinator may therefore still display accepted remote
work, unsettled reservations and managed holds. This is a material product limit to disclose, not
a reason to add an unreviewed distributed transaction or silently retry elsewhere.

I sent this counterexample to Rowan and Elin. Rowan accepted it and revised P04/adoption; I reread
their owner-local/remote-cancel clauses and corrected A1/execution-deliverable blanket wording.
The discriminating implementation oracle has two ordered runs: cancellation retained at B before
child entry must prevent that entry; cancellation lost after B acceptance may allow it, while A
retains uncertainty and sends no replacement. Add a same-daemon nested publication whose immediate
service account differs from the commitment to prove local ancestor gating. Existing cancellation
or reply-loss tests alone do not establish this combination.

### F-C2 — unproved legacy conversion can remove useful future repair

This records the pre-U19 counterexample and response. The later U19 section supersedes the
writer-removal selection: NP4 preserves the versioned graph-native edit path instead.

Counterexample: an accepted legacy graph contains an uncertain active child and still has a
legitimate Task-only future repair under its governing agreement. The new converter cannot prove
that graph/frontier corresponds to editable structured source. Continuing the exact old plan
preserves execution, but it does not create a writer capable of saving the desired successor.
With the selected removal of new legacy graph writing, that repair must refuse even if it would
have been representable by today's old graph mutation API.

Delta's [final conversion contract](structure-comparison.md) states this limitation explicitly;
P01's one-plan legacy reader does not disprove it. P06's promise of complete prospective repair
must be read within that supported conversion boundary. A new method selecting old evidence is
not continuation of the old run, does not inherit control over its uncertain child and cannot evade
an identity-bound protected agreement. No claim that an arbitrary new start is an equivalent repair
is acceptable.

**Disposition:** retain the selected refusal/continue-old-or-explicit-new-work design, but expose
loss of arbitrary editing/prospective repair for unconvertible legacy runs in the brief and adoption
decision. This is a compatibility tradeoff for the user to approve, not a defect to hand P06 as
permission to restore a hidden legacy writer. If preserving every current graph's future editing
is a mandatory requirement, the structured-source-only selection needs a complete conversion proof
or must give way to the corrected-graph alternative. I sent this consequence to Rowan after reading
the new adoption plan. I then reread the brief's added explicit loss of legacy edits/repairs and
requirement to hold the writer switch if unrestricted existing repair is required. Those clauses
address the architectural choice without pretending the user has approved that tradeoff.

The implementation oracle must use an actually accepted legacy graph and active history for which
conversion is deliberately unproved, including an unsettled obligation. Assert preserved old
execution/inspect/replay/cancellation/resolution, precise conversion/repair refusal, and separately
authorized evidence selection. A toy invalid graph rejected by the old validator proves nothing
about this supported-state tradeoff. No fresh conversion experiment was performed here.

### F-C3 — reusable human evidence must not become protected qualification through a common receipt

Counterexample: the author retrospectively reviews an architectural lesson, records a favorable
judgment and receives authorized knowledge-reuse approval. A later commitment selects that lesson
and submits its receipt as the comparison supporting protected method promotion. The selected
knowledge contract permits the first actions; it must reject the final misuse despite favorable
wording, exact hashes and legitimate review authority.

P07's closed `Knowledge`/`ExecutableMethod` subjects and retained protected profile are the correct
boundary. The current [`LearningReceiptReference`](../../../../../crates/control/src/learning.rs)
identifies actor/command, not receipt kind. Current
[manual promotion](../../../../../apps/daemon/src/host/commands/learning.rs) and
[automatic promotion](../../../../../apps/daemon/src/host/commands/learning/promotion.rs)
therefore pattern-match actual retained comparison/policy records and bind the declaration,
candidate, agreement and publication facts. The new common learning owner must retain equally
strict subject/profile matching; a generic `Eligible` outcome or a frontend label is insufficient.

**Disposition:** no additional evidence engine or blanket independence restriction is needed.
Keep knowledge selectable as guidance with self-review/retrospective labels, reasoning, limitations
and counterevidence. Refuse its receipt at protected qualification/promotion inputs before changing
a service generation. A new explicit protected comparison still requires its own original
verifier/target/held-out/returned-product facts and publication authority. Ordinary independent
publication routes retain their actual authority/agreement obligations; knowledge reuse approval
is neither a substitute for those obligations nor a new blanket prohibition on publication.

The required cross-owner oracle feeds the approved knowledge receipt to both manual and policy-
authorized promotion, and to the verifier-evidence consumer, then observes unchanged publication
and protected state. Selecting it in a planning packet must still succeed under read authority.
P07 already requires wrong-subject misuse and mandatory-verifier refusals; I sent the concrete
receipt cases to Rowan. This is an adequate selected contract needing implementation proof, not
a discovered current bypass.

One wording correction was also sent: U17 requires criteria before comparison for a controlled
knowledge judgment. The stronger executable held-out profile fixes inputs/criteria before proposal
generation. Adoption's initial universal “before outcomes” wording would wrongly impose the latter
on comparison of existing knowledge artifacts. I reread Rowan's correction distinguishing those
two timing contracts and explicitly permitting comparison of existing design artifacts.

### F-C4 — upgrading an old account must preserve a suspended child's exact settlement path

Counterexample: an old supported store has a parent use in `Suspended`, an entered child with lost
stop proof, original claim/generation numbers, a publication association, an unknown account
reservation and a saved handoff receipt. Upgrade changes source lowering, account-origin shape
and the location of managed transition policy together. Reopening successfully is insufficient
if any conversion re-identifies the child/account, releases the hold, loses its no-entry parent
evidence, recomputes old receipt digests, or makes the legitimate return operation unreadable.

Current [`managed::transfer`](../../../../../adapters/redb-store/src/managed/uses.rs) requires exact
claims, generation and accepted lineage, child quiescence and current authority; a cancelled parent
cannot resume editing. A wrapper's `NoExternalEntry` evidence is preserved on return rather than
manufacturing a new physical epoch. Current
[lost-stop recovery tests](../../../../../crates/control/tests/control_service/published/managed.rs)
compare retained uses exactly across cancel/reopen and require later explicit resolution. They
exercise the same code generation on both sides, so they are not upgrade evidence.

**Disposition:** the chosen versioned readers and old-origin preservation can handle this case;
it does not require a global invocation ledger or conversion of the active graph into regions.
Migration preserves original declaration identity/digest, run binding, reservations, association,
claim numbers and accepted receipts. New settlement consumes that same reservation, and managed
return follows the same accepted child. Closing admission for orderly upgrade must not itself
rewrite installation lifecycle facts or require a fabricated terminal outcome. A new commitment
may read allowed evidence but cannot absorb those liabilities into a fresh allowance.

Require an actual old-generation fixture upgraded by the new supported reader/writer: unsafe return
and maintenance still refuse; new admissions do not reuse the old allowance; exact original request
replay returns its retained result; valid later stop evidence permits the supported return/settlement
without another external entry. An old writer refuses the new generation and rollback does not
restore a pre-effect backup. I sent this concrete extension to Rowan for the final adoption/gate
oracle. No new versioned upgrade or physical-stop test was executed during this review.

I then reread the revised [P09](../deliverables/proposed-implementation/09-final-full-system-test-and-repair.md)
acceptance clauses. They now require the old-store suspended-child/account/receipt fixture, explicitly
reject same-binary reopen as upgrade proof, exercise lost peer cancellation and locally admitted
effects starting after the fence, and reject knowledge evidence as protected qualification while
preserving guidance use. This resolves the program-assignment gaps identified here; it does not
turn those future tests into evidence already obtained. The three edited documents pass local-link
target and diff-whitespace checks. No Cargo command or production change was made by this reviewer.

The four cases leave two material tradeoffs visible: a stopped local commitment does not instantly
stop a remote accepted operation, and unproved legacy conversion cannot preserve arbitrary future
editing once the old writer is removed. The knowledge and suspended-child cases have concrete owner
contracts and require combined tests; they do not reveal a further foundational owner needing invention.

## Faris U19 reconstruction: preserve graph-native repair under one owner

Faris-20261010, `/root/execution_faris`, 2026-10-10, after the user's exact
[U19](../intent-source-excerpts.md#u19--preserve-legitimate-adaptability-while-choosing-migration).
U19 rejects automatic withdrawal of existing repair and authorizes investigating a bounded graph
maintenance path. It supersedes my earlier proposed removal of every graph writer at the source
switch. That earlier rule confused a preferred new authoring representation with a requirement
for only one serialization writer. Two version-specific encoders under blueprint are not two
semantic authorities when each immutable revision has one source and both use one checked-plan,
runtime, reconciliation and history owner.

**Recommendation, revised after Rowan's challenge:** select the NP4 typed source family with
`Program` as the default for new convenience authoring, and preserve the existing explicit versioned
`GraphNative` construct/genesis, import, copy and edit APIs. Both belong to blueprint and use the
same runtime and reconciliation. Conversion is an available verified operation, not a prerequisite
for a legitimate graph repair. This preserves inactive reusable definitions and ordinary graph
creation/import as well as active recovery. Cancellation or a separate run is not an equivalent
substitute for a repair that the current owners permit.

My first U19 draft proposed a local admission epoch, maintenance eligibility inherited by graph
descendants and special existing-graph import intent. Rowan challenged the need for those new
facts. I withdraw them: they enforced my earlier preference for program-only genesis without a
technical repair requirement or user mandate. Explicit versioned graph APIs already bound their
vocabulary, operation size, identity and authority. Retaining them is simpler and preserves more
useful behavior. This is an actual change in my recommendation, not evidence that the earlier
single-writer premise was adequate.

### Current sources establish an ordinary graph repair route

[`BlueprintRevision::revise`](../../../../../crates/blueprint/src/revision.rs) applies one bounded
`MutationBatch` to an exact immutable base, validates the completed candidate and creates a new
identity. The original revision and its semantics do not change. The
[v3 mutation vocabulary](../../../../../crates/blueprint/src/mutation.rs) includes node/edge
addition/replacement/removal, interface/metadata, exact child-pin upgrades and deliberate merge
parents. Its 512-operation bound and final validation already support coherent compound changes.
This is a graph-native versioned compiler, not a scheduler or mutable definition table.

[`ControlService::submit`](../../../../../crates/control/src/service.rs) verifies actor, exact
proposal/base/digest/observed sequence; resolves the stored base; builds the graph successor;
assesses controller policy, authority and risk; retains the immutable candidate; and requests
ordinary runtime reconciliation. Offline proposals use `ProposeOffline`, with their existing
controller-policy restriction. Save is not live adoption. Approval and application recheck their
own conditions, and resume is a separate ordinary run action. The fact that candidate retention
can precede the outer response remains an exact-command recovery concern, not a reason to turn
maintenance into a direct store mutation from the UI.

[`plan_revision_adoption`](../../../../../crates/runtime/src/engine/reconciliation.rs) takes the
current run, requested revision and actual history. The
[reconciliation matrix](../../../../../crates/runtime/src/reconciliation.rs) distinguishes pending,
active, completed and uncertain occurrences with their scopes and dependencies. Unchanged work
is preserved; pending work can change; changes to completed/uncertain effects and started descendants
receive their existing restricted classifications. `FinishCurrentThenAdopt` can use a changed
active definition on its next invocation while retaining the current occurrence; safe restart
requires the actual existing cancellation/effect conditions. Refactoring to a checked plan must
preserve those decisions, not replace them with “old source cannot be converted.”

The [governed repair test](../../../../../crates/control/tests/control_service/agreements.rs)
already submits a compound graph change: it replaces an editable task, adds an investigation task
and rewires internal edges, preserves the agreement, applies through control, reopens the store
and replays without another adaptation. It also refuses removal of the accepted agreement and
an exhausted revision budget. Another test refuses indirect child adaptation under an inherited
agreement after reopen. These tests have a stronger product oracle than “the old document can be
read”: legitimate prospective work still changes and prohibited bypass still fails. I read their
source; I did not run them or establish the new engine's compatibility.

### Representation and live adoption are different proof obligations

An operation advertised as unchanged **definition conversion** must prove that the resulting
program preserves the original supported control/data meaning, lexical choices, contexts,
interfaces, agreement and independent invocation behavior. A live run additionally needs exact
accepted-frontier correspondence. That is correctly stronger than merely finding similar nodes.

A **prospective revision**, however, intentionally changes future behavior. It need not prove
the whole successor equivalent to its predecessor. It must preserve accepted facts and satisfy
the current reconciliation, authority and agreement rules at the exact run sequence. A graph-native
task repair needs neither region decomposition nor an all-future-runs equivalence proof. A proposed
graph-to-program successor may likewise be considered as an intentional revision when its actual
accepted frontier can be proved compatible; do not label that conversion or silently weaken the
old reconciliation classifications to make it pass. When such cross-format compatibility is not
established, the complete graph-native path remains available.

There is no requirement to replay old history into a new source tree. Each accepted occurrence
retains its governing source revision and source-to-plan identity. Graph-to-graph repair can use
the exact existing configuration/dependency fingerprints. Cross-format adoption requires an explicit
owner-verified correspondence, not a model-supplied “same node” claim. The common runtime may have
old and new immutable plans pinned to different occurrences of one run; that is existing prospective
history semantics, not two competing current definitions.

### One source owner, two bounded source forms

Blueprint owns the closed `WorkflowSource::{Program, GraphNative}` family, with an explicit
source/schema version and corresponding owner-validated edit form. One revision contains exactly
one source form. Graph-v3 retains the original constructor/validator/canonical encoder and lowers
into the same checked `ExecutionPlan`; Program uses the new pure editor/lowerer. A program
revision cannot acquire a separately editable stored graph projection, and a graph revision cannot
acquire a separately editable region tree. Layout remains presentation.

Use the existing blueprint module boundary, with private version-specific implementation, rather
than a generic compatibility framework or new legacy engine. Runtime/reconciliation consume the
common checked plan, stable element identity and exact source fingerprint/correspondence facts.
Authority, risk, context, agreement and publication checks use the same owning services. They may
have explicit version-reader logic where old contracts differ; they must not acquire separate
graph-specific authority or outcome ledgers.

The final-entry, managed claim, publication association and account code remains shared. A legacy
external reducer can retain its v3 source encoding and supported edits while lowering to the same
ordinary Task execution. Therefore P01's former “delete every capability-reducer writer” instruction
must narrow to new structured authoring and duplicate execution paths: retain required v3 source
codec/mutation compatibility. No old producer may bypass shared admission because it is legacy.

### Exact versioned writers, ancestry and reuse

The useful bound is the supported source vocabulary and current operation limits. There is no
admission-date cutoff, global grandfather catalogue, new eligibility fact or special import-intent
gate. Retain the complete graph-v3 mutation vocabulary under its existing permissions; arbitrarily
restricting compatibility to `ReplaceNode` would withdraw the tested investigate-and-rewire repair.
Freeze graph-v3 semantics rather than expanding it with every new Program feature. Program-only
features can be used through ordinary exact compatible calls where that meets the actual need;
do not disguise an unsuitable child-workflow boundary as equivalent graph behavior.

Existing immutable bytes, digests, ancestry and exact compiler semantics remain untouched. An
explicit source family/version selects one constructor, editor and lowerer, not a separate
authority path. New convenience operations may default to Program; an already supported versioned
graph command continues to mean graph even with no base. No new registry is needed to distinguish
these two requests. All descendants and independent copies remain supported under the same
contract as their roots; a conversion on one branch does not freeze its graph siblings.

| Producer/action under NP4 | Exact supported behavior |
| --- | --- |
| New Program convenience creation | Emit Program through a versioned shared blueprint constructor; graph and outline consume its one authoritative source. Do not reinterpret an old graph command as this new operation. |
| Explicit versioned graph construct/genesis, including old convenience commands | Retain exact graph-v3 semantics and current bounds, even without an existing base or historical age claim. Move private daemon compilation into shared blueprint helpers without removing its supported behavior. |
| Graph successor from an exact retained graph base | Apply the complete v3 batch through blueprint; same workflow identity; create an immutable v3 successor. Siblings remain valid; no fictitious latest-head lock. |
| Graph merge | Require the exact selected base and retained parents in the same workflow, with current ancestry/sequence/risk checks. Do not infer the selected base from sorted parent order. |
| Import of a supported graph or its missing ancestors | Preserve exact validated bytes through the current import operation, `ImportBlueprint` authority and ancestry checks. Local upgrade date or an unverifiable creation date adds no new refusal. |
| Explicit independent graph copy | Preserve supported graph semantics under the current copy operation and its copy provenance. Authorize the target workflow separately. Identity-bound agreement-copy refusal remains; no run, account or mutable resource is copied. |
| Graph reused by exact pin, run start or published generation | Keep the exact immutable revision without conversion. New invocations obtain ordinary current authority and appropriate accounts. |
| Explicit verified conversion or intentional program successor | Create a new program revision with exact provenance. Definition-conversion equivalence and live-adoption compatibility remain separate checks. Existing graph siblings are not frozen by the conversion of one branch. |
| Program shown through a graph projection | Layout and derived execution edges are not editable GraphNative source. Cross-family transformation is a deliberate validated owner operation; no hidden reverse conversion. |

This preserves a finite versioned language, not a fixed finite population of documents. Existing
graph creation/import/copy can continue producing definitions. If that ongoing maintenance cost
outweighs Program's benefits, reconsider corrected graph as the common authoring architecture;
do not make the comparison favorable by silently removing useful graph clients or future repair.

### All writers and ordinary consumers must cross that boundary

Current [`authoring::candidate`](../../../../../apps/daemon/src/host/commands/authoring.rs) selects
`genesis` versus `revise` from the draft base. Generic construct/save accepts full mutations, while
the private `ModelWorkflow` helper recognizes one narrower graph shape. Current
[definition import](../../../../../apps/daemon/src/host/commands/definitions.rs) and control proposal
both eventually call [`RevisionStore::put_revision`](../../../../../crates/persistence/src/revision.rs).
The [redb implementation](../../../../../adapters/redb-store/src/revision.rs) atomically validates
same-workflow/lower-sequence parents and inserts canonical bytes plus indexes. These are the actual
producer/consumer points, not just a frontend switch.

Concretely, extend blueprint's source and edit contracts with explicit family/version dispatch,
and keep one immutable store contract for the resulting revisions. The store continues its exact
canonical identity and retained-parent checks; it does not gain a migration-eligibility gate.
Pure constructors remain useful offline and are not authority boundaries. Daemon imports,
authoring, control proposals, copies and maintained tools use the shared helpers and current
authorizing operation; a direct store insertion is not an alternative public control path.
Already-present identical bytes retain their original identity rather than being re-enrolled or
rehashed. Consumers must distinguish the exact candidate base from a sorted ancestry list.

Control owns prepare/edit/save/propose/approve/apply/resume for both source forms. A WorkCommitment
packet identifies the target's exact source version and supplies its applicable mutation contract.
It can request a graph-native repair when conversion is unavailable; the same response-provenance
admission authenticates the result. It cannot rewrite an old target as program source because that
is the format the planner prefers. CLI, independent JSON and Svelte must expose graph maintenance
as actual useful editing/proposal work with the same diagnostics. The existing complete graph
mutation route must not become a hidden emergency command accessible only to bespoke agents.

The daemon's old private semantic compiler should still move to blueprint. An existing finite
model-step convenience delegates to version-specific blueprint helpers for its original v3 command
or base, and new Program operations use Program helpers; this is a client convenience, not another
owner. Keep the exact old recognizer's accepted shapes and refusals. Graph read
models show the real old branch order, input dependencies and accepted occurrences. They must not
display a false editable region decomposition for an unconverted graph. A graph and outline can
project different source forms without treating their layout as a conversion proof.

One concrete audit item must accompany graph merge maintenance: `BlueprintRevision::revise` sorts
merge parents, whereas current `ControlService::apply_proposal` obtains a delta base from
`candidate.parents().first()`. The selected maintenance path must take its exact base from the
accepted proposal/reconciliation facts, not lexical parent order. I have not demonstrated an
executed exploit or declared every current merge defective; the inspected consumers plainly do
not make “first parent” equivalent to “selected base.” This is a bounded source/consumer correction
to resolve in the common owner, not a justification for dropping graph merges without analysis.

### Required five-category behavior matrix

These categories overlap: a protected workflow can also contain uncertain work, and a reusable
definition can have closed historical runs. All applicable rows apply; none grants new authority.

| Existing state | Execution, inspection and reuse | Editing and prospective repair | Recovery, conversion and retirement |
| --- | --- | --- | --- |
| **Closed historical revisions** | Original immutable bytes and completed history remain inspectable/replayable. The exact definition remains reusable under current authority. A historical run is never made active by editing. | The historical revision stays immutable. An authorized graph successor may branch from that exact base; the original events do not change. | No conversion rewrites original identities or results. Historical readers/receipts survive any later writer retirement. A closed run alone does not prove its definition has no reusable consumers. |
| **Reusable inactive definitions** | Exact graph can start a new run, be pinned, copied where currently allowed, or support an authorized published generation. No active run or migration eligibility is required. | Ordinary offline graph edit/save/proposal retains full currently allowed mutation semantics and applicable policy restrictions. Conversion is optional. Successors and explicit copies remain maintained. | Inactivity does not retire support. Any future withdrawal requires replacement/conversion or explicit approval of the lost use, addressing usable definitions and affected consumers; exact readers/replay remain. |
| **Active runs with unentered future work** | Current run/history/accepted choices/account remain the same. Existing future invocation semantics use the governing revision until ordinary adoption. | Graph-native successor and current proposal/reconciliation can add, remove or replace permitted unentered work. No whole-graph conversion proof is required. Apply/resume still uses exact guards, approval where required and current authority. | Interrupted save/propose/apply/resume recovers exact commands/candidate/plan. If conversion is attempted, prove definition or intentional-change claim and the actual frontier separately. Refused conversion does not remove graph repair. |
| **Active runs with started or uncertain effects** | Started occurrences retain their original revision, invocation/claim identity, physical holds and unknown reservations. Nonconflicting future work may remain useful. | Keep current classification of unchanged, changed-active, completed/uncertain and dependency-affected work. A compatible graph repair to unaffected future nodes can proceed where those owners permit it. Never reparent/retry/erase uncertainty by conversion or rewriting the source. | Existing resolution, cancellation, safe restart or remediation remain their explicit operations. An unsafe proposed repair refuses with the actual affected execution/obligation and legitimate available controls. Cancel/new-work is not presented as equivalent repair. Writer removal cannot strand a live supported repair/resolution dependency. |
| **Governed/protected workflows** | Accepted agreement identity, required verifier/target, source pins and public-service boundary survive either source form. | Apply the original agreement's actual permitted task-region changes and revision budget, including compound graph rewiring inside that region. Structural/envelope/agreement weakening and indirect child adoption retain their present refusals. New source does not grant broader adaptation. | Conversion must preserve or explicitly bridge the exact versioned agreement check, including its graph-derived protected digest; logical similarity does not recreate its identity. If that cannot be proved, graph repair remains available within the old agreement. Any changed agreement requires its existing separately authorized boundary. |

The agreement row matters technically. The
[current protected digest](../../../../../crates/blueprint/src/agreement.rs) binds workflow,
blueprint, metadata, interface, protected nodes and edges crossing the editable boundary. Region
syntax is not interchangeable with those bytes. Keep the version-specific checker against the
accepted governing source and exact binding; converting visual structure cannot re-seal an agreement
as if it were unchanged. An inherited child still cannot directly adopt a new revision when the
accepted enclosing agreement forbids it, even though graph editing itself remains supported.

### Supported lifecycle and any later retirement

The selected lifecycle now is simple: both source families remain supported. Graph-v3's feature
vocabulary is frozen, while its existing creation, editing, recovery and reuse operations remain
available. There is no newly authoritative per-lineage support status, retirement command or
admission index in this design. That extra machinery would not make a single semantic owner more
single; it would add storage and policy merely to enforce a source preference.

Removing graph-v3 writers would be a later explicit supported-release decision with a concrete
replacement and adoption procedure. The review must account for closed history, reusable definitions,
their copies/descendants, accepted proposals and exact requests, active/paused/created runs,
uncertain effects, account/resource obligations, child pins and publication generations. It must
also state what happens to existing external clients, old imports and restored backups. A local
zero-active-run count establishes none of those conditions. Since retained graph create/import
remain available, a finite inventory today is not proof of future absence either.

Deletion is justified only when a supported replacement preserves the required edits/recovery/reuse,
or the relevant capability loss is explicitly approved with those consequences and affected
consumers made concrete. A usable inactive definition cannot silently become archival evidence.
An active or uncertain run cannot be stranded by withdrawing a still-promised repair operation.
Retiring a published generation does not discharge its accepted child calls. If a future release
chooses to stop accepting new graph work, that is its own visible compatibility change, not a
consequence inferred from Program being the default.

Original decoders, exact identities and receipt replay, agreement checking and old-plan lowering
have their own obligations even after a future writer removal. Any replacement or retirement must
recover a candidate already committed before its outer receipt and preserve old accepted command
meaning. Readers cannot simply be deleted with an editor. Rollback must not let an old writer
open new facts it cannot interpret; restoring old data cannot erase surviving external effects.
No retirement or capability withdrawal is authorized by U19 or this recommendation.

### Compare costs and select the next design boundary

| Alternative | Benefit and cost under the same old active/reusable workload | My disposition |
| --- | --- | --- |
| Prove complete conversion before removing graph edits | One new editable source form and no maintained v3 mutator afterward. Requires proof for the actual supported graph population plus distinct live-frontier/agreement compatibility; today that proof is absent. A failure need not invalidate all verified conversions. | Pursue conversion where it can be proved. It cannot presently be the condition for all legitimate repair. |
| Program default plus versioned GraphNative support in blueprint/control | Preserves existing construct/import/copy, compound repair and reuse through current owners. Retains v3 validation/mutation/encoder and source-specific UI/proposal readers; both lowerers must satisfy one plan/reconciliation contract. No migration eligibility registry is needed. | Preferred NP4 while the substantive new-source comparison remains favorable. It has real ongoing source-tooling cost, substantially narrower than a second scheduler or ledger. |
| Corrected graph as canonical authoring for everyone | Preserves all current graph editing without conversion pressure and reduces simultaneous source tooling. Keeps authorable edges/ports, structured-constructor lowering and ownership reconstruction; richer choice/result boundaries still require real semantics. | Strong fallback if actual current structures require repeated source escape hatches, per-feature duplicate reconciliation, or disproportionate permanent maintenance. U19 requires keeping this alternative open on that evidence. |

A realistic future policy change tests the distinction. If an external task adds a new final-entry
authority requirement, both source forms should expose it through the same Task/checked-plan
contract and share one admission implementation. The v3 encoder may need a version-specific
diagnostic if its frozen document cannot represent a genuinely new configuration; old existing
behavior and legitimate old edits still remain. If every such change instead needs separate task
semantics, authority logic or reconciliation matrices, the compatibility design has failed its
one-owner criterion and the source choice should reopen. A field decoder or pure lowering branch
alone is not that failure.

Required discriminating implementation evidence follows the five rows: offline reusable graph
repair with no run; the actual compound governed task/edge repair; an unconvertible accepted graph
with safe pending repair; a retained started/uncertain sibling whose claims and account stay exact;
explicit graph genesis and import after local upgrade; legitimate copy plus agreement-copy refusal;
successor-of-successor and same-base sibling edits; merge with lexical first parent different from
selected base; old convenience compilation and exact command recovery across the default change.
Every safe repair must run through public control, not direct test-store insertion. Compare the
new common plan's decisions to the existing v3 behavior with independently stated outcomes.

For command recovery, retain current
[external receipt lookup before new execution](../../../../../apps/daemon/src/host/receipts.rs).
An old accepted graph creation/repair command must replay its old result after the new-source
default changes. A fresh supported graph-genesis command remains allowed under its existing
authority and validation. Persist exact proposal/candidate material and internal command identities
before effects at compound boundaries, and recover the same immutable candidate if revision storage
committed before the outer receipt. Keeping the old command's deterministic compiler matters even
when that outer receipt is absent: silently changing its output to Program could create another
candidate from the same request. New defaults require a distinguishable versioned operation;
moving the old helper out of daemon must preserve its old shapes, bytes and refusal semantics.
Never turn a missing response into an apparently confirmed refusal. Graph compatibility does not
weaken operation-specific authority/disclosure rules.

This is a grounded recommendation to retain graph-native maintenance, not approval to implement
it or proof that the future source plan already satisfies it. The current source establishes the
existing edit/reconciliation/agreement mechanisms and their useful tests; the new typed source and
common-plan contracts above are proposed design. No Cargo, production, schema or live-state change
was made.

### Final U19 source and program reread

Faris-20261010, `/root/execution_faris`, 2026-10-10, independently reread current
[P01](../deliverables/proposed-implementation/01-structured-source-and-shared-authoring.md),
[P04](../deliverables/proposed-implementation/04-ongoing-work-and-response-admission.md),
[P07](../deliverables/proposed-implementation/07-knowledge-and-evaluated-reuse.md),
[adoption](../deliverables/adoption-plan.md) and the completed
[NP4](../deliverables/notation-profile.md), including the native `ProgramAgreement` v2 scope
added after Elin's challenge. I found no additional foundational blocker in their combination.
This is source/design review, not implementation qualification or a claim of total conversion.

- **One plan must not erase source identity.** I reread the domain-separated raw node and incident-edge
  fingerprints in [`blueprint::document`](../../../../../crates/blueprint/src/document.rs), and
  their comparisons in [`runtime::reconciliation`](../../../../../crates/runtime/src/reconciliation.rs).
  A normalized legacy reducer's ordinary Task plan cannot replace its original configuration hash,
  become v1-editable merely by changing kind, or alter its historical dependency ordering. P04 now
  explicitly retains those source-specific facts; P01/adoption preserve their original bytes and
  occurrence identities. Pure versioned fingerprint/correspondence production feeding one decision
  owner is compatible with one runtime; separately implementing its decision rules would not be.
- **Replay requires the old producer as well as the old receipt reader.** I reread
  [`authoring::candidate`](../../../../../apps/daemon/src/host/commands/authoring.rs) and
  [`receipts::execute_inner`](../../../../../apps/daemon/src/host/receipts.rs). Native construction
  is deterministic, while an effect can precede its application receipt. P01 and NP4 now require
  exact old command-version compilation and a distinguishable new default operation, so that gap
  cannot silently switch a retry from graph to Program. The explicit selected-base correction
  also addresses the inspected sorted-merge-parent consumer; a common plan alone would not.
- **Protected source correspondence does not confer authority.** The original
  [`agreement` checker](../../../../../crates/blueprint/src/agreement.rs) seals protected graph
  structure and forbids changing the agreement during an accepted run. NP4 keeps direct v1 checking
  for native graphs and requires a complete current-source-derived `LegacyAgreementView` for a
  converted v1 method, rather than pasted protected facts. New v2 seals an outer source skeleton
  plus stable Task-only Sequence scopes and their input/output contracts. Editable result mappings
  do not make output correctness trusted; verifiers/effects stay protected and runtime preserves
  already accepted results. Existing v1 fragments need not fit v2's narrower scope to remain usable.
  I also reread control's old/new requirement and bounded pinned-body traversal: migration must
  retain those authority checks for either family, not authorize only newly visible Task nodes.
- **Ordinary adoption and protected qualification remain different.** Current
  [`learning` promotion](../../../../../apps/daemon/src/host/commands/learning.rs) requires an exact
  eligible comparison/declaration/candidate/agreement/publication match and publication authority;
  ordinary proposal/application uses its own control gates. P07 now preserves that distinction,
  rejects approved retrospective self-review as protected Promote/AutoPromote evidence, and allows
  the same item as honestly labeled selected guidance. It does not impose learning qualification
  on every ordinary method revision. No permissive source-family or knowledge shortcut remains in
  the reviewed selected contract.

The remaining burden is concrete implementation evidence already assigned to these phases: both
native and converted positive protected repair, original hash/replay fixtures, same-authority denied
cases, completed/uncertain occurrence preservation and owner-level wrong-evidence refusal. The native
v2 obligation choice is now specified rather than left to a later implementation agent. I made no
Cargo, prototype, production, schema or live-state change during this final check.
