# Context to preserve — strengthen the product, not just the instructions

This is an **assistant-derived working synthesis**, not a newly accepted product specification or an independent source of user intent. Read the [user-source excerpts](intent-source-excerpts.md) and accessible original context before adopting its interpretations. Primary-source intake reviewers form their first account before the coordinator's preferred synthesis. Recheck implementation claims against the locked source in 01; reconcile goals explicitly in 02.

## What the user is trying to recover

Milkdrift has accumulated months of substantive implementation and evolving ideas. The goal is not to defend that code, and not to discard it because a blank-slate system is easier to describe. The goal is to understand the valuable relationships, correct the incoherent ones, and finally expose a usable human interface.

The user wants agents to develop methods of work, not merely execute a hardcoded prompt chain. A method can respond to what happens, be inspected and revised, and become reusable for different inputs or product variations. Tools, services and workspaces help the agent do real work; other machines can provide capabilities or own their own workflows. Reliable uses still need enforceable requirements, permissions and evidence. Exploration and reliability must coexist without being forced into two unrelated products.

A useful shorthand for review is:

> People and agents should be able to develop, run, inspect, change and reuse complex methods of work across independently controlled tools and machines, without losing control of what is allowed or knowledge of what happened.

This is a **working synthesis**, not proof of demand for every mechanism. Challenge how well the existing and proposed designs serve it. Do not turn it into a slogan that replaces actual scenarios.

## Outcome obligations, not a frozen list of types

Use the following seed outcomes to construct the preservation map. Add missing outcomes from source, history and user intent. Their labels do not prescribe pages, nodes, crates or classes.

| ID | Ability or value to account for | A false simplification to test |
| --- | --- | --- |
| V01 | A human can understand and operate the work without assembling internal JSON or learning the database. | Beautiful screens that still require hidden CLI steps or private helpers. |
| V02 | A goal, human plan or agent proposal can become an explicit executable method, with visible choices and consequences. | Only hand-authored fixed prompt sequences remain expressible. |
| V03 | Running work can develop through authorized investigation, repair and new future work while retaining what already happened. | A linear runner with a retry button replaces genuine adaptation. |
| V04 | Reuse preserves a method's meaning across different inputs; a collapsed view can expand where permitted. | Copying text/commands substitutes for reusable workflow behavior. |
| V05 | A workflow can be offered as a constrained callable service without granting callers its internal privileges. | Calling a method requires full administrative permission, or silently inherits it. |
| V06 | Independent workflow owners and execution hosts can cooperate, while the standalone client can manage several connections. | One implicit local daemon, shared credentials or a published method used as the only network bridge. |
| V07 | An authorized agent can improve its tools and working environment, and useful services/data can outlive an operation. | Removing managed work leaves the operator to manually recreate every environment. |
| V08 | Direct useful operations do not require fabricated workflows; native tools and attached external inference remain meaningful choices. | Every operation becomes a forced wrapper workflow or container. |
| V09 | Parallelism, waiting, dependency decisions and result combination have reliable meaning under failure and change. | Arbitrary text promises replace scheduling, or extra boxes exist solely to satisfy an engine restriction. |
| V10 | Evidence and selected knowledge can support evaluated improvements and independent product variations. | A successful demo is called learning, or learning is deleted because its first proposal did not improve a result. |
| V11 | Authorization, privacy, limits, uncertainty and recovery constrain real effects without preventing legitimate progress. | Rejecting everything makes tests pass; a disconnect invites unsafe repeated work. |
| V12 | Results, definitions, histories and owned resources have understandable lifetimes and can remain useful outside one UI session. | Closing the client kills work, or release from management is confused with destroying the result. |

For each row, record **origin**, **present standing**, and **support** independently under [whiteboard and decisions](whiteboard-and-decisions.md). Add the factual implementation state with source/evidence: supported in a stated scope, partial, absent, or unexamined. These are not competing labels: a still-desired outcome may be partly implemented; an implemented capability may rest on an obsolete assumption. An aspiration is not an implementation pass, and implementation does not establish value. V12's operator handoff details, for example, must be established rather than inferred from container persistence.

The seed rows are deliberately broad. 02 must decide their intended scope from original goals and evidence, not simply ratify this table. It must neither freeze every accumulated wish as mandatory nor remove a meaningful ability without a replacement/value account and any required user decision. When a seed interpretation changes, preserve the old reference, record the new grounds, and recheck affected scenarios and design recommendations.

Preserving an outcome may require replacing its representation or implementing a missing public operation. Do not automatically preserve the old shape. Do not silently drop the outcome when that replacement is difficult.

## Required loss-and-gain account

For every material redesign, state: existing useful behavior; proposed behavior; what becomes easier or newly possible; what becomes harder, narrower or unavailable; which safeguards/lifetimes change; affected dependencies; migration effort; and evidence for the comparison.

There are three grounds for proposing a feature's retirement: its useful behavior is provided elsewhere with a complete path; evidence and reviewed goals show the claimed value is absent or better met differently; or an explicit product tradeoff is proposed for user approval. The first two can justify a technical recommendation, not unilateral removal of an explicit user commitment. Any meaningful outcome loss or weaker protection remains conditional until approved. A vague `later`, `out of MVP`, `not essential`, or `too complex` is not another ground.

Deferral is a real loss of near-term availability. It needs a named outcome, reason, dependency, next assignment/trigger and user-visible consequence. A future task is not implemented behavior. An implementation program cannot be called complete while important outcomes quietly fall into an indefinite backlog.

This is **not** a preservation veto. A large rewrite may be the right route. A local change may be insufficient. Require the larger proposal to explain its complete target, transition and retained value; require the small proposal to explain why it fixes the actual cause rather than hiding it.

## The archived vocabulary is evidence, not the destination

The baseline has a generic capability-driven task and separately typed control constructs. Its blueprint is a graph/definition, not simply one text node. See [node definitions](../../../../crates/blueprint/src/model/node.rs) and [graph definitions](../../../../crates/blueprint/src/model/graph.rs).

Its architecture distinguishes a pinned subworkflow, a workflow published as a callable service, and remote delegated execution. The [independent-host decision](../../../decisions/0038-independent-host-execution.md) does not require method publication for ordinary remote execution. The [publication decision](../../../decisions/0041-published-method-invocation.md) adds service authority and a recoverable internal run. Trace current implementations before relying on those descriptions.

These distinctions may remain, combine differently, or disappear through a justified replacement. Never make the user defend a false limitation invented by the review. In particular, **reusable**, **published**, **remote**, **collapsed**, **adaptive** and **authorized** are not synonyms, and they need not be separate product object types either.

The earlier discussion itself contains hypotheses to reconsider: “method workflows must be first-class objects,” “a small generic node core is best,” “the GUI must use Canvas/Timeline/Inspector,” and “the engine must retain all existing node types.” None is an additional binding decision from this package.

## Preserve distinctions in evidence, not accidental complexity

Separate purpose from mechanism: “do not repeat a possibly executed deployment” is an outcome requirement; a particular receipt table is one mechanism. “Change the future without falsifying the past” is a requirement; a particular revision enum or reconciliation layout is reviewable.

A node can look generic while its operation carries validated inputs and effects. A compositor can be one human action while waiting and combining results remain distinct responsibilities internally. Conversely, hiding two distinct effects under one name can be unsafe. Trace what changes, rather than treating either uniformity or explicit typing as intrinsically good.

Svelte is a standalone client, not a new workflow owner. It may manage selection, forms, unsaved drafts, layouts, connection state and authorized cached views. It must not decide execution permission, invent successful effects, or reconstruct private execution meaning to compensate for an inadequate public interface.

UML Activity and BPMN principles are the requested foundation for deliberate analysis and notation. They are not a promise that arbitrary model-generated diagrams are valid, that full compliance is necessary, or that their complete metamodels must replace the working runtime. Evidence must decide the supported profile and any extensions.

## Diagnose how drift happened

For consequential confirmed incoherence, investigate a small relevant history: the original need, the decision made, what later changed, which assumptions survived unchanged, and why existing checks did not expose the mismatch. Possible causes include missing neighboring context, a changed goal, an example promoted into a universal rule, overlapping ownership, or a test that restates its implementation. These are hypotheses to distinguish, not accusations to repeat.

The repair must address the surviving cause. Renaming a type cannot fix competing lifecycle owners. Adding another policy object cannot fix the absence of a usable operator path. More reviews do not help if every reviewer inherits the same unexamined assumption.

Record lessons as narrow proposed practice/test changes tied to actual evidence. Do not respond to every issue with another global rule, subsystem, approval step or mandatory document.

## What cannot substitute for a result

Do not claim product coherence from word count, the number of agents, all-green tests, standard-looking diagrams or a reduction in lines of code. Do not claim incoherence solely because one screen is awkward. Distinguish an unsound product rule, a correct but badly exposed operation, a missing API, a weak model result and an implementation defect.

The corrective target is **a stronger version of Milkdrift whose important abilities work together**. This planning process is itself a representative Milkdrift workload: maintain context, challenge decisions, reconcile revisions, retain dissent, and reuse a better method. Examine that fit without requiring Milkdrift to orchestrate its own review before it has been shown capable of doing so.
