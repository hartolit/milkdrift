# The proposed executable workflow and diagram language

Profile **NP3**, selected planning recommendation under
[N1/r3](../../whiteboard/discussions/implementation/executable-notation.md), supersedes NP2's
paired Branch/ExclusiveMerge source proposal. New workflows have one canonical structured program;
the graph and accessible outline edit that same source through daemon operations. Runtime executes
one derived checked plan. Existing accepted graph revisions retain their exact meaning through
versioned reading/lowering. This is a reconstruction proposal, not implemented behavior, standards
conformance or user authorization to execute the implementation program.

The comparison now covers the same demanding inputs through current child-call encoding, a complete
smaller graph correction, canonical regions, and an executable BPMN basis. See
[Delta's source/operation comparison](../working/structure-comparison.md), including concrete authoring
packets, future change, migration and dissent. The
[prior NP2](../working/notation-profile.md) remains historical evidence of the earlier recommendation.

## What becomes canonical

`WorkflowProgram` has stable element/region identities, Sequence, ordered Conditional clauses with
matching result mappings, Parallel with explicit completion policy and result exposure, bounded
Repeat, pinned Call, Await, Return and ordinary capability tasks. A region's position in an array is
not its durable identity. Every external composition uses ordinary task requirement/context/authority;
a collection binding supplies selected results instead of introducing another executor kind.

Control order comes from structure. Typed value sources carry data dependencies; the compiler derives
the runtime readiness and causal indexes. A person or agent does not maintain both an output binding
and a second matching edge. The diagram can still show separate control and data relationships, but
those lines project one source. Dragging or connecting submits a structural/value edit and displays
the daemon's accepted result. Layout moves remain outside revision identity.

The internal ExecutionPlan supports both region activation and exact legacy graph activation rules.
It is derived from an immutable revision and is never a separately editable persisted graph. Runtime
occurrences, attempts, workspace values, child acceptance, authority and physical resource obligations
remain different facts. Neither region containment nor a familiar symbol creates a new grant or
proves that an external writer stopped.

## Standards principles with executable consequences

The selected Conditional takes substantive ideas from UML 2.5.1 §16.11.3.3: clauses, predecessor
ordering and each clause's mapping to the conditional's result pins. It restricts tests to pure
bounded conditions and a total explicit order. Missing required results fail; optional absence is
explicit. This avoids relying on unspecified selection among several enabled alternatives or an
implicit null result. Structured Sequence/Loop ideas also inform the source, while real effect
cancellation remains an observed durable lifecycle. These are declared departures from general UML,
not a conformance claim. [UML normative specification](https://www.omg.org/spec/UML/2.5.1/PDF).

BPMN's ordered exclusive routing and pass-through merge were evaluated as execution semantics.
An unrestricted merge can forward multiple arrivals; it does not imply one owned choice result.
Parallel token synchronization also needs explicit error routing to represent a failed attempt's
outcome. Those differences, plus live instance adoption and scoped data/authority extensions, make
full BPMN execution a separate product choice. This proposal does not claim BPMN interchange or
silently mix a second token model into region execution.
[BPMN normative specification, §§10.6.2, 13.4.1–2](https://www.omg.org/spec/BPMN/2.0.2/PDF).
The [standards evidence](../working/standards-evidence.md) records prior inspected clauses/PDF identities;
Delta's comparison adds the ConditionalNode result contract rather than merely adopting its glyphs.

## What a person sees and can predict

| Visual or authoring action | Saved meaning and outcome |
| --- | --- |
| Task with typed inputs/outputs | One full capability requirement/context declaration. Instructions cannot grant authority or replace a required operation contract. |
| Ordered choice region | First matching clause in explicit semantic order, distinct default, compatible selected results and one continuation. Renaming generated IDs does not change priority. |
| Parallel region with named branches | Isolated concurrent work, visible All settled / Any completion / First success / Quorum policy, explicit result exposure and loser treatment. Completion does not certify usable results. |
| Result composition | Deterministic selection/collection or an ordinary external task receiving exact result/outcome references. First by declared result order is distinct from earliest completion. |
| Reusable call | Exact owner/revision/interface and linked execution. Collapse or authorized expansion changes no pin, grant or child ownership. |
| Wait | Declared deadline or precise event type, correlation, delivery mode and occurrence target. Browser timers never drive execution. Current limited SignalWait is not relabeled as an implemented arbitrary correlated catch. |
| Bounded repeat | Explicit body, condition timing, iterations/budgets and limit disposition. Iterations and child calls retain the accepted account; none silently resets allowance. |
| Result/return | Declared outputs and logical outcome. Resource/account uncertainty can remain visible after a useful result exists. |
| Edit future work | New immutable candidate plus exact affected frontier. Completed work and selected clauses remain fixed; compatible pending descendants can change. |
| Old graph revision | Exact accepted historical/execution view. Editing first requests a checked conversion; unsupported conversion is visible and preserves the old revision. |

Choice regions retain accessible identity and order at every selectable zoom; compact views aggregate
explicitly rather than showing indistinguishable diamonds. Parallel completion and composition show
separate statuses. Every interaction has keyboard/outline equivalents. Authorization/connection state
stays visible where it changes the next action. A private service keeps the public invocation
breadcrumb; losing internal read permission clears that private view. A lane is a view of its stated
ownership/placement dimension, not an operation that relocates work or mints authority.

## Selected results, repair and recovery

A selected clause produces only its declared result mapping. Data from an unselected alternative
cannot become a required continuation input through a hidden bypass. Common ancestor inputs remain
valid. Runtime records selected clause, exact producing occurrences and result references atomically
with the region's logical completion boundary. Replay uses those facts; it never evaluates today's
inputs or creates another continuation after a lost reply.

If investigation is complete and repair is pending within the selected clause, an authorized revision
can replace the pending repair while retaining the choice, completed investigation and result
contract. Replacing a whole active region solely because one descendant changed would defeat the
product's adaptation requirement. The planner compares stable descendant identities and ownership
as well as result interfaces. Changing the selected clause, reparenting active work, changing consumed
inputs or changing a required result contract refuses that adoption.

A nested first-success result may be usable while a losing writer remains uncertain. The region does
not erase that writer, its hold or its reservation. Normal entry checks still control later work.
A restricted published call may continue to withhold public completion until its own editing/hold
obligations are satisfied. Logical continuation, workflow completion and physical quiescence are
separate inspector facts.

Current protected agreements permit Task-only adaptation inside a fixed boundary; their children
cannot escape the inherited pin through direct adoption. New region source does not silently broaden
those permissions. Exploratory S28 work can use ordinary authorized prospective revisions. Changing
an accepted protected agreement requires a distinct agreement/run decision with earlier evidence
preserved.

## Existing revisions and the product tradeoff

Old immutable graph bytes, revision identities, receipts, events and already selected routes are
unchanged. Their reader lowers directly to the same checked execution plan with historical activation
rules, including port-key branch precedence. New Conditional order is explicit. The
[production-runtime probe](../working/priority-runtime-evidence.md) confirms why this is a semantic
change: swapping two port IDs changes the chosen business route even with identical true predicates.
No reinterpretation of those historical selections is authorized.

A checked conversion creates editable program source as a new revision only if lowering preserves
control/data/ownership semantics. Conversion into a paused active run additionally requires exact
frontier correspondence under stale-sequence guards, with active/uncertain work retained under its
original governing plan. Pausing does not assert physical stop. No second legacy scheduler or
permanent legacy writer is hidden behind a temporary adapter.

For example, a legacy protected parent pins child `H0`, whose investigation has completed and
repair is pending. An equivalent source conversion to `H1` does not authorize adoption into the
inherited child or retarget the already active protected Call. Adding a review/choice still refuses
that adoption. Resume `H0`, or separately authorize new work with `H1` and selected earlier evidence;
the old run's effects and obligations remain owned. This modeled case makes the compatibility
limit concrete; it is not executed upgrade evidence. The full trace is in Delta's comparison.

That is an already-existing refusal, not proof of preserved editing. P01 must positively retain
the current model editor and prompt-sequence outputs, managed protected **root** Task-only repair,
and existing parallel/pinned-call examples. In particular, completed `repair.begin` followed by
pending `repair.end` must still admit an allowed `repair.investigate` task and pending task changes
inside the same unchanged old agreement. Protected verifier/effect/boundary changes still refuse.
Blanket conversion refusal cannot satisfy this supported corpus.

Blueprint's pure versioned `agreement::legacy_v1` derives a `LegacyAgreementView` from the current
checked source/plan, exact origin revision and checked stable source-to-legacy correspondence.
The old fingerprint includes node/edge identities and configs, not only equivalent activation.
The adapter reconstructs and validates the complete current v1 protected shape and original
Task-scope rules; it cannot copy an old digest while omitting new or changed source behavior. The
view is ephemeral compatibility data, not another persisted graph or writer. New native agreements
have versioned program obligations; existing accepted agreements are not silently upgraded.

There is a material compatibility limit: this planning review has not proved that every currently
accepted graph is representable as editable regions. An unconvertible graph remains executable,
readable, exactly replayable and reusable; its conversion/edit is refused with the precise relationship
that could not be preserved. The user can continue it or create new work with selected evidence.
If preserving unrestricted graph editing for every old accepted shape is mandatory, the smaller M
correction remains the better choice until complete conversion is established. This consequence must
remain in the decision brief; successful region examples do not settle it.

## What implementation must prove

The same D1–D8 packet in the [comparison](../working/structure-comparison.md#equal-authoring-input-and-independently-chosen-outcomes)
must pass through ordinary public authoring and runtime decisions: overlapping conditions and renamed
IDs, failed/missing parallel results, uncertain losers, pinned nested reuse, positive pending repair,
crash recovery, targeted decision replay and bounded exhaustion. It must also reject forbidden data
escape, stale revisions and protected-agreement changes. Old-version fixtures and active-frontier
conversion/refusal need independent expected outcomes. Diagram, outline and machine source must
reload the same semantics; unknown constructs cannot disappear on save.

No paid model comparison or real frontend usability experiment was performed. Normative reading,
source inspection and the two controlled current-behavior probes justify architectural decisions
and reveal limits; they do not establish future implementation acceptance. First human use remains
a test of the real standalone Svelte client after the selected source/daemon boundary exists.
