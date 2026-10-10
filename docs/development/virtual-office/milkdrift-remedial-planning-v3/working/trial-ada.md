# Ada: independent position for the early review trial

This position follows my preserved [first intake](intake-ada.md). Since that checkpoint I have
read the shared v3 context, phase 02, trial and deliberation instructions, decision procedure,
and scenario corpus, plus the sources below. I have not read Bram's intake or position or a
coordinator decision. The source checkpoint remains `908e7893f5dadb84d12712573c8daaa946829e39`.
The coordinator confirmed that no original transcript was located in the repository, its path
history or the adjacent prompts workspace. The selected excerpt limitation therefore remains.

## The question and my position

The question is what a person should have to choose in order to reuse work, invoke a service
without internal privileges, or use a different machine. U07 supports reusable workflow behavior
while explicitly questioning its relation to remote execution. It does not require any current
object taxonomy. U02–U03 require a daemon client that respects independent ownership and authority.

I favor **one discovery interaction with explicit distinct actions**, initially presented around
the work the user wants to do. A user can find a saved workflow or offered operation in a
searchable chooser, but must choose an action with a concrete consequence: reuse the exact
workflow definition, make an independent editable copy where permitted, or invoke an offered
operation under its public contract. The UI should explain the selected owner, version and
available control before committing the choice. “Use this work” can be a common entry point;
it must never imply one backend identity, shared permission, or the ability to expand internals.

This is a recommendation about interaction and product explanation, not a proposal to introduce
a new generic call registry or durable union object. It keeps the existing semantic operations
until a complete replacement demonstrates better behavior. It also challenges a UI organized
primarily around the present Rust/publication taxonomy: the operator's task should lead them
to the distinction, rather than require mastery of that taxonomy before they can begin.

## Surrounding system and evidence

The input is a permitted definition or advertised operation at an identified daemon. Definition
reuse consumes an exact workflow revision and interface. A published service consumes a public
contract tied to a generation and uses its configured service authority for internal work.
Remote delegation consumes capability selection, peer authority and an accepted serving request.
These differences determine what the caller can inspect, adapt and cancel, and which owner can
recover the operation. A UI connection or visual expansion cannot add such rights.

Source inspection establishes that
[PinnedSubworkflow](../../../../../crates/blueprint/src/model/structured.rs) contains an exact
workflow, revision and interface, while
[NodeKind](../../../../../crates/blueprint/src/model/node.rs) puts that reference in a distinct
subworkflow variant. The runtime has its
[structured child owner](../../../../../crates/runtime/src/engine/structured/subworkflow.rs).
I inspected the type and located the runtime paths; I have not independently traced all child
recovery and authority behavior, so I do not treat it as an executed pass.

The [published-method guide](../../../../guides/published-methods.md) documents a constrained
invoker with declared result access and no implied internal-run access. It distinguishes accepted
call retirement from new admission and says lost create/start replies recover the same internal
run. Those are documented claims for the source investigator to verify. Configuration source
directly checks that publication services require workflow-enabled role and controller accounting
in [config compilation](../../../../../apps/daemon/src/config/compile.rs); it would be incorrect
to promise local workflow publication on an execution-only host merely because its remote
capabilities look callable.

The owner of a consequential effect remains the relevant runtime, host and resource operation,
under daemon authority. Shared browsing creates no new effect owner. A disconnect must leave
the exact accepted operation recoverable through its original owner. Neighbor concerns are
public result disclosure, active-generation retirement, cancellation acknowledgement, and
capability disappearance during selection. A common chooser must explain each without
inventing workflow state for direct invocations.

## Credible alternatives and what separates them

| Candidate | Strongest case | Cost or countercase |
| --- | --- | --- |
| Distinct entry routes for workflow reuse, method invocation and direct/remote capability work. | Makes authority and lifetime differences explicit early; closely matches working source and can be implemented without inventing a common selection contract. This is a serious baseline. | Users may have to classify their intent in internal terms before discovering the available work. A renamed or moved implementation can force them through an apparently unrelated route for the same practical need. |
| Common discovery with explicit action consequences, my initial preference. | Finds the available work by user intent, while preserving exact owner/version/authority decisions in the action. It may reduce navigation and explanation without a backend redesign. | A common entry can still encourage false interchangeability. Disallowed or unavailable choices must have meaningful explanations without revealing private objects. The alleged comprehension benefit is untested. |
| Publish every reusable workflow as the single call mechanism. | Gives every call a stable advertised contract, generation, bounded internal run and constrained service identity; avoids teaching an additional pinned-child call form. | Publication becomes an administrative prerequisite for local composition and can change who owns authority and recovery. Execution-only hosts still cannot own such implementations. This candidate needs a value and migration argument, not merely a uniform box. |

A “single generic instruction node does everything” is not a credible complete alternative yet:
there is no supplied account of its durable authority, dependency and recovery obligations.
It could become credible with that account; a smaller enum alone would not discriminate it.

The consequential preference is whether ordinary reuse should primarily mean **an exact callable
reference** or **an independently editable copy**. Both can be useful; the user has not specified
which should be offered first or how often both matter. U07's “collapsed into a node” leans toward
an exact call but is explicitly tentative. The UI recommendation must preserve both supported
uses and keep a default ordering conditional; it cannot delete copying or force publication on
the assumption that the collapsed-node phrase settles the question.

## Prospective discriminating case and predictions

These predictions are recorded before any new observation. No trial has yet been run by me.
The narrow first trial can be a modeled source-backed operator trace, followed later by a real
browser comprehension test. It must not pretend that a mocked screenshot proves backend behavior.

Use one reviewed two-step workflow and three principals:

- Its author may read and author its exact revision and compose local work, but lacks publication
  administration. They want to reuse it twice with different inputs, then modify only one use.
- A customer may invoke an already published version and read their declared results, but may
  not inspect internals. They want to see progress after disconnect without obtaining the graph.
- An operator may directly invoke a process capability on an execution-only host. They want that
  operation to remain usable even though the host cannot own workflows.

For each candidate, write the initial action, submitted operation, authority prerequisite,
accepted identity, visible progress, and lost-reply recovery before consulting current expected
test outputs. Compare the number and nature of unexplained choices; do not invent an aggregate score.

My predictions are:

1. Mandatory publication will block or add an administrator/service-configuration dependency to
   the author's otherwise permitted exact local reuse. If source shows ordinary subworkflow
   reuse already requires the same publication authority, this prediction is false and the
   proposed separation needs reconsideration.
2. Both distinct routes and common discovery can preserve the customer's privacy and exact
   replay, provided the action exposes only the declared public contract. Common discovery
   fails if the user can only understand progress by expanding the private graph, or if the
   client infers hidden workflow state from a generic operation status.
3. The execution-only case will remain a direct operation under both the first two candidates.
   The mandatory-publication candidate must either admit a separate direct-operation exception
   or lose this behavior. If its “one call” model already permits capability operations without
   publication, it is a unified presentation rather than universal workflow publication and
   should be evaluated as such.
4. The common chooser's claimed comprehension benefit cannot be established from source alone.
   A fresh reader should be able to predict which one-use edit affects which future calls and
   which old executions remain pinned. If separate routes make this clearer, I would reverse
   the interaction preference while retaining the semantic distinctions.

Record source facts and modeled consequences separately from observed use. Cargo execution,
paid calls and live deployment are outside my assigned intake/trial-draft scope. The coordinator
can use independent source evidence now, with browser usability and integrated recovery evidence
explicitly assigned to the later implementation acceptance program.

## Labeled hypothetical premise changes

**Hypothetical only:** the customer gains internal read permission. They may now receive an
authorized link to internal execution/definition views. That changes inspection and possible
visual expansion, not the accepted call generation, service authority, child owner or replay key.
Read permission alone still does not permit modifying the service or its accepted run.

**Hypothetical only:** the chosen destination is execution-only. Direct capability calls remain
eligible, but local workflow composition and publication ownership are unavailable. The frontend
must route composition to a workflow owner selected by the operator and show the remote operation
as a dependency, or truthfully refuse the requested creation there. It cannot silently upgrade
the host role or relocate an existing workflow.

**Hypothetical only:** evidence establishes that all targeted users want independently editable
copies and never pinned composition. That would reopen the default reuse interaction and the
need to expose pinned composition prominently. It would not justify rewriting old pins, exposing
private service internals, or removing the current mechanism without a migration decision.

## Dependency and reversal

Product-model, notation and frontend work may rely on the separate meanings of exact reuse,
service invocation and operation placement as a planning hypothesis supported by source. They
may not rely on common discovery being easier, copying being the user's preferred default,
or every backend path already having a complete browser representation. Architecture can retain
the existing owners while those interaction questions are compared.

The strongest objection to my recommendation is that it merely hides a complex taxonomy behind
one extra chooser, increasing the number of concepts without improving decisions. A fresh
reader who cannot predict ownership, permissible edits and recovery would establish that problem.
I would then prefer explicit separate routes, or a narrower task-specific entry, rather than
defend uniformity for its own sake.

## Cross-examination contribution — Ada, 2026-10-10

I have now read [Bram's independent position](trial-bram.md). The strongest disagreement is when
the operator should encounter the distinction between reusing an inspectable workflow and calling
an opaque service: before browsing possible work, or when choosing the committing action for a
selected result. We agree that the accepted operation must preserve distinct authority, version,
disclosure and recovery meaning. This is an interaction disagreement, not evidence of conflicting
execution semantics or a discovered source defect.

Bram's strongest objection changes the design obligation: a common chooser cannot offer a generic
“Use” confirmation and rely on a subtitle or later inspector to reveal that the choice delegates
to a service owner. Its committing actions must instead name the consequence in the same way as
separate routes. For a service, the action promises public inputs/results and identifies its owner
and generation; expansion is not implied. For an exact workflow reference, it identifies the
pinned revision and explains that a change to a saved definition does not rewrite an accepted
execution. Independent copying is a different action with a new lineage and permitted contents.

That combined interaction addresses the semantic part of Bram's objection only if these facts
remain visible after commitment and through later permission changes. Gaining read permission
must add an inspection action, without relabeling the service as locally editable. Losing a
permission must revoke the relevant action without relocating accepted work. Reconnecting must
recover the original accepted identity. These are concrete acceptance conditions for the combined
candidate, not inferred benefits of a shared chooser.

The combination therefore resolves the apparent conflict over a single generic committing
action, because I reject that action. It does **not** establish that shared browsing is easier
than distinct entry routes. It can still hide complexity by delaying a meaningful choice until
the operator has invested effort in a target. Conversely, distinct routes can impose unexplained
technical categories too early. A fresh reader should evaluate both complete task paths; a
choice of navigation may be a coordinator recommendation, but cannot be reported as measured
usability or an explicit user preference.

I narrow my recommendation accordingly: common discovery may be used for planning only with
separate explicit committing actions and persistent boundary identity. No implementation task
may interpret this as a generic durable call object, automatic publication, automatic copying,
or the transfer of service authority. If the actual screen design cannot carry those consequences
clearly, separate entry routes are the fallback. Bram's position usefully exposes this failure
condition; it has not been defeated by merely renaming the chooser.
