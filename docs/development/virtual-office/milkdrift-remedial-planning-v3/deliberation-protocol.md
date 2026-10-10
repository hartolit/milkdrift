# Deliberation — make the surrounding structure inspectable

Use the existing [whiteboard rules](../whiteboard/README.md). This is a method for this review, not a new authority system or issue tracker. Its purpose is to prevent a locally plausible answer from concealing a globally wrong relationship.

## 1. Give every session the same facts, not the same preferred answer

The coordinator maintains `working/context.md` with a source identity and document revision. For primary-source intake in 00/01, first provide original excerpts/history, explicit constraints and source references without the package's outcome synthesis or the coordinator's preferred interpretation; compare with that synthesis after initial accounts are recorded. Subsequently every specialist receives the reviewed working outcomes, system map, relevant successful and failed traces, constraints, open dependencies and source links, each retaining its source/claim standing. Attach a focused task; do not replace the context with that task.

Keep the **factual packet** separate from authors' preferred proposals. During initial alternative generation, reviewers may see the existing code and the user's intent but not another designer's conclusion. Later critics receive the candidate designs, but write initial criticism before reading other critics' reports. Record what was actually shared. Different pseudonyms do not establish independence.

Before making a consequential recommendation, a reviewer supplies a brief, public account of the surrounding system: the user outcome, upstream fact it consumes, downstream assumption it creates, owner of a consequential effect, recovery path, and a relevant neighboring concern. This is an explanation of the design, not a request for private chain-of-thought. If it cannot be established, mark the missing link and investigate it.

The packet is an index and explanation, not a dump of the repository. Link authoritative detail and include the dependencies that would change the answer. If a source or decision changes, list affected recommendations and request targeted rereading. A context reset must not silently erase rejected alternatives or resurrect stale assumptions.

## 2. Reconstruct the causal path, then look sideways and backward

For a major operation, document the meaningful transitions from user intent to observed result:

| At each boundary, establish | Why it matters |
| --- | --- |
| Input and origin | What fact arrives, who supplied it, and which context gives it meaning? |
| Owner and identity | Which component/host records it, and what distinguishes it from another occurrence or request? |
| Authority and prerequisites | Who may request/observe the action, and what must already be true? |
| State and lifetime | What is committed, temporary, retained, editable, pinned, or still unknown? |
| Output and consequence | What can the next consumer safely conclude; what can it not conclude? |
| Interruption and recovery | What happens if the transition or reply is missing, repeated, delayed, or reordered? |
| Human operation | What can the person see and do through the public client rather than an internal shortcut? |

Do not force one giant linear trace where the operation branches or has independent lifetimes. Represent parallel paths, alternative outcomes, asynchronous callbacks and partial observations explicitly.

Then apply three checks:

**Backward justification:** start at the promised outcome and establish each necessary prerequisite. Does the actual path provide it, or merely a similarly named fact?

**Neighbor agreement:** ask a producer-side and consumer-side reviewer to state the meaning independently. Compare them. An accepted request is not a completed action; completed work is not necessarily accepted output; a saved definition is not permission to execute it.

**Composed lifetime:** combine the path with another supported action: concurrent edit, nested call, resource maintenance, grant change, reconnect, promotion, cancellation. Do two individually valid features now contradict each other or stop progress?

Choose interactions by actual shared state/meaning. Do not exhaustively enumerate irrelevant pairs. The [worked examples](worked-coherence-traces.md) show the level of specificity required without prescribing solutions.

## 3. Compare outcomes and competing explanations

For each material decision, keep one source-linked record: question; user outcome; evidence; current mechanism; strongest defense; credible alternative(s); lost/gained behavior; failure and success cases; dependencies; preferred option and uncertainty; result that would reverse the choice.

Use these dimensions together:

- **Expression:** can legitimate user/agent work still be represented, including new methods rather than only current examples?
- **Safety and privacy:** can forbidden effects or disclosure occur, including through alternate public paths?
- **Progress:** can permitted work complete, recover or obtain a meaningful decision without deadlock or endless approval?
- **Comprehension:** can a human explain and act on the process without internal implementation knowledge?
- **Maintenance:** where does a realistic future change land; how many owners or representations must stay synchronized?
- **Transition:** what happens to actual users, active work, supported stored data, current clients and external tools while adopting it?

A simpler diagram is not proof of simpler execution. A larger diff is not proof of a worse design. A higher test count is not proof that expectations are independent. Refusing an unauthorized operation is insufficient if the authorized version cannot work.

Do not invent a numeric “coherence score.” Use concrete differences and unknowns. Measure operator actions, repeated decisions, dependency spread, observed errors or effort only where the method makes those measurements meaningful.

## 4. Distinguish support from origin, approval and implementation state

Use the independent origin/standing/support fields in [whiteboard and decisions](whiteboard-and-decisions.md). A user aspiration can be partly implemented; an implementation can be complete but unjustified; a chosen design can still be untested. Do not classify those as mutually exclusive alternatives.

For support, distinguish source findings, observed results, documented statements, design reasoning and untested hypotheses. A source finding names current code/consumers and does not imply execution. An observed result needs exact source, command, environment and retained observation. A documented statement may be stale or agent-derived. A hypothesis may justify an investigation but cannot silently become a readiness premise. A planning choice means selected under stated conditions, not uniquely true or implemented.

For consequential contributions, make **finding → interpretation → recommendation → dependent consequence** visible. Clear facts do not need ritual hedging. Personal-derivation language is useful only if unresolved conditions affect the work allowed to rely on it. Never promote a preference because several agents repeat it, and never manufacture opposition to satisfy the request for criticism.

Follow high-impact citations to their original grounds. Several documents copied from one inference are not independent support. Primary-source intake and the [early substantive trial](review-method-trial.md) test this before it becomes a shared mistake. Later reviewers recheck selected common assumptions, even when all specialists agree.

A present-contract violation is a defect. Conflicting meanings between owners or representations are a coherence problem. A legitimate operation that is inaccessible or misleading is an interaction/API problem. These classifications can overlap, but must not be substituted for one another.

Existing tests are clues, not the sole oracle. Write the expected behavior of important trials from user needs, effect facts and normative semantics **before** consulting the implementation's expected results. Test doubles may control external providers; they must not replace the scheduling, permission, revision, publication or recovery decision being examined.

## 5. Debate to discriminate, not to accumulate opinions

Use actual separate sessions where available. Cover these responsibilities; combine them when appropriate rather than requiring a fixed number of agents:

- product value and freedom of expression;
- current-design defense grounded in difficult working cases;
- representation and implementation alternatives;
- concurrency, data, revision and recovery semantics;
- federation, service authority, privacy and browser trust;
- real human operations, diagram semantics and accessibility;
- frontend engineering, migration and future maintenance.

The **value reviewer** must challenge both an underpowered redesign and an expensive feature with no demonstrated purpose. This is not the same job as defending the current code.

For a contested question:

1. Register one precise question on the existing whiteboard. Link evidence and affected outcomes/scenarios. Do not duplicate the topic's state outside the overview.
2. Obtain independent initial positions. Each includes a serious alternative and a countercase against its own preferred design.
3. Cross-examine the strongest disagreement. Each side must identify the factual or value premise on which it depends; repeat slogans do not count as replies.
4. When factual uncertainty matters, design the smallest safe experiment that distinguishes the alternatives. Specify competing predictions before running it. More experiments are allowed when new evidence warrants them.
5. Revise or defend the actual proposal. Record the best unresolved objection and whether it changes the recommendation.
6. Have a challenger examine the **revised** design and the implementation tasks that claim to resolve the issue. Rewriting a paragraph without fixing its dependencies does not close a finding.

Votes, shared confidence and number of reports do not decide. Strong evidence may justify keeping every examined construct; strong evidence may justify changing many. Neither pattern is automatically suspicious or automatically successful. Never manufacture dissent or require a quota of removals.

## 6. Select a planning decision, or keep its consequence visible

Use the explicit promotion and dependency-reopening steps in [whiteboard and decisions](whiteboard-and-decisions.md). Keep the current answer in one topic; working summaries cite its revision. The coordinator can select an adequately supported technical recommendation inside reviewed goals, not approve an unrequested outcome loss or new access to private resources. Retain the actual chooser, grounds, dissent and scope.

A hybrid of incompatible recommendations is a new candidate. Review the combined user path, owners, lifetimes and failure behavior before it inherits any approval. A clean target does not justify an unsafe transition; both need a complete account.

Allowed conclusions are: evidence supports retaining; evidence supports a specified change; a bounded experiment is still needed; a user tradeoff requires approval; or a question is irrelevant to the current dependency and parked with a real trigger.

A core unresolved decision cannot become both “deferred” and an assumed prerequisite in another document. Dependent tasks remain conditional until it is resolved. The coordinator may proceed with genuinely independent planning, but cannot certify the affected implementation ready.

A substantive new fact, counterexample, user decision or implementation result reopens relevant decisions. A repeated preference without new grounds does not trigger an endless review loop. Add depth where uncertainty has consequences; there is no arbitrary iteration cap and no reward for length.

## 7. Turn conclusions into durable improvement

For each consequential confirmed problem, identify how the error survived prior work and what would expose recurrence: a contract test, an end-to-end positive/negative scenario, one owner removed, a source-of-truth change, clearer operator feedback, or a narrowly improved practice. Do not automatically create another lint, abstraction or approval layer.

The final trace is:

**original grounds → reviewed outcome → product meaning → actual path and mismatch → competing explanation → discriminating evidence → planning decision → affected owners and consumers → adoption task → acceptance proof and real use.**

Check it in reverse as well. A task without a justified decision may be speculative. A decision without adoption and proof is merely prose. A deleted concept without an accounted-for outcome may be an accidental product reset. A chosen implementation without a legitimate upstream premise may merely reproduce the original conceptual drift. When a new observation changes that premise, revise its dependents rather than only the explanatory paragraph.
