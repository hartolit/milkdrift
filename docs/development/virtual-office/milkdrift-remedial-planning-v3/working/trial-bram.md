# Bram — independent position for the first review trial

Position revision: Bram trial 1. Source HEAD:
`908e7893f5dadb84d12712573c8daaa946829e39`. This is my authored position, not the trial's integrated
result. I wrote it after [my independent intake](intake-bram.md) and before reading Ada's account,
Ada's trial position, another reviewer's conclusion or the coordinator's selected answer.

After intake I read the v3 phase-02 assignment, trial guide, deliberation protocol, context and
preservation seeds, scenario corpus and whiteboard procedure. Broad combined output was partly
truncated; I make no claim of exhaustive packet reading. I also read the implementation practice
and inspected the source/document anchors below. The coordinator confirmed that no original
conversation export or v1/v2 source packet had been found. U07 remains selected excerpt evidence.

## Question and surrounding system

The user wants to avoid rebuilding a reusable workflow and may want to execute work on other
machines. U07 explicitly questions whether a method is the bridge between those needs.
Restricted service invocation is established current behavior and a valuable candidate outcome;
the excerpts do not independently establish its precise contract as an explicit user instruction.
U03 does explicitly require that standalone frontend design account for multiple daemons,
authorization and capabilities.

The upstream fact a client consumes is an authorized definition or capability description with
an owner and an exact revision/generation. The downstream promise differs: an ordinary nested
workflow continues under its parent's execution authority, whereas a callable service accepts
public inputs and executes under its configured service authority. Remote placement describes
where an accepted capability operation occurs. The effect owner, accepted identity and recovery
path must survive a client disconnect. Neighboring concerns include private internals, who can
change future work, output disclosure, and whether the destination contains a workflow runtime.

## My recommendation and its strongest opponent

I recommend retaining **distinct author actions for reusing a workflow and calling a service**,
while allowing a shared collapsed visual form where its label and inspector state the real
boundary. Placement should be chosen for capabilities where supported; it should not require the
author to pretend that an ordinary remote process is a reusable workflow service.

This is a product-model recommendation, not a request to preserve every current Rust type or to
add a mandatory menu of technical classifications. For example, the actions could be “Use this
workflow” and “Call this service.” Before commitment, each action should explain what is pinned,
who controls execution, which results are visible, and what edits are possible. Their exact words
need design evaluation.

The strongest alternative is a **single workflow-use interaction** that discovers the chosen
target's available rights, then guides the person to local reference, independent copy or
restricted service invocation. Its strongest case is U07's simple explanation: reuse should feel
like selecting familiar work, not learning an internal taxonomy. A shared interaction could
reduce repeated catalog browsing and remain truthful by showing boundary consequences before
commitment. This alternative does not inherently weaken safety.

My objection is that “use” may appear to promise transferable understanding and control when the
same gesture sometimes selects an inspectable workflow and sometimes delegates to an opaque
service owner. A later permission gain, grant revocation or promotion can make that ambiguity
visible at the worst time. Distinct entry actions make the difference easier to teach before the
operation, at the cost of exposing another choice. Source inspection cannot decide which design
people will understand better. That value preference remains disputed; there is no measured
usability result in this position.

Mandatory publication for all reuse is not a credible default on the supplied user grounds. It
would introduce service setup for a person merely referencing their own existing workflow. Nor
does “everything is one generic text node” explain who records a child run, rejects a missing
result, or owns cancellation. Either would need a stronger independent argument before replacing
current behavior.

## Competing predictions, fixed before the modeled comparison

These are predictions for the comparison below, not results from an executed test.

| Case | Distinct author actions | Unified workflow-use interaction |
| --- | --- | --- |
| Same owner, readable reusable workflow, no publication grant | Can create an ordinary reference without publication setup. | Can do the same if its chooser routes by supported capability and authority rather than assuming publication. |
| Another owner, callable service, no internal graph inspection | Call action can promise only the public contract and authorized results. | Can preserve the contract, but must explicitly retract any default expectation that the collapsed item can expand into editable internals. |
| Exact remote process on an execution-only host | Placement selects the process capability without a method publication. | Must leave workflow reuse or broaden “use” to capabilities; a method-only interpretation fails. |
| Permission to inspect service internals is added later | Additional inspection can appear while service authority and accepted invocation remain unchanged. | Risks implying conversion to an ordinary editable child unless the interaction preserves an explicit service classification. |
| Submission reply disappears after service acceptance | Recovery uses the accepted service/invocation owner and exact request identity. | Same requirement; unification gives no right to restart the work locally or infer failure from disconnect. |

Both credible alternatives can meet execution and authority requirements. Their discriminating
prediction concerns comprehension: after performing the first two cases, can a fresh reader
correctly predict whether expanding the item reveals internals, whether editing it changes the
service, and which identity recovers a lost reply? No human-participant comparison was run here.
A later independent reader can identify a contradiction in the modeled interaction; that is
useful evidence, but it is not a measured usability claim.

## Narrow source-grounded comparison

I modeled three calls rather than starting live daemons or substituting fake permission results.
No Cargo or external calls were made. Source observations are scoped to the HEAD above.

1. **Local nested use.** `NodeKind::Subworkflow` carries a `PinnedSubworkflow`; its owner contains
   workflow/revision/interface facts. In
   [the runtime child owner](../../../../../crates/runtime/src/engine/structured/subworkflow.rs),
   child creation loads the exact revision and copies the parent's execution-authority basis,
   accepted agreement where present, and published-source binding where present. Stable child
   identity lets interrupted creation be recovered. This supports distinct local nesting without
   evidence that publication is a prerequisite. It does not prove a finished GUI authoring path.
2. **Restricted service use.**
   [Protocol commands](../../../../../crates/control-protocol/src/command.rs) separate method
   preparation, publication and administrative inspection. The
   [public contract](../../../../reference/control-api.md#published-workflow-capabilities)
   describes callable discovery without graph or service identity, a configured service grant,
   and caller-owned terminal-output reads. This is more than a collapsed node: the boundary
   permits invocation without granting general internal inspection. The distinction is supported
   by current contracts, not by treating U07 as unqualified approval of publication machinery.
3. **Remote direct/process work.**
   [The capability-host explanation](../../../../../crates/capability-host/README.md) distinguishes
   direct input selection and durable serving ownership from workflow context and runtime
   ownership. Process/fresh-model operations can be hosted independently. Source in
   [daemon configuration](../../../../../apps/daemon/src/config/compile.rs) refuses publication
   services without the workflow-enabled role and enabled accounting. Thus an execution-only
   destination does not become a workflow-service host merely because the caller draws a
   collapsed box. Remote execution and published-method invocation are not interchangeable facts.

The comparison rules out collapsing these contracts into one execution meaning. It does **not**
rule out one well-designed selection interaction over multiple meanings. That is precisely the
unresolved preference between my recommendation and the strongest alternative.

## Real conclusion and what would change it

Preserve the three semantic concerns in the planning baseline: ordinary reuse, restricted
service invocation, and execution placement. Do not require a different visual glyph for each
unless the resulting diagram becomes more truthful or comprehensible. Do not make publication
the prerequisite for every reusable workflow, or make a service invocation imply graph access.

I favor distinct author actions because the control and disclosure difference is consequential.
I would reverse this preference if a concrete unified interaction shows all three cases without
repeated boundary explanations, conveys the changed control relationship before commitment, and
a fresh critic can accurately reconstruct the successful, denied and interrupted paths. Merely
giving the same card a different subtitle is not enough if accepted requests still appear locally
editable. Conversely, if distinct actions force people to choose terms they cannot understand
before seeing their target, the assisted unified chooser may be the better design.

Downstream architecture can preserve authority and durable owners now. It cannot claim the user
selected my author-action vocabulary. Frontend design should carry both credible interaction
options until a concrete comparison resolves their costs. Runtime migration is not justified by
this trial alone; source does not establish a defect in current ownership.

## Labeled hypothetical premise change

**Hypothetical only:** the caller gains permission to inspect the service's internal blueprint.
The GUI may now offer an authorized graph view, and later design work must reconsider the
collapsed-item expansion and denied-state copy. The accepted service call still runs under its
configured service authority. Inspection does not imply permission to edit, publish or convert
that call into a local child. Invocation identity, exact generation, private output rules and
lost-reply recovery remain attached to the existing owner. My preference for distinct actions
weakens slightly as inspection becomes similar; its authority distinction remains.

**Second hypothetical:** the selected destination changes from workflow-enabled to execution-only.
Any plan to publish/execute its internal workflow must be reopened or explicitly refused. A direct
process or fresh-model operation can remain a valid candidate if separately authorized and
configured. The frontend must revisit target eligibility, service catalog state, accepted
obligation handling and diagnostics; it must not relabel the same request and continue.

Neither hypothetical is an observed source defect or a real change to the live baseline.

## Actual cross-examination after reading Ada

I subsequently read [Ada's trial position](trial-ada.md), including its independently written
predictions and reversal conditions. This section is a later contribution; it does not replace
or retroactively broaden the exposure claimed for my initial position.

Ada's common discovery followed by explicit distinct committing actions resolves the strongest
semantic objection I raised. Finding two items through the same search does not imply the same
execution authority, and Ada explicitly rejects a common backend object or permission. My initial
term “distinct author actions” already permits that separation. There is therefore less actual
disagreement than a headline contrast between “unified” and “separate” might suggest.

It does not yet resolve the interaction objection. A common result list can still imply that
every found workflow can be expanded, copied or called interchangeably. The comparison must show
what is visible before selecting a result, and what changes when a person lacks internal read
permission. A disabled “expand” button can itself suggest knowledge of private internals; a
generic call card with no boundary label can suggest ownership the caller does not hold. Ada's
proposal avoids these failures in its stated contract, but no concrete production interaction has
yet demonstrated the contract.

I would permit common discovery as a candidate without blocking architecture work, provided the
committing actions remain explicit and no implementation assignment treats its comprehension
benefit as proven. The remaining test is not whether a critic remembers our taxonomy: after
finding the same named work at two owners, can they predict whose version will execute, what they
may inspect or change, and how a lost submission reply is recovered? A design can use concise
language and progressive disclosure, but cannot defer those consequences until after acceptance.

Ada also exposes a preference I underweighted: reference versus independent copy. These must not
be presented as equivalent forms of “edit one use.” A pinned reference can retain shared starting
meaning while one accepted run later adapts; an independent copy creates a new definition lineage.
The available evidence does not choose their default ordering. I agree this should remain a
visible, conditional product choice, without claiming that the user definitely wants both in every
entry point.

The trial's current source evidence justifies preserving the distinct execution meanings and
rejecting mandatory publication as an unexamined prerequisite. It does not establish a winning
navigation pattern. I retain my preference for discoverable separate routes alongside any common
chooser, because frequent operators should not have to repeatedly disambiguate a known task. That
is a disputed convenience preference and may be reversed by the concrete comparison.
