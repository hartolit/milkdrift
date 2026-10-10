# 03 — Define product behavior and compare complete alternatives

**Owner:** independent outcome/model designers, current-design defender and product-value reviewer, coordinated through the whiteboard. **Output:** credible alternatives, loss-and-gain accounts and concept trials. **Prerequisites:** 01 factual dossier and 02 reviewed working goals/trial result. Unresolved goals block only dependent choices. **Stop:** reasoned candidates and unresolved decisions; no implementation.

Read [context and preservation](context-and-preservation.md), the current factual packet/dossier, [deliberation protocol](deliberation-protocol.md), [scenarios](scenario-corpus.md), [implementation practice](../../practices/implementation.md) and [documentation practice](../../practices/documentation.md). Do not adopt the author's preferred model simply because it appears first.

## Define the missing layer between aspirations and architecture

Use the current `working/vision-baseline.md` and its referenced decisions, not the old vision alone. For each important human/agent action, explain what can be created, selected, run, observed, changed, shared and retained; whose work it is; what is promised on success; and what failure or uncertainty permits next. Give concrete allowed and refused cases before selecting crates, menu items or storage objects.

A product model is not just a glossary or a screen map. Distinguish what “reuse,” “run,” “publish,” “change” or “finish” means for the user, even if several actions can eventually share one visual form. Conversely, do not force separate product objects for every implementation responsibility.

For each alternative, link the reviewed goal and decision revision it assumes. A change to the purpose or acceptable tradeoffs reopens 02; it is not an architectural detail to settle silently here. Record your interpretation and recommendation as your own derivation, with consequences and uncertainty, under [whiteboard and decisions](whiteboard-and-decisions.md).

## Begin with outcomes before defending current nouns

Have separate sessions formulate a coherent explanation of the product from the same reviewed working goals, original-source links and observed constraints **before** seeing one another's solution. Keep provisional product preferences separate from facts. Each should explain how a human and an agent plan, execute, observe, change and reuse work; how tools/machines participate; how results and permissions persist; and how the system improves its method.

The task is not to invent a new product. Use existing strengths and lessons. Give each model the hard cases in addition to first use: autonomous permitted repair, callable private methods, direct/remote work, managed tool improvement, multi-authority administration and evaluated reuse. A generic chat/node editor does not qualify by being easy to draw.

After initial positions are recorded, compare them with the current implementation and each other. Candidates may share substantial code and concepts. Keep a current/conservative route, a structural reinterpretation where justified, and credible standard-informed alternatives. Do not manufacture a different architecture for every row or choose alternatives that are obviously weak.

Compare on the common dimensions in the deliberation protocol, with an explicit loss-and-gain account for V01–V12. Separate technical consequences from user/product preferences. A design can win by restoring expression or removing contradictions even if its first implementation has more code.

## Challenge every significant concept; investigate the risky relationships deeply

For every concept family in 01, record its outcome and disposition or open question. Apply a full trial to high-impact decisions: evidence/current behavior, strongest justification, credible replacement/consolidation, affected neighbors, positive and negative countercases, lost/gained abilities, migration, recommendation and reversal evidence.

A retained concept must earn its complexity. A replacement must earn its disruption. Do not retain an unnecessary layer because it has many tests, and do not remove a useful mechanism because the reviewer has not read its consumers. If the same trial addresses several overlapping concepts, keep one explanation rather than repeating it.

## Required deep comparisons

### Operations, blueprints and control behavior

Trace the actual generic task and typed node definitions. Compare the current operation/control split, a smaller or more uniform representation, and standard-aligned semantics. A generic node is a representation choice; it does not automatically eliminate the need to know what work does, when it starts, what data it consumes, or what survives interruption.

For `Task`, `Branch`, `Fork`, `Join`, `Reducer`, `Repeat`, `Wait`, `SignalWait`, `Subworkflow` and `Terminal`, identify distinct behavior and overlap. Ask whether a constraint belongs in core semantics, operation schema, policy, graph structure, adapter, presentation or nowhere. Investigate repeated port/configuration/validation mechanics before proposing a new abstraction.

Test control versus data flow, multiple inputs, conditional activation, parallel lifetimes, synchronization versus data combination, external signals, loops and cancellation. A graph that merely looks simpler must not change scheduling or hide an unsafe inference. A complex graph must not force unnecessary operator steps if one meaningful edit can express it through the daemon.

### Reuse, publication, location and visibility

Test the user's collapsed-reusable-workflow model seriously. Compare current pinned subworkflows, copies, service publications and remote delegations. Which are implementation variants of one user action, and which require different permission/lifetime contracts? Does every reusable method really require the current publication setup? Can it remain understandable without exposing all internal forms?

Do not assume publication is the only remote bridge. Do not assume a remote call transfers workflow ownership. Conversely, do not assume existing boundaries are optimal merely because they protect real obligations. Carry these questions to 04 with explicit alternatives.

### Live work and continuous adaptation

Contrast manual graph construction, agent-proposed work, and mixed human/agent control. Distinguish a one-run repair, a changed saved method, a changed published commitment and an updated tool environment. Ask which can be authorized in advance and which need a new decision.

A solution that makes every change require manual rebuilding/approval may preserve safety but defeat V02/V03. A solution that lets an agent redefine every failed requirement may preserve motion but defeat dependable reuse. Demonstrate useful authorized adaptation and truthful failure using the same model, not two unrelated engines.

### Resources, context, accounts and learning

Review whether installation state, working files, artifacts, causal selection, budgets, evidence, controller state, evaluation and promotion have distinct legitimate responsibilities. Find duplicate ownership and accidental restrictions. Compare delegating a mechanism to existing tools with owning it inside Milkdrift; include the operator burden of the “simpler” external approach.

Treat thresholds, size/count bounds, required schemas, readonly regions, fixed example assumptions and hardcoded presets individually. Some are necessary limits, some should be validated configuration, and some may be wrong. A literal named `MAX_...` is not its justification; making everything configurable is not automatically better.

### Product and interface identity

Compare diagram-first, instruction-first and hybrid use as production interaction models, not disposable prototypes. The living diagram must expose real control/data relationships and execution progress where valuable. Goal entry must lead to a visible, authorized process rather than an opaque autonomous loop. Preserve advanced abilities without making every human start with an architecture lecture.

## Cross-domain implications and root causes

For each preferred alternative, trace downstream changes to schema, runtime, permissions, storage, API, CLI, Svelte design, examples and evidence. Maintain a short “requires / informs” relation to goal and product decisions. Similar vocabulary or a cross-reference is not proof of compatible semantics. A proposed hybrid is a new candidate: check its combined assumptions, user path and failures instead of averaging two approved halves. Ask a neighboring owner to challenge those consequences. Record incompatible pairs of claims, but also missing capabilities and needless coordination that prevent intended work despite technically consistent prose.

Study the relevant history for confirmed problems. Identify whether a stale premise, test blind spot or duplicated owner caused the drift. The proposal must fix that cause; renaming all public nouns or writing a new vision document is insufficient.

Compare the same demanding scenario across candidates, including S21/S26/S28 as applicable. Reviewers add variants within intended scope. Require both useful progress and required refusals. A candidate cannot win by declaring the hard cases outside the product.

## Output and gate

Maintain `working/product-alternatives.md` with the product-behavior model and compared options, and concept coverage/dispositions in the dossier. Link consequential choices to the relevant product-model whiteboard topics and their current decision revisions; do not duplicate the topic's current answer. Record the chosen dimensions, losses, uncertainty and dependencies. Update shared context with established facts; keep recommendations labeled.

**Gate:** significant concepts are accounted for; credible alternatives preserve or explicitly negotiate product value; no option wins solely through line count, narrow demo success or inherited confidence. Full-depth unresolved decisions have discriminating questions, not vague requests for more research. Commit coherent planning decisions and proceed to ownership/federation review.
