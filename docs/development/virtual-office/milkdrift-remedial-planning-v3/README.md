# Milkdrift remedial planning — v3

**Purpose: turn the valuable but inconsistent product we have into a stronger, coherent product and a real human interface.**

This sprint reviews intentions, actual behavior, competing designs and their consequences. It produces a reviewed proposal and a complete, **unexecuted** remediation and Svelte implementation program. It does not build a prototype or change production behavior. Implementation starts only after the user reviews and approves the result.

## What v3 changes

V2 already required serious alternatives, preservation of useful abilities and cross-system criticism. V3 adds a missing prerequisite: reconcile which goals still belong to Milkdrift before using them to justify architecture. It also distinguishes a contributor's observation, interpretation and recommendation, and controls when a recommendation can become a premise for other work.

The main chain is:

**source and intent → reviewed goals → product behavior → compared designs → reasoned decisions → adoption work → tests and real use.**

Check it in both directions. A task without a justified need is suspect. A valuable goal without an implementation path may have been discarded. A changed premise must reopen affected conclusions.

[Changes from v2 and coverage check](revision-review.md).

## Baseline and existing work

The supplied source archive identifies `c016cd3c33aea074c37a8b461047caaa9392a72d`. This is an inspected reference, **not a claim about current HEAD**. V2 additionally recorded `6252766` as a logo-only successor; treat that as inherited historical context and verify it before using it. Inspect the actual checkout, worktree and later changes in 00. Never reset newer work to match this package.

V3 supersedes planning instructions, not useful v1/v2 investigations. Reuse their evidence, dissent and decisions, then repair only affected dependencies. Extracting this package writes a new directory; it does not overwrite previous plans, the office overview, whiteboard topics or product documents. The supplied SVG is included unchanged as a reference asset; prefer the repository's approved asset after checking identity, without redesigning it here.

## Sequence and decision gates

| Prompt | Responsibility | Required result |
| --- | --- | --- |
| [00](00-first-execution-prompt.md) | Establish scope, source references and whiteboard organization | One active planning assignment, independent intake and a clear place for each kind of question. |
| [01](01-recover-intent-and-current-behavior.md) | Recover intentions and trace existing behavior | Primary-source intent and code/behavior evidence are distinct, with gaps and changed assumptions visible. |
| [02](02-vision-reconciliation-and-review-trial.md) | Reconcile the vision and test the review method | Reviewed working goals, explicit unresolved tradeoffs, and evidence that the review can detect a changed premise. |
| [03](03-product-model-and-alternatives.md) | Define product behavior and compare complete alternatives | Clear meanings for human/agent actions, without treating current names or mechanisms as requirements. |
| [04](04-architecture-federation-and-browser.md) | Examine architecture, ownership, distribution and browser feasibility | Viable owner/authority/lifetime and connection designs, with source-backed costs and gaps. |
| [05](05-standards-and-executable-notation.md) | Compare UML/BPMN foundations and exact mappings | A truthful proposed semantic/diagram profile, not compliance or AI-quality claims based on familiarity. |
| [06](06-production-frontend-design.md) | Specify the production Svelte interface and practices | Build-ready interactions connected to real public operations or explicit backend remedies. |
| [07](07-adversarial-debate-and-revision.md) | Challenge the combined proposal and its grounds | Material objections resolved or explicitly conditional, with affected dependencies rechecked. |
| [08](08-adoption-and-implementation-program.md) | Plan adoption and audit the implementation program | Context-rich executable assignments, safe transitions, real-user review and final integrated testing. |

These are coordinating entry points, not a fixed ceiling on investigation or implementation. Add work where unresolved consequences justify it. Do not multiply prompts for routine edits or repeat unchanged analysis. There is no target number of findings, deletions, agents or words.

**02 is a prerequisite for choosing dependent product/architecture designs, not a ban on learning from code first.** Existing behavior informs the goals. Later findings may reopen those goals. Unresolved product choices block only the decisions that rely on them.

**04–06 are a joint design loop, not a waterfall.** Begin from the reviewed product model. Develop the human actions, executable meanings and owning components together. No architecture is finally endorsed before its notation and real-interface consequences have been checked. Early browser investigation in 01 prevents late discovery of an impossible deployment assumption.

## Shared context without shared blindness

All participants eventually read [context and preservation](context-and-preservation.md), the relevant [user-source excerpts](intent-source-excerpts.md), [deliberation rules](deliberation-protocol.md), [whiteboard/decision rules](whiteboard-and-decisions.md), and the current source-backed context index. **Intake exception:** primary-source readers first receive original excerpts/history, explicit instructions and source references, not this package's outcome synthesis or the coordinator's preferred interpretation. They compare with the shared packet after recording their first account. Use [the 30 scenarios](scenario-corpus.md) and [worked traces](worked-coherence-traces.md) to inspect relationships, not to limit the product to these cases.

Primary-source intake reviewers write their interpretation before reading the coordinator's preferred synthesis. Alternative designers and later critics receive necessary surrounding facts, but form initial positions before seeing each other's recommendations. Record actual information exposure and authorship. Separate sessions reduce some shared assumptions; they do not prove independent evidence or guarantee correctness.

Each consequential contribution distinguishes **what was found, what is inferred, and what is recommended**. Do not add ritual disclaimers to every fact. A hypothesis can guide investigation but cannot silently become an implementation requirement. A planning choice records a selected option under stated conditions, not the one universally true design.

## Organization and authoritative records

Use the existing whiteboard. Organize relevant discussions under `vision/`, `product-model/`, `architecture/`, and `implementation/` as specified in [whiteboard and decisions](whiteboard-and-decisions.md). These classify decisions; they are not isolated teams. Concrete defects remain in `issues/`. One topic owns one current answer, with links to neighboring questions. Only the whiteboard overview owns topic state, assignment and next action.

Create working outputs when the corresponding investigation occurs, not empty paperwork for every possible feature:

- `working/context.md`: short incoming-session index of current facts, applicable decisions, assumptions and changed dependencies; not an independent authority.
- `working/intent-register.md`: sourced statements and interpretations, preserving origin separately from standing and evidence. No invented user history.
- `working/system-dossier.md`: outcome/concept/scenario coverage, current implementation traces, observations and gaps.
- `working/vision-baseline.md`: concise derived view of the reviewed goals, conflicts and conditional tradeoffs, linking the decisions that authorize each conclusion.
- `working/review-method-trial.md`: the early substantive case and context/dependency check from [the trial guide](review-method-trial.md).
- named design working documents from 03–06, and `working/review-resolution.md`: integrated proposal views, evidence and impact references, not duplicated whiteboard arguments.
- one `handoff.md`: progress, actual contributors, source/context revisions, checks and exact resume point.
- `target/remedial-planning-v3/`: raw inventories, private logs and isolated probes. Deliver sanitized evidence when a final claim depends on it.
- `deliverables/`: the reviewed proposal and unexecuted implementation program from 08.

A working summary cites the primary statement or decision and its revision. Repetition in vision, roadmap and several reviews is not multiple independent sources. User-facing final documents must be understandable without hunting through every comment, while retaining links to grounds and dissent.

## Scope and authority

Allowed now: inspect source/history; question current product and architecture commitments; edit planning/office/whiteboard records; draft canonical changes for later approval; exercise existing public paths in isolated test installations; run narrowly justified safe diagnostic probes.

Not authorized now: production code/schema changes; weakening existing permissions or checks; editing live credentials/deployments or private working data; paid-provider use without authorization; a mock/prototype frontend; executing the resulting implementation program. Browser-native probes may test platform behavior without becoming a UI or bypassing the daemon. Missing execution access is an evidence gap, not a successful experiment.

The latest user request authorizes this **planning investigation** even where the current roadmap excludes a successor implementation. Do not rewrite that exclusion into permission to implement. Questioning an invariant is allowed; operating outside existing safety/data protections is not. The coordinator may select technical recommendations for the proposal. Removing a meaningful ability, weakening a protection, or changing an explicit user commitment needs an identified user decision before dependent implementation is ready.

A meaningful boundary correction can be large. Prefer neither a small diff nor a blank-slate rewrite automatically. Preserve useful outcomes through a complete replacement, or make the loss explicit for approval. “MVP,” “later” and fewer lines are not sufficient reasons to shrink the product.

## Checks, commits and completion

Follow [AGENTS.md](../../../../AGENTS.md), [office procedure](../README.md), [whiteboard procedure](../whiteboard/README.md), [workflow](../../workflow.md), and the [implementation](../../practices/implementation.md)/[documentation](../../practices/documentation.md) practices. Planning edits use appropriate documentation checks and focused investigations, not repeated full-product gates. Coordinate shared edits and serialize Cargo jobs.

Commit coherent checked work regularly, stage explicit files and preserve prior checkpoints. No automatic squash, rebase, hard reset or push. Reuse unchanged evidence with its exact source/scope; do not report an old run as newly executed.

The later implementation program must have focused verification and regular working commits, early retained production Svelte use, a subsequent demanding combined-work review, and a named **final full-system test-and-repair assignment**. Known in-scope problems are fixed when found, not deferred to that final test. CI protections stay intact.

Stop for user review with the proposal, evidence, ready assignments and precisely blocked branches. Keep this planning context available through review; do not delete disputed topics or register implementation as active. After accepted promotion of lasting decisions, use the normal office cleanup procedure.

Completion establishes a defensible, inspectable route to a better product. It cannot establish that an unimplemented redesign works or that every conceptual defect has been found.
