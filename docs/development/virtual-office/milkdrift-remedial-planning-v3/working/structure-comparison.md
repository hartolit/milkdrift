# Delta — workflow structure, data and prospective change

This is Delta's independent comparison, recorded before reading the reopened reviewers'
recommendations. It examines source at `ff818e5` and the user-authorized reopening instruction,
with the [selected original excerpts](../intent-source-excerpts.md), especially U05–U08 and
U15–U16. The excerpt selection is incomplete original context; its compiler's interpretation is
not additional user testimony. The existing product-alternatives selection is an input being
challenged, not this comparison's premise. No implementation, schema, fixture, frontend or paid
model call was made. Packets below are design notation, not accepted Milkdrift documents.

The initial recorded recommendation was M, a corrected canonical graph. Independent criticism
changed that selection to R: canonical structured program source, lowered to one checked runtime
execution plan. R has explicit ordered clauses/results, Parallel completion/results and stable
element identities. Runtime occurrences, workspace values and real effect obligations remain distinct.
The current graph and M are worked just as concretely below; M remains the strongest dissent on
migration risk. The [cross-review](#cross-review-changes-the-selection) explains why the initial
retention argument failed. This is a specialist recommendation, not approval of the whole
architecture or the prior P00–P09 program. Conversion of existing graphs into editable region source
has an explicit compatibility limit below; continued execution does not depend on that conversion.

## Coverage audit before selection

“Deep” here means source, consumers and test expectations were examined and an alternative was
worked through for the named responsibility. It does not mean every line of its package was reviewed.

| Package/relationship | Prior v3 depth identifiable in the packet | Delta depth and actual anchors | Remaining limit |
| --- | --- | --- | --- |
| `crates/blueprint`: structure, bindings, validation, immutable revisions | Source mapped; E07 tested direct exclusive reconvergence refusal; standards mapping described kinds; competing canonical representations remained sketches | Deep for control/data topology, branch priority, fork/join/reducer, pins, agreement edit limits: [structured configurations](../../../../../crates/blueprint/src/model/structured.rs), [node kinds](../../../../../crates/blueprint/src/model/node.rs), [bindings](../../../../../crates/blueprint/src/model/contract.rs), `validate_edges_and_bindings`, `validate_control_topology`, `validate_fork_join`, `validate_interface_outputs` in [validation](../../../../../crates/blueprint/src/validation.rs), [agreement](../../../../../crates/blueprint/src/agreement.rs). [Kernel tests](../../../../../crates/blueprint/tests/kernel.rs) and [agreement tests](../../../../../crates/blueprint/tests/agreements.rs) inspected. | No completeness proof for every accepted graph's region decomposition; revision canonicalization and hostile parser bounds inherited from earlier tests, not independently re-evaluated here. |
| `crates/runtime`: activation, selected results, structured lifetimes | Specific branch/join/recovery behavior cited; no equal full alternative trace | Deep for [structured driver](../../../../../crates/runtime/src/engine/structured.rs), [successor/join completion](../../../../../crates/runtime/src/engine/completion.rs), `predecessors_ready` in [support](../../../../../crates/runtime/src/engine/support.rs), [workspace resolution](../../../../../crates/runtime/src/engine/workspace.rs), [reducer](../../../../../crates/runtime/src/engine/structured/reducer.rs), [child handoff](../../../../../crates/runtime/src/engine/structured/subworkflow.rs). [Structured tests](../../../../../crates/runtime/tests/structured_runtime/structured_graph.rs) inspected, including test oracles. | Scheduler fairness/admission limits only read for neighboring assumptions; effect entry, compaction and complete event replay implementation not all re-reviewed. No new execution proof from source reading. |
| Runtime reconciliation × agreements × children | Prospective preservation was a retained invariant; the child workaround's protected-edit consequence not resolved | Deep for [pure planner](../../../../../crates/runtime/src/reconciliation.rs), [engine admission/application](../../../../../crates/runtime/src/engine/reconciliation.rs), [reconciliation integration tests](../../../../../crates/runtime/tests/structured_runtime/reconciliation.rs), [attached child/repeat tests](../../../../../crates/runtime/tests/structured_runtime/structured_graph/continuations.rs), ADRs [0005](../../../../decisions/0005-prospective-revision-reconciliation.md) and [0040](../../../../decisions/0040-protected-adaptive-methods.md). | No claim that a new choice boundary already has a valid migration or reconciliation implementation. |
| `crates/workspace` × runtime data × context | Logical scopes/artifacts named; existing isolation tests cited | Deep comparison of exact values versus activation and exposure; runtime resolution above inspected; [workspace README](../../../../../crates/workspace/README.md) and [context exposure](../../../../../crates/runtime/src/context/source/discovery/exposure.rs) identify consumers. | Workspace constructors/budget arithmetic and context selection itself are mapped/inherited, not a complete independent design review. Mutable host working areas are a different owner's facts. |
| `crates/control`: external reducer, protected proposals | Mostly retained as an owner | Targeted deep comparison of duplicated external-task requirement/risk logic in [policy](../../../../../crates/control/src/policy.rs), [service](../../../../../crates/control/src/service.rs), [publication validation](../../../../../crates/control/src/published/validation.rs), alongside runtime [authority](../../../../../crates/runtime/src/engine/authority.rs) and [dispatch](../../../../../crates/runtime/src/engine/dispatch.rs). | General goal-to-plan, authority/accounting, learning and publication architecture belong to other independent reviews; no general approval follows. |
| `crates/persistence`, redb adapter | Durability retained from existing ADR/tests | Mapped consumers: branch selection, join result, child and reconciliation events; workspace/event atomicity used as a required boundary. ADR [0002](../../../../decisions/0002-append-only-run-events-and-deterministic-projections.md) inspected. | Storage mechanism, recovery schema and transaction implementation not deeply compared by Delta. |
| Prompt sequence, protocol/client, CLI, daemon | Authoring and composition gaps identified | Consumer impact mapped from manifests, package docs and prior current-journey evidence; specific required migrations named below. | These packages are not independently approved by this source trace. Authoring usability and model success remain unperformed experiments. |

Prior evidence has real value: E07 establishes a refusal, not a graph that was accepted and then
stalled. The previous branch test proves that one constant true arm activates and the other does
not; it does not establish that lexicographic arm precedence is desirable. The reducer test
explicitly checks branch-port order against lexical value-reference order, so it tests a deliberate
implementation rule; it does not show that an operator asking for the best or earliest result wants
that order. These are distinctions between executable correctness and product justification.

## What the implementation actually owns, and why it arose

A `SemanticBlueprint` stores nodes and control/data edges; `Node` separately declares ports and
bindings. A `NodeOutput` binding requires a corresponding data edge: the binding identifies a value
or path, while the edge is the scheduling/causal dependency. The information overlaps, but currently
serves two consumers. A constructor can derive the edge from the binding without asking a person to
supply both; removing the edge from serialized meaning is a separate migration decision.

A definition node is not a runtime occurrence. Runtime records one concrete execution with revision,
scope, selected branch, inputs and attempts. A logical workspace reference identifies an exact value
version and scope; it does not say whether a node is activated. A mutable host repository is neither
of these: sharing a graph scope does not grant concurrent editing of the same directory. This
separation survives every credible alternative considered here because the same definition can be
called/repeated and the same artifact can be used by several independently authorized operations.

Current activation is deliberately not general token flow. `predecessors_ready` requires all control
predecessors succeeded in the relevant scope/epoch, except the special join path, and all required
data edges have a successful output. The validator correspondingly admits multiple incoming control
edges only at a fork-owned Join. Choice selects a port; it does not create tokens for unselected
arms. Fork allocates independent child scopes. Completion builds `BranchResultReference` values;
`JoinSatisfied` records which completed branches were present when the policy was satisfied.
`All` waits for termination, including failure. `Any`, `FirstSuccess` and `Quorum` request cancellation
of active losers, but that request is not physical stop or permission to release their holds.

The original pure kernel commit `c867155` introduced `BranchConfig`'s `BTreeMap`, the single-entry
acyclic graph, and the “only a join” restriction together, before the durable runtime. Its original
`ARCHITECTURE.md` §8 already prescribed branches, separate fork/join/reducer, pinned repeat and
explicit terminals. ADR 0001 gives the real reasons for external capabilities, immutable work and
bounded rather than arbitrary cycles. I found no original design argument there explaining why
exclusive alternatives must not reconverge or why port spelling should encode precedence. History
locates the choices; it does not establish that they were operational necessities. The later runtime
commit `8bf6b71` contains the ordered branch iteration; current source retains it. The original
conversation that generated those restrictions was not available to this reviewer.

Later features increase the cost of those early choices. Public authoring now exposes revisions and
selected outputs, and protected agreements make a child call a meaningful frozen boundary.
`check_inherited_agreement_adaptation` rejects direct child adoption when the accepted agreement's
origin run is its parent. `GoverningAgreement::validate_method` permits only ordinary Task nodes in
the editable prefix: structured nodes, terminals, calls and crossing edges are protected. Packaging
an exclusive choice into a child therefore changes not just layout but how future repair is allowed.

## Equal authoring input and independently chosen outcomes

The following packet is supplied unchanged to each author/representation. No option receives a
prebuilt graph. It is deliberately concrete enough to catch the current prioritization and data
boundary assumptions. Operation names below are fictional capability contracts for this exercise.
They neither install capabilities nor authorize calls.

```text
Method: investigate-and-repair
Inputs:
  brief: Artifact<text/plain> (required, immutable upload)
  baseline: Artifact<application/json, assessment.v1>
    { bad_premise: true, risk: 0.9, accepted_candidate: false }
  repository: managed working-area reference already authorized for sequential editing
Outputs:
  report: Artifact<report.v1> (required)
  accepted_candidate: Artifact<candidate.v1> (optional)
Limits: at most 3 review/repair cycles, 2 concurrent reviewers, supplied cumulative budget.
Permissions: read the brief and selected repository evidence; two read-only reviewer capabilities;
  repair may edit only the supplied working area; publication is not authorized.
Reusable child: independent-review@exact-revision H, inputs brief/evidence, output review.v1.
Reviewer inputs are isolated; neither receives the sibling's private notes.

Actions:
  Assess baseline.
  Choose first matching case, in this business order:
    invalid-premise if bad_premise: re-investigate before repair;
    cautious-repair if risk >= 0.8: repair with additional verification;
    ordinary-repair otherwise.
  Each case returns candidate.v1 plus explanation.v1.
  Continue exactly once with two independent calls of independent-review@H in parallel.
  Wait for both reviewers; collect their reports; a separate judge evaluates disagreement.
  If criticism changes future work, preserve all accepted assessments/reports, revise unfinished
    work prospectively, and run fresh verification. Never reclassify old failure as success.
  If a human decision is needed, await one authenticated targeted decision for this occurrence.
  Before another bounded cycle, wait 30 seconds durably. On limit exhaustion require a decision.
  Finish with a report; publish no application or method implicitly.
```

The baseline makes both named predicates true. Expected outcome is `invalid-premise`, irrespective
of display labels or generated node/port IDs. Precedence is a business decision and belongs in
semantic order. An explicit user change to that order creates a revision. Model inference about
which repair is better is the assessor/judge's job; the runtime interprets a validated condition.

The independent result oracle for the packet and held-out variations is:

| Case | Outcome before looking at any representation |
| --- | --- |
| D1 baseline above; rename `invalid-premise`'s ID to `z` and cautious arm to `a` | Re-investigation still wins; unselected repair never enters. One candidate/explanation pair reaches one continuation. |
| D2 first reviewer reports a failed invocation, second returns a valid criticism | Both facts retained; “both finished” does not imply two usable reviews. Judge receives declared available results plus outcome evidence and must not invent the missing review. |
| D3 replace “both” with first successful reviewer; loser is a still-running writer in a legitimate extended method | Continue on the recorded success under explicit policy; retain the loser's cancellation/uncertainty, editing hold and budget. A later write cannot be treated as absent. |
| D4 nested child changed to H2 after one H invocation starts | Started H call retains H; future selected calls may use H2 only after compatible authorized adoption. Caller sees declared outputs, not all child context. |
| D5 assessment accepted, selected arm's investigation completed, repair pending; criticism inserts a different read-only probe before repair | Preserve completed evidence, reconsider only affected future work, and return one selected result. If protected agreement forbids the structural change, refuse with the exact boundary and require a newly accepted agreement/run rather than quietly weakening it. |
| D6 crash after branch selection, after join decision, after child intent, during timer/decision wait | Reopen the recorded selection/result/child/deadline. No new decision from mutable inputs and no duplicate continuation/call. A fresh agent receives exact selected evidence and pending state. |
| D7 identical targeted decision delivered twice; unrelated broadcast uses same type | Consume the targeted decision once for its declared occurrence; do not satisfy another wait from an unrelated signal. A human approval remains a separate authorized operation. |
| D8 each of three cycles fails acceptance; no remaining budget | Stop/hold according to declared limit policy. A retry, child or revision must not reset the shared budget. |

These expectations derive from the work requested, not a snapshot of today's tests. D3's writer is
an explicit variation, not something the baseline's read-only reviewer grant permits. D5 distinguishes
ordinary exploratory work from a protected deployment method; they use one runtime with different
accepted obligations, not two global static/dynamic modes.

## Candidate C — complete current representation

The strongest current encoding uses an exact child workflow for the choice. It is more capable than
showing the rejected direct merge, and its additional boundary is part of the comparison.

```text
R-choice@C1 interface: baseline, brief -> candidate, explanation
  decide: Branch arms { "01-invalid": bad_premise, "02-cautious": risk>=0.8 }
          fallback "03-ordinary"
  "01-invalid" -> investigate -> repair-after-investigation -> success-invalid
  "02-cautious" -> cautious-repair -> success-cautious
  "03-ordinary" -> ordinary-repair -> success-ordinary
  each success terminal binds required candidate/explanation from its own route

R-cycle@L1 interface: brief, baseline, repository -> report, candidate?, repeat?
  choose: Subworkflow(R-choice@C1, copied exact interface)
  fork reviewers: { alpha, beta }
  alpha -> Subworkflow(independent-review@H) -> join
  beta  -> Subworkflow(independent-review@H) -> join
  join: Join(All, owning fork)
  collect: Reducer(Collect, minimum_items chosen explicitly, branch output edges)
  judge: Task(full capability requirement + selected brief, candidate, review evidence)
  decision: Branch(accept / revise / need-human), each ending in its own terminal
  need-human route: SignalWait -> durable Wait(30000) -> terminal with repeat=true
  revise route: ordinary proposed-revision control actions -> Wait(30000) -> terminal

R-main@M1: Repeat(R-cycle@L1, post-condition repeat, maximum=3, AwaitApproval) -> report terminal
```

Each arrow expands to declared control ports and one edge; each named input requires a schema and
binding, and each upstream output binding also supplies its matching data edge. Each successful
terminal must provide every required child output. Imports of declared child outputs create parent
workspace values with an explicit child link. These aren't requirements the human should retype;
they are the current construction packet a builder would have to produce.

D1 works only if priority is encoded into stable lexical port spelling, as shown. Display labels can
change separately, but identifier regeneration/reordering is consequential. Direct shared continuation
and multiple source edges to one ordinary required input are refused, so copying one downstream judge
onto each route is another current workaround; it duplicates semantic authoring rather than solving
selected output ownership. The child method is a better current encoding when its independent reuse
and pin are wanted.

D2 needs separate outcome evidence: `Collect` emits ordered value references, not a typed complete
branch-outcome record. A failed producer cannot supply a required successful data output. Defining two
required incoming values is not the same as asking to wait for both branch terminations. The packet
must declare optional value availability and separately expose join outcome context. Current tests
exercise All with success/failure and reducers with two successful values in separate cases; they do
not prove the complete D2 composition. This trace therefore predicts the contract burden, not a new
observed bug or passing run.

D4 succeeds for future calls and existing creation pins. D5 under an ordinary ungoverned child needs
an explicit child revision/adoption, not merely a parent pin edit. Under an inherited agreement that
child adoption is refused. Even on the parent, the editable agreement region is Task-only and crossing
edges are immutable. That refusal protects real obligations but demonstrates why “wrap the choice in
a child” is not an equivalent same-scope repair operation.

D6 uses existing durable child intent/creation/import steps. More records here are justified by actual
separate run acceptance, not automatically duplication. D7 needs a precise authorable wait contract:
`NodeKind::SignalWait` currently names only an operation, and the structured driver registers it with
`correlation: None`; lower runtime signal contracts support richer targeting/correlation. The ordinary
node shape cannot be described as an arbitrary correlated-message catch. D8 uses pinned repeat bodies,
iteration scopes and child account inheritance. Replacing a repeat body while its current occurrence
is active cannot be advertised as ordinary pending-task removal; the existing runtime tests refuse
that reinterpretation.

## Candidate M — corrected graph, complete owned constructs

M keeps a single immutable graph but removes the need to invent a child simply to select an output.
Its public construction packet is a full compound operation, not a frontend collection of partially
valid port edits:

```text
same R-cycle inputs, outputs and body as C, except:
  choice decide:
    arms = [
      {id: invalid, when: bad_premise, entry: investigate, normal_exit: repaired,
       yields: {candidate: repaired.candidate, explanation: investigate.explanation}},
      {id: cautious, when: risk>=0.8, entry: cautious-repair, normal_exit: cautious-repair,
       yields: {candidate: cautious-repair.candidate, explanation: cautious-repair.explanation}}
    ]
    otherwise = {id: ordinary, entry: ordinary-repair, normal_exit: ordinary-repair,
       yields: {candidate: ordinary-repair.candidate, explanation: ordinary-repair.explanation}}
    continuation = reviewers
    outputs = {candidate: candidate.v1 required, explanation: explanation.v1 required}
  reviewers consumes decide-result.candidate, never a union of arm-local required edges
  judge consumes the join's recorded outcome/result set, not every sibling's mutable state
  decision-wait declares type, target occurrence, matching correlation and delivery mode
```

M's concrete durable representation is ordered `BranchConfig` plus one `ExclusiveMergeConfig`
referencing that Branch and owning the result contract and each arm's normal exit/result mapping.
Entry still uses Branch's control ports; the merge is an explicit runtime occurrence with its own
completion fact. These are different facts: choosing a route and successfully returning its results.
A private validated choice index derives region membership from the complete candidate and supplies
it to validation, activation, context exposure, reconciliation and inspection. No caller stores a
second mutable list of region members. Public compound edits create/rewrite the pair and wiring
atomically; a raw mutation may still be supplied but must pass exactly the same complete validation.

`BranchConfig` uses an ordered vector of uniquely identified `BranchArm { port, condition }`,
plus a distinct fallback. This deliberately adopts ordered evaluation, not UML's unconstrained
choice among several enabled guards. Order is included in the semantic digest. Renaming an ID cannot
move its position. For old immutable documents, their versioned reader derives the vector from the
historical map's lexical ordering while keeping original bytes/digest and recorded selections.
Saving an editable successor writes explicit current order. Never recalculate historical route
selection, and never silently sort a new vector during canonical encoding.

A merge validates one owning Branch, one declared normal exit per arm, compatible output schemas,
no bypass into the shared continuation, no foreign/concurrent incoming activation, and no forbidden
crossing into or out of an arm. Nested choices/forks/calls are legal inside an arm. A selected failure
or cancellation doesn't masquerade as a normal exit. Unselected arms neither activate nor supply
required data. At selected normal exit, runtime atomically records owning choice occurrence,
selected arm, source execution/value references, imported result values and completed merge. The
successor becomes eligible from that durable fact exactly once. A selected required value missing at
normal exit fails explicitly; an optional absent output remains absent with evidence.

Choice containment is definition structure, not a new workspace, child run, actor, grant or account.
The same root scope continues. Parallel work inside the arm still owns isolated scopes and unsettled
losers. The choice normal-exit predicate waits for its declared computation/result obligation, while
unresolved physical obligations remain with their existing owners and cannot be erased by merge
completion. The run cannot report all owned work settled because a result became available.

D5 preserves the selected arm even when its unfinished internal task is replaced: the new candidate
must preserve the active choice identity, selected arm, result interface and compatible ownership.
Planning compares all changed node/dependency and choice-boundary fingerprints. Editing a condition
already used has no effect on its occurrence; removing/reparenting the active arm or changing an
already adopted result contract refuses. Pending content within the selected arm remains eligible for
ordinary prospective repair. No new copy of the completed investigation appears. After merge completion
its result is immutable; future improvement is new work using that accepted evidence.

M meets D1–D8 as a modeled contract; existing source does not implement it. Compared with C it removes
an unnecessary child definition, child run and import boundary for exclusive continuation while
retaining child calls where they are real reusable operations. D7 additionally requires a complete
wait configuration; simply drawing a message symbol is not enough.

## Candidate R — canonical structured regions using UML's substantive ideas

R makes structure the stored definition, rather than a compound authoring operation expanded into
canonical edges. A strong R is not a generic “tree is simpler” claim. It has stable element IDs,
explicit value references, result signatures, and separate definition containment/workspace ownership.
Its task leaves use the same capability/context contract as M.

```text
MethodRevision {
  body: Sequence(id=cycle, [
    Conditional(id=decide,
      clauses=[
        Clause(id=invalid, test=bad_premise,
          body=Sequence([investigate, repair-after-investigation]),
          results={candidate: repaired.candidate, explanation: investigate.explanation}),
        Clause(id=cautious, test=risk>=0.8, predecessor=invalid,
          body=cautious-repair,
          results={candidate: cautious-repair.candidate, explanation: cautious-repair.explanation}),
        ElseClause(id=ordinary, body=ordinary-repair, results=...)
      ], result_signature={candidate, explanation}),
    Parallel(id=reviewers, completion=AllSettled,
      branches=[Call(id=alpha, revision=H, inputs=...), Call(id=beta, revision=H, inputs=...)],
      exports={reviews: each declared review, outcomes: each branch outcome}),
    Task(id=judge, inputs={candidate: decide.candidate, reviews: reviewers.results}),
    Conditional(id=decision, clauses=[accept, revise, need-human], ...),
    AwaitTime(id=pace, duration=30000)
  ]),
  outer: Repeat(id=review-cycles, body=cycle, post_condition=repeat, max=3,
                exhaustion=AwaitApproval),
  result_signature={report, accepted_candidate?}
}
```

This uses UML 2.5.1 §16.11.3.3 ConditionalNode's ordered/predecessor clauses and matching body
outputs/result pins, not merely a decision diamond. It also takes Sequence/Loop structural ideas.
The standard's nondeterministic alternative selection is restricted by total order and pure bounded
conditions; missing required outputs fail instead of producing an implicit nullable result. Activity
edges may cross structured nodes in UML, so R's explicit export rule below is a Milkdrift restriction,
not something “the standard requires.” UML termination of internal execution is interpreted as a
request plus retained real-world obligations for external capabilities, never instantaneous rollback.
These are deliberate departures supporting deterministic durable work and truthful effects.
[Normative source: UML 2.5.1, §§16.11.3.1–4](https://www.omg.org/spec/UML/2.5.1/PDF).

R's `RegionKind` is a closed enum of Sequence, Conditional, Parallel, Repeat and leaf Call/Task/Await/
Return. There are no persisted control ports, control edges, independent Fork nodes, independent Join
nodes or separate exclusive merge nodes. Data references remain canonical `ValueSource` values;
the graph is a computed view of those dependencies plus region sequencing. Parallel creates workspace
scopes; Conditional and Sequence do not. A lexical region is not automatically an authority boundary.
A region exports selected results through its declared signature. Reusing an ancestor value in two
later tasks is allowed without copying bytes; sibling private values require the Parallel export.

The runtime owns `RegionExecution {id, region_id, governing_revision, phase, selected_clause,
children, result_refs}` and ordinary task attempts. Activation dispatches only the selected clause,
creates all bounded parallel branch children, advances Sequence after its preceding element's
completion, and closes Conditional with the selected clause's exact result references. A Parallel
records the same completion-versus-success distinction and cancellation obligations as M. A Repeat
owns each bounded iteration plus loop-carried values; Call remains an exact independent child pin.
Thus R can remove graph reachability reconstruction for well-nested control structure. It cannot
remove branch outcome/selected value provenance, child acceptance, effect uncertainty or workspace
scope transitions, because those facts still differ in the demanding cases.

Public operations become `InsertElement`, `ReplaceFutureElement`, `SetClauseOrder`,
`SetRegionResult`, `MoveFutureElement` and `PinCall`, applied as one validated immutable candidate.
No UI operation writes runtime state. Node IDs must be stable identities independent of tree paths:
using `cycle/clauses[0]/body[1]` as durable identity would turn insertion into historical renaming.
Validation checks unique identities, bounded nesting, input availability, result totality, reference
visibility and supported agreement changes. Conditions remain a safe AST rather than embedded UML
opaque executable code. These are executable semantics, not an interchange or model-proficiency claim.

D5 illustrates the major extra decision R must make. Replacing the entire selected Conditional or
Sequence because any child changed is too coarse; it would reject or duplicate completed investigation.
R therefore needs prospective adoption inside an active region: freeze the selected clause and
accepted prefix, keep running children on their governing revisions, diff the pending descendants by
stable ID, preserve the result contract and append a region continuation amendment. This is a new
region-level form of the current node/dependency reconciliation, not elimination of reconciliation.
Moving a pending task between parallel branches changes workspace/authority visibility and cannot be
accepted as an innocent tree rearrangement. D6 replays recorded region transitions and never reruns
tests merely to recover the selected clause.

A pure R cannot claim every currently accepted graph fits it without proving that. Current validation
checks reachability, fork association and acyclicity; this review did not prove global well-nested
single-exit structure or all allowed cross-region value references. A migration must either express
those graphs with the same semantics, or refuse conversion while preserving inspection, exact replay
and running work. A permanent “legacy graph” region that embeds the old scheduler would be a second
language/engine, not successful consolidation. A viable migration instead compiles both supported
immutable document versions into one checked runtime plan and one engine; new authoring emits only
R. Old revision identities/receipts remain untouched. Active occurrences keep old plan semantics;
prospective conversion requires a proved correspondence for their continuing frontier. Unsupported
conversion means continuing the accepted old revision, or explicitly starting new work with selected
retained evidence; it cannot mean discarding an unresolved effect. This cost is material even when
current hand-authored examples happen to decompose neatly.

R handles D1–D8 at least as expressively as M under these explicit rules. It wins on native complete
construction and definition ownership. Its costs are not simply more changed files: it moves control
causality into containment, needs new cross-region exposure rules, and requires region-aware adoption
while still preserving stable task identities and exact result provenance. Its strongest case is that
these rules become one shared source from which execution and diagrams are derived, rather than
repeated recovery of structured meaning from graph edges.

## Candidate S — actual BPMN execution basis, rather than standard-looking glyphs

A serious BPMN candidate stores a restricted executable process: assess task; ordered exclusive split;
three paths; exclusive merge; parallel split; two Call Activities; parallel merge; judge task; decision;
correlated receive; intermediate timer; bounded loop subprocess. Data associations bind explicit
input/output mappings. Exact immutable call revision, scoped grant, retained artifact references and
prospective adoption are extension attributes/contracts owned by Milkdrift.

BPMN 2.0.2 §13.4.2 evaluates exclusive outgoing conditions in order and passes each incoming token
through the merge; this provides a substantive priority and reconvergence basis. It does not promise
that unrelated concurrent incoming tokens produce only one continuation. The safe subset must prove
exclusive ownership or deliberately support multiple activations. Parallel gateway token
synchronization is also not “all attempts terminated”: failed activities must route an explicit error
outcome to a branch result before joining. Message reception fits the targeted decision; a broadcast
signal is a different contract. [Normative source: BPMN 2.0.2, §§10.6.2, 13.4.1–2](https://www.omg.org/spec/BPMN/2.0.2/PDF).

Under S, D2 requires an explicit error boundary/handling task that creates an outcome token. D3 needs
a completion-condition subprocess or supported complex policy plus cancellation extension; an ordinary
parallel gateway is insufficient. D5 requires a specified process-instance migration operation outside
the inspected core semantics, with the same accepted-history and effect constraints. D6 persists token
identity/locations, activity-instance identity, correlations and call links. It cannot infer execution
from the BPMN XML shape alone. The engine still needs an exact value/provenance and authority owner.

This is a coherent standard basis if declared as such: multiple tokens have one explicit supported
meaning, unsupported events/expressions/mappings refuse, and extensions round-trip or import refuses.
It may be worth its cost if external process-tool interchange or multiple active tokens at the same
logical step becomes a real requirement. Neither is established by the supplied original intent.
Calling S “too big” is insufficient criticism; here its extra token model fails to remove the exact
migration/account/uncertainty rules the demanding case requires, and it adds modeling choices for
failure-as-data that M/R can state directly. No comparative human/model authoring experiment was run.

## A realistic future change across owners and consumers

After using the method, the operator asks: “Keep two required reviews, add an optional specialist
review only for migration changes, and let the judge see a typed missing-review outcome if the
specialist times out; an active repair still holds its working area. Preserve the old verdict.”
This tests more than insertion of a box.

| Concern | C current graph | M corrected graph | R regions | S BPMN subset |
| --- | --- | --- | --- | --- |
| Definition | Add specialist route in the fork, a nested child/branch workaround for optional work, explicit output contracts and timeout handling. Existing SignalWait node does not itself declare signal-or-timer. | Add conditional arm in specialist branch, selected result `ReviewOrSkipped`, explicit wait contract and timeout result; shared join result contract carries outcome. | Add Conditional child in Parallel; every clause returns `ReviewOrSkipped`; Await has explicit deadline/result; no separate control wiring. | Conditional route plus timer/error boundary produce a result token; update parallel gateway inputs and data associations. |
| Authoring | Must coordinate fork ports, join, route terminals, bindings/edges, additional pins/interfaces and required/optional inputs. A complete helper could hide these, but the child edit boundary remains. | One blueprint-owned compound edit derives connections and validates result mapping; the edit's meaning is independent of UI. | One structural edit updates tree/result signature and derived graph. | One supported subprocess edit updates flow and mappings; importer validates subset/extensions. |
| Activation/results | Existing All waits for branch termination, but available report refs and missing outcomes need explicit handling. | Existing branch ownership plus selected result boundary; join outcome/result input exposes exactly settled selection, not all sibling outputs. | Parallel/Conditional executions own result set; additional timeout must retain external obligations. | Token from timeout handler completes logical branch; late external result needs separate retained effect handling. |
| Repair | Parent and child pins/agreements may force a new governed run. | Pending specialist work may change within accepted boundary; new protected structure still needs a new agreement/run. | Amend only pending descendant frontier, preserving active region/accepted prefix; changing protected result contract still needs new agreement/run. | Migrate token/instance frontier; changing protected contract still needs a new agreement/run. |
| Shared consumers | Blueprint/readers, control risk, runtime scheduling/data/context, persistence events, authoring, result views and examples all need coherent changes. | Same owners; validated flow index prevents separate graph traversals deciding different region membership; selected-result contract is shared by runtime/context/inspection. | Replace graph consumers with region/derived-plan consumers; stable task IDs and region-result provenance required by context, agreements and history. | Add process parser/metamodel/expressions/token plan; teach all consumers extension semantics; no authority or hold owner can be omitted. |

M and R make the author's edit similarly compact only if M actually supplies the compound operation.
A promise that a clever UI will manipulate edges is not equivalent. R removes more construction
invariants by type, but it does not make a timeout writer safe to forget. C's child workaround
multiplies method/repair boundaries as this change repeats. That consequence positively justifies
replacing the workaround rather than declaring existing tests sufficient.

## External composition: remove a real duplicate path

Current `ReducerStrategy::Capability(OperationId)` is a second representation of external work.
Runtime dispatch and authority independently synthesize `CapabilityRequirement::new(operation)`;
control risk gives it separate conservative classification; publication traverses it separately.
It cannot declare TaskConfig's exact profile/placement/context policy. This is source-confirmed, not
an inference from the word “reducer.”

Three complete options were compared:

| Option | Construction and behavior | Consequence |
| --- | --- | --- |
| Keep operation-only reducer | Join → external reducer, item input assembled by `ordered_reducer_references`; context follows existing non-Task behavior | Smallest change, but a judge has poorer requirement/context expression and a separate policy path. No compelling outcome requires this restriction. |
| Embed `TaskConfig` in capability reducer | Preserve reducer node and multi-input assembly; replace operation-only field with full task config; one shared task-requirement accessor used in authority/risk/dispatch | Fixes expressive loss, but still duplicates two external-work kinds and requires every future task feature to address both. This is a viable smaller correction, not a strawman. |
| Ordinary Task plus declared collection input | Only Task represents external execution. A typed collection binding assembles existing exact selected references; deterministic Collect/First remain optional explicit data transforms | Selected recommendation: one task admission/context/account path; collection is input meaning, not a second executor kind. |

The proposed type is a bounded `CollectionBinding` on an ordinary task data input. It names the
owning join occurrence through its definition ID, an explicit list of exported source ports, item
schema, minimum item count, and semantic result order. Runtime resolves it only from that exact
`JoinSatisfied` receipt and eligible declared exports; it cannot scan unrelated sibling successes
or later outputs. The value supplied to the task is a versioned `JoinedResults` document containing
join identity/rule/accepted sequence and ordered `BranchResultReference` entries with outcomes and
exact value references. A projection onto values supports an existing compositor that expects the
old ordered array. Required value absence is evaluated against the collection's minimum, not a
requirement that every losing branch have a successful output. A second independent mandatory input
can still require an exact particular review. Metadata does not authorize artifact bytes; context
selection/materialization independently checks read authority and selected sources.

This preserves information a naive “Collect then Task” conversion can lose: Collect currently outputs
only an array of value references, while the join receipt knows outcomes. A new authoring operation
may emit a visible deterministic collection node plus Task when the collection is reused, or a
collection binding directly when it has one consumer. Both call one runtime resolver, and both use
the same `JoinedResults` source. There is no second durable selected-result owner: the document is a
projection of the exact join receipt and references; workspace publication records which projection
was supplied. A model judge receives selected report bytes through its ordinary context policy, not
merely JSON identities it cannot read.

Migration must not pretend all legacy reducers were after a join. Existing validation admits reducers
with explicit incoming data edges without that ownership. The version-specific blueprint lowering
therefore maps old capability reducers to an ordinary external task plan with their *historical*
ordered-source collection contract and historical default requirement/context. The shared collection
resolver supports explicit source collections and join-receipt collections as different declared
sources; it does not invent missing outcome facts. New normalized successors use TaskConfig and must
explicitly choose their source. The old reducer occurrence/attempt identity and replay are unchanged;
no synthetic extra durable task is inserted into already accepted history. Old readers and validation
remain compatibility code; the external dispatch/authority path is unified, not two schedulers.

The full consumer set found by `rg 'ReducerStrategy::Capability'` includes runtime `engine/support`,
`dispatch`, `authority`, `state`, `structured/reducer`; control `policy`, `service`,
`published/validation`; blueprint configuration/validation; proposal-risk fixtures and structured
runtime fixtures. New writers, protocol authoring, CLI/agent construction, examples and graph labels
must emit ordinary tasks. Protected agreement v1 cannot silently treat an old protected reducer as
editable merely because its new representation is Task. Normalization is a new revision; agreement
identity/fingerprint preservation or a newly accepted agreement is explicit. No automatic authority
expansion follows from a richer requirement declaration.

## Cross-review changes the selection

After recording the comparison above, Delta read Elin's
[agent-directed-work](agent-directed-work.md) and Faris's
[execution reconstruction](execution-reconstruction.md). Elin challenged the graph retention
argument with a concrete counterexample: the cited advantage of passing typed ancestor values across
sequential control portions already exists in R's `ValueSource` references. R can also handle D1–D8
and the optional-specialist change without extra runs or copied artifact bytes. Delta accepts this
criticism. Existing graph flexibility was an asserted discriminator, not a demonstrated one.

M's full ChoiceDraft, inferred no-bypass boundary, result map and shared membership index amount to
a region definition split across graph fields. Saying the construction operation is equally concise
does not explain why its derived ports and edges should be the permanently editable source. This
changes the recommendation to R. The region source remains the editable product for human and model;
a graph is a projection with commands mapped back to that source. Source containment owns control
structure. Typed `ValueSource` owns data meaning; the execution plan derives both scheduling and
causal dependency indexes from it. A user does not maintain both an output binding and matching edge.

The real remaining case for M is lower migration risk and known behavior of unusual accepted graphs.
No demonstrated task in this comparison makes M more expressive than the strongest R. That limit
must remain visible; the conclusion is not that a tree is universally superior or that graph execution
should be deleted. The selected runtime plan can and must still represent legacy graph behavior.

Elin's assistance design compiles a finite prepare/model/admit round to ordinary work. R fits that
placement: assistance constructs the canonical program through the same blueprint-owned operations
as the editor, and the model's untrusted result uses those operations or a complete program candidate.
It cannot contain a second ChoiceDraft/Parallel compiler inside the response parser. Proposal
admission binds actor, evidence, exact target and authority through control. The public assistance
result remains a normal validated proposal, not a privileged write to program structure.

The later [U18 instruction](../intent-source-excerpts.md#u18--bounded-automatic-reconsideration-within-ongoing-work)
requires bounded automatic reconsideration as a product capability; it explicitly does not approve
a session owner or architecture. Elin consequently developed C's continuous work-session alternative.
Delta compared its strongest ordinary-root competitor rather than treating current primitive gaps
as a proof that the application owner must exist.

The full root competitor is a fixed supervisory program with independent work and supervisor
branches. It needs an asynchronous `StartOwnedChild` which freezes an admitted target and inherits
the account, returns an exact child handle, and keeps child lifetime attached to the enclosing root
after the start action itself completes. `ObserveOwnedChild`/`AwaitOwnedChild` must distinguish a
nonterminal checkpoint, pause, result availability and terminal drain. A bounded supervisor loop
reads selected criticism and the exact child frontier, admits a revision, then uses ordinary target
commands. It must carry immutable evidence/state to the next round and retain all outstanding owned
children through cancellation/restart. The root definition can remain fixed while revising work
children, avoiding the self-staling proposal against the supervisor's own advancing sequence.

This is a credible single-runtime design. A dynamic target on today's synchronous Call alone is
insufficient: the child handle/result cannot reach the next action while a paused child is still
active. Publishing it early changes data readiness; returning the start action early changes
structured ownership and active-state retirement. Keeping the work in a parallel sibling allows
the supervisor to run, but does not itself define the handle, exact observation delivery or final
join/account/cancellation contract. The full root therefore needs genuine new public runtime
semantics rather than a compiler wrapper around the existing pinned Repeat and terminal-returning
Subworkflow.

For U18's bounded ongoing commitment Delta favors Elin's concrete `control::session` placement:
append-only next-action intent, immutable policy and exact runtime associations belong to the
commitment; scheduling, attempt outcome, effects and resource holds remain with their current
owners. The session's account origin/binding must be explicit and atomic, not an arbitrary CreateRun
with a copied account ID. The application continuation can react to an independently paused target
without pretending the target returned. This does add a coordination state machine, and its phases
are not a second source of run success. The selected region source keeps fixed pinned Call; no
unselected dynamic-call/async-child primitive is smuggled into NP3.

The strongest dissent is programmability: the root competitor makes the supervisor itself an
ordinary reusable/evaluable method. A closed session policy is application semantics, not the same
thing. If users require replacing the session control algorithm by editing arbitrary workflow
structure, that requirement would favor the full-root implementation and its explicit asynchronous
child contracts. U18 requires bounded automatic assessment with preserved authority/continuity; it
does not by itself require that further metaprogramming surface. C's model and method choices still
use canonical source and ordinary admission. Session phase must never decide task eligibility,
erase an uncertain child or bypass the result/effect owners in order to appear simpler.

Faris's unified-invocation alternative is orthogonal to source structure: a Conditional selects which
task becomes eligible; it does not own the external invocation's accepted/physical outcome. Moving
every local attempt to host would add a report-consumption transaction regardless of whether source
is graph or regions. R therefore does not justify that unification. Retain the two fact owners on
Faris's evidence, while removing the separate capability-reducer external-task representation. In the
combined case, a region result may be available before a losing writer is physically settled, and a
published call may still withhold public completion while its resource claim remains suspended.
Neither a region return nor a unified invocation terminal may clear that hold.

## Selected reconstruction and exact migration boundary

The recommendation replaces authorable control structure, not immutable history, effect execution or
logical workspace identity. The following are proposed types/modules, not implemented APIs:

1. **One blueprint source.** `blueprint::program` owns `WorkflowProgram`, stable `ElementId` and
   `RegionId`, `Sequence`, ordered `Conditional`/`Clause`, `Parallel`, bounded `Repeat`, pinned `Call`,
   `Await`, `Return`, and TaskConfig-backed leaves. Conditions/results/capability declarations remain
   bounded typed values. Pure `ProgramEditBatch` applies insert/replace/move/order/result/pin operations
   to a private candidate and validates it before revision creation. The operations cover all those
   constructs, not just choices. `blueprint::document` owns a new semantic version; new writers emit
   this source only. Neither client layout nor an assistance packet owns a second executable graph.
2. **One checked execution plan.** Private `blueprint::execution_plan` lowers a validated source into
   a bounded immutable `ExecutionPlan` exposed narrowly to runtime/control/context consumers. It
   contains stable source-to-execution keys, activation rules, value sources, causal dependencies,
   owned completion boundaries and effect requirements. It supports `AllPredecessors` and current
   legacy fork/join rules as well as `SelectedClause`, sequence and region completion rules. This is
   deliberately richer than a region tree: an old graph reader can lower its exact existing graph
   without proving it decomposes into new source. Plans are derived from immutable revision bytes;
   do not persist an independently editable compiled graph or let its digest replace revision identity.
3. **One runtime.** Runtime consumes that plan instead of traversing `SemanticBlueprint.nodes/edges`
   in each scheduling, reconciliation and context path. Existing attempt admission, final entry,
   cancellation, accounts, signals, clocks and workspace transactions remain. Region execution facts
   record selected clause/parallel result/iteration frontier and result provenance. The old graph's
   exact node IDs remain its execution keys; new synthetic control boundaries derive stable keys from
   owning RegionId and role, not array position. Existing event variants remain readable. New region
   facts receive explicit versioning and cannot be disguised as historical Branch events.
4. **Ordinary external composition.** Adopt the collection binding and Task normalization described
   above. Full TaskConfig is the one requirement/context owner. Deterministic collection/selection
   remains data meaning, either a reused explicit transform or an input expression calling the same
   bounded resolver. Remove capability-reducer dispatch, authority and risk branches after migration;
   a legacy reader lowers their historical restriction into the ordinary task plan.
5. **Complete public adoption.** General authoring preparation/validation/save lives with the pure
   program owner and control orchestration. CLI, prompt-sequence, assistance, fixtures/examples and
   Svelte all emit new program source. Delete duplicated edge/port assembly in their migrated scopes.
   Read models expose source regions, exact pins, accepted region occurrences and selected results.
   Drawing connections edits `ValueSource` or a structural operation; a diagram cannot store executable
   edges which the outline does not know. The same result contract drives context exposure/inspection.

**Software upgrade and existing work.** Keep every supported old revision's original canonical bytes,
identity and ancestry. A version-specific reader constructs its historical semantic graph and lowers
it directly into the single ExecutionPlan; it does not run a separate legacy scheduler. Existing
projection/events, active leases, attempts, wait deadlines, accepted branch choices, child intents,
uncertain effects, resource holds and account reservations retain their exact ownership. Scheduler
admission stays closed through normal startup validation; legacy active occurrences continue under
their governing revision plan. An accepted route is replayed, never re-evaluated against new order.
Old map order is represented explicitly in its derived plan. No result merge or region history is
fabricated for a preexisting run. Old clients reject the new source version and upgrade together.

**Make an old revision editable.** `PrepareProgramConversion` is a bounded pure owner operation over
an exact immutable graph revision. It recognizes sequence, disjoint exclusive alternatives with
declared exits, fork-owned parallel, pinned calls/repeats and waits; extracts stable IDs and data
sources; and verifies the resulting program lowers to an equivalent activation/data/ownership plan.
It materializes old lexical branch priority as explicit clause order. A successful result is a new
revision with provenance to the old source, not changed historical bytes. Identical accepted task IDs,
interfaces and causal references retain their meaning. A mismatch reports the exact unrepresentable
relationship and returns no candidate. `Copy` uses the same conversion for editable independent work;
exact immutable reuse/invocation remains available without conversion.

**Adopt conversion into a paused active run.** Pausing prevents new admission but does not pretend
entered effects have stopped. Conversion then produces an exact frontier correspondence: completed
occurrence IDs/results unchanged; active leaf attempts/calls/waits unchanged under original plans;
accepted branch selection frozen; active structured boundaries mapped only when their selected arm,
ownership and result obligation agree; unstarted descendants mapped by stable IDs. Reconciliation
records this correspondence at an exact run sequence and applies it with the existing stale guards.
Any changed selected arm, reparented active work, uncertain lifetime reassignment, missing old value
or changed required result refuses adoption. The operator can continue the old revision or explicitly
start new work from selected evidence. Later completion never rewrites the earlier revision's facts.

**Compatibility tradeoff requiring visibility in the decision brief.** This review has not proved
that every graph accepted by today's validator has an editable R representation. The selected design
preserves execution, inspection, exact replay and reuse of such revisions through the same plan,
but refuses conversion/editing if equivalence cannot be established. It does not keep a permanent
legacy graph writer behind a “temporary” adapter. If unrestricted future graph edits to every current
accepted graph are a non-negotiable supported-data requirement, R cannot be approved on the present
evidence: select M or first establish a complete conversion proof. Delta recommends R with the
explicit refusal/continue-old-or-create-new behavior. This is a concrete user-visible tradeoff, not
a permission to let implementers invent migration semantics later.

**Concrete protected legacy target.** An old accepted parent `P0` has a protected pinned Call to
`H0`. Its inherited child run has completed investigation `I` and is paused before repair `A`.
The proposed next action is to add a reviewer/Conditional around pending `A`. Even if `H0` has an
equivalent region conversion `H1`, conversion creates source, not authority: direct adoption into
that inherited child remains refused under `check_inherited_agreement_adaptation`. The parent's
new region source must preserve its accepted pin to `H0`; changing the active Call to `H1` would
also change protected/entered structure and refuses. An update cannot quietly call this a Task
replacement merely because lowering ultimately contains tasks. The user can resume the old
child unchanged, or explicitly authorize a distinct new agreement/run that references `H1` and
selected evidence from `I`. Earlier effects, holds, reservations and the old run are still owned;
no copied result proves them settled. If a commitment admits the new run, its ordinary accepted
association consumes that commitment's remaining account, rather than adopting the old entered
run into a fresh allowance. Conversion success therefore does not imply active-edit support.
This is a modeled upgrade/refusal trace grounded in the existing protected-adoption rules, not
an executed conversion or a promise to preserve arbitrary legacy edits.

That example retains a refusal which already exists; it does not demonstrate that previously
supported edits survive. P01 has a positive compatibility obligation as well as negative cases.
The minimum supported corpus is current finite model-editor output, prompt-sequence output,
managed protected root methods with their permitted Task-only prospective investigation/repair,
and existing fork/join/parallel and pinned-call examples. Their currently supported operations
must remain usable through the new source owner. An implementation cannot satisfy migration by
refusing conversion for every member of those families. The exhaustive set of arbitrary valid
hand-authored graphs remains the separate consequential compatibility question; a finite passing
corpus must not be labeled a whole-language conversion proof.

**Old agreements need an actual representation adapter.** Current
[`protected_digest`](../../../../../crates/blueprint/src/agreement.rs) hashes workflow/blueprint
identities, metadata, interface, full noneditable nodes and all boundary-crossing edges, including
their old identities and port/binding structure. Equivalent runtime activation alone cannot keep
that v1 fingerprint valid. Place a versioned pure `LegacyAgreementView` producer in
`blueprint::agreement::legacy_v1`, alongside the v1 validator. Its inputs are the current checked
program/ExecutionPlan, exact immutable origin revision and bounded stable conversion correspondence
from source elements/plan entries/value links to legacy node/edge/port identities. Conversion owns
that correspondence; clients cannot submit an authoritative digest or an unchecked mapping.

The producer reconstructs the v1 semantic view from **current** source/plan meaning. It verifies a
complete mapping of protected nodes and crossing edges, exact configs/inputs/results/interfaces,
and no extra activation or data bypass before serialization in the original v1 shape/order. It may
read old identity/material from the origin to reconstruct wire spelling, but may not copy the old
protected nodes while ignoring changes in their mapped current source. Every relevant current plan
entry/link must be accounted for; a new boundary or capability cannot disappear merely because
the map predates it. Editable Task entries project their actual current `TaskConfig` and identities.
Then the original agreement identity, protected fingerprint, prefix/count/requirement checks and
cumulative revision rules apply. An inability to produce this complete exact view refuses conversion
or adoption; agreement identity is never recomputed to bless changed protected facts.

The conversion's origin and stable correspondence are provenance for this derived compatibility
view, not a second editable or persisted graph. The v1 view is ephemeral, cannot be saved as a new
source, and creates no alternate graph writer/scheduler. Runtime/control/publication call one
blueprint agreement-validation entry point dispatching by recorded agreement/source version.
New native program agreements use a new version with explicit stable region/element obligations;
converting a method does not silently upgrade its accepted old agreement to that new format.
Inherited-child origin checks, maximum adoptions and final effect-policy checks remain with their
existing runtime/host owners.

**Positive protected root trace.** Start an old governed root with `repair.begin` accepted,
`repair.end` pending, fixed acceptance/verifier/effect nodes outside the allowed `repair.` prefix,
and fixed crossing edges. In the supported converted source, insert a permitted ordinary
`repair.investigate` task between those two tasks and replace only the pending repair instructions;
preserve endpoint identities, crossing edges and accepted outputs. The derived v1 view must pass
the same unchanged agreement, while prospective reconciliation preserves the completed occurrence
and admits only the pending change under exact sequence guards. Restart must retain that adoption,
account, holds and remaining revision ceiling. Changing the verifier, effect policy, boundary edge
or protected Call must still fail. This is the semantics of the existing useful root repair tested
in `blueprint/tests/agreements.rs::useful_investigation_and_repair_preserve_the_agreement_and_roundtrip`,
extended to the proposed conversion/runtime path. It is required future evidence, not an executed
test result. If the adapter cannot satisfy that positive case, the selected source implementation
is incomplete for this corpus; blanket refusal is not a passing outcome.

**Usable intermediate units.** First replace repeated graph traversal with the checked plan while
old source remains the only writer; prove old revisions and active histories unchanged. Then add
the complete program source/reader/compiler, full public edit operations and refusal-preserving
conversion, with source/public client upgrade in one unit. Only after this complete vertical slice
switch new authoring to R and remove the old graph writer. Keep legacy reader/lowering, not legacy
runtime ownership. Task collection normalization can use the same plan during these units; active
legacy reducer attempts are never replaced. Svelte's region/outline/diagram and assistance routes
then consume the already operative source semantics. No production implementation is authorized here.

Strongest dissent remains M's smaller semantic migration and ability to keep arbitrary accepted
graph editing unchanged. R's positive justification is the removal of repeated authorable structural
truth, not file count or fashion. No comparative authoring success, model superiority or full UML/BPMN
compliance has been observed. The remaining old-graph conversion limit is material and must not be
hidden behind the fact that all eight designed examples have a region representation.

## Discriminating observations and verification

The coordinator subsequently ran Delta's standalone priority diagnostic through production runtime
and redb. Swapping the two true arms' destination-associated port IDs changes the selected destination;
the [exact source, command, hashes and output](priority-runtime-evidence.md) preserve that observation.
This changes the first probe below from proposed observation to executed evidence. It does not execute
any proposed region/ordered-vector behavior or establish frontend/model authoring quality.

No Cargo was run by Delta; the coordinator owns authorized focused observations and documentation
contracts. This document's packets are non-executable design notation. Useful safe probes are:

- Existing-library construction: identical two-true conditions with ports `a`/`z`, then swap only
  identities while holding business order in the task input. Expect current selection to follow
  lexical keys; record this as behavior to migrate, not a bug in replay.
- Build the complete current child-choice with two typed terminal outputs and a downstream task;
  distinguish successful representation from the already-observed direct-merge refusal. Then attempt
  compatible child future repair with/without inherited agreement; expect the protected case to refuse.
- Current All/FirstSuccess + reducer with a failed/missing output: vary required versus optional
  declared data input and minimum count; observe exact eligibility/refusal. This can reveal an accepted
  but unusable composition without equating all missing-output cases with an implementation defect.
- Construct an external capability reducer and an ordinary task for the same operation with an exact
  profile/context need. Record what each can declare and what authority/risk traversal receives. No
  external provider call is needed.

Future implementation acceptance must run D1–D8 through the public authoring and production decision
paths, including crashes after selection/result commit and before successor dispatch, old-version
replay, stale prospective plans, active nested writer uncertainty, exact child pins and protected
agreement refusal. Current cited tests and E07 do not establish those proposed semantics. Root's
whole-system comparison must combine this finding with goal-directed work and other owner reviews;
this specialist analysis does not turn mapped packages into deeply reviewed ones.
