# 08 — Plan complete adoption and review the execution program

**Owner:** coordinator, with a fresh-context review of the generated assignments. **Output:** user decision packet and unexecuted implementation program. **Prerequisite:** 01–07 with current goal/product/architecture decisions, credible evidence, reviewed dependencies and explicit blockers. **Stop:** user approval; no backend or frontend implementation.

Read the whole current packet, [preservation requirements](context-and-preservation.md), [review method](deliberation-protocol.md), [scenario coverage](scenario-corpus.md), active whiteboard decisions and [workflow](../../workflow.md). Apply the [implementation](../../practices/implementation.md) and [documentation](../../practices/documentation.md) practices to the proposed work.

## Audit the grounds and transition before consolidating

Use [whiteboard and decisions](whiteboard-and-decisions.md). Trace major choices back through product meanings and reviewed goals to original intent/evidence. Check that no required premise is superseded, unreviewed or silently inherited from a summary. A selected planning recommendation is not user authorization to remove an ability or weaken a protection.

Separate the **target design** from the **transition to it**. For each coherent implementation boundary describe the usable before/after state, which owners/producers/consumers must change together, which current records and pending work are affected, the removal of competing paths and a safe rollback or explicit refusal. A destination may be coherent while its rollout creates months of contradictory behavior; reject that rollout.

No priority ranking may turn a distinct core outcome into an indefinite “later.” Account for every retained outcome and proposed loss. Conversely, avoid inventing distant low-level details whose upstream choices remain unresolved: write the ready assignments fully and the exact blocked branches, assumptions and work needed to make them ready. The user brief must not claim the entire program is unconditionally executable when it is not.

## Select a coherent target, not an average of opinions

Recheck source drift since intake. Preserve accepted current behavior while proposing its replacements. A favored design must have a defensible outcome account, neighboring contracts, lifetimes, standards meaning and real interface path. Do not merge incompatible recommendations because both reviewers sounded persuasive.

Separate evidence-supported facts, judgments, unperformed hypotheses and user-approval decisions. State exactly which choices block which tasks. Do not call an unresolved core relationship “an implementation detail.” Conversely, do not block all work for a question whose consequences are genuinely isolated.

Propose canonical changes to vision, architecture, ADRs, status, roadmap and practices in the later adoption assignment. Vision should explain enduring purpose without quietly legislating every current implementation. Architecture should explain the selected ownership and semantics. Status must describe implemented evidence, not promote this plan to a claim that it already works.

## Consolidate the review without multiplying sources of truth

Produce `deliverables/` with the following content. Files may be combined where that improves ownership; coverage cannot disappear through consolidation.

| Content | Required substance |
| --- | --- |
| Decision brief | What Milkdrift is for; reviewed source-backed commitments; biggest retained strengths; consequential changes and alternatives; strongest objections; exact user choices; what becomes usable next. |
| Product model and value account | Outcome/concept/relationship map; preserved, improved, replaced and lost behavior; proposed terminology; explicit scope/approval decisions. |
| Execution and federation design | Who owns what, public operations, permissions, compound lifetimes, live adaptation, uncertainty and recovery; browser connection/security decision. |
| Notation profile | Normative references, supported semantics/shapes/grammar, divergence, accessible representation, agent-authoring evidence and round-trip constraints. |
| Production interface | Build-ready interactions, real data/action sources, per-owner states, graph/history/results, direct work, methods, resources and learning; brand/layout/accessibility. |
| Frontend practices | Proposed canonical Svelte guidance, state/transport/credential boundaries, checks, assets, deployment and future-client separation. |
| Remediation/adoption plan | Specific current owners/producers/consumers, replacements/deletions, versions/fixtures/examples, data/active-work treatment, safe transition and rollback. |
| Review and evidence | Primary intent/evidence references, actual review-method trial, traceable findings/decisions and revisions, dissent, experiments/results, evidence limitations, root-cause lessons and dependent decisions rechecked after changed assumptions. |

Retain enough explanation to survive later removal of temporary planning notes. Keep raw logs outside permanent docs but deliver inspectable sanitized evidence for critical claims. Do not replace source references with invented summaries or require the user to reconstruct the argument from hundreds of isolated messages.

## No arbitrary ceiling on the implementation program

Under `deliverables/proposed-implementation/`, write a program README and executable assignments grouped by coherent responsibility and dependency. It may span multiple sprints. There is no prescribed prompt count or word ceiling. The aim is complete, contextual work—not either eight rushed prompts or 128 copies of boilerplate.

Every chosen product outcome must map to a fully specified ready assignment or an explicit blocked/approval-required branch with its prerequisite investigation. Fully write the ready work, not just the next two attractive screens. Do not invent distant interface details that depend on unresolved decisions. Explain the whole route and complete those conditional assignments when their decisions are resolved; do not certify them ready in advance.

The first assignment adopts approved design/practice changes and reconciles current source. Implementation must then remedy actual causes, migrate all affected consumers, and remove superseded paths. This can be extensive; do not preserve wrong architecture merely to keep a small diff. It also must not force a blank-slate replacement when adoption can preserve good work.

## What every implementation prompt must carry

Each prompt needs more than a task list:

- the reviewed user outcome and exact current goal/product/architecture decision revisions that justify it, distinguishing required premises from background references;
- the source/context baseline and changed premises it depends on;
- the surrounding upstream/downstream contracts, state and lifetimes;
- existing strengths to preserve and behavior deliberately replaced;
- concrete owner-level work, all consumers to adopt it, and obsolete code/config/tests to remove;
- normal behavior, meaningful allowed variation and failure/recovery cases;
- public daemon and actual Svelte interactions where relevant, with owner-qualified identity and permissions;
- compatibility/migration treatment for currently supported data, exact requests, active/uncertain work and current clients;
- independent acceptance expectations, focused check commands/targets, evidence location and honest limits;
- regular coherent commit checkpoints, handoff dependencies, what reopens the decision, and how implementers report a disproved premise before dependent work continues;
- clear completion/stop conditions and exclusions arising from authorization, not arbitrary convenience.

Supply the context once in a maintained shared packet, then attach the relevant neighborhood to each assignment. Do not force every implementer to infer why a local change matters from the entire archive. Do not fragment one shared contract and its callers into separately “complete” incompatible phases.

## Required implementation ordering

1. Resolve/adopt approved product, ownership, notation and public-contract decisions. Fix unsafe or semantically inadequate prerequisites before presenting the affected action as usable.
2. Establish the actual supported browser-to-daemon path, per-connection identity/authority, frontend practices and build/checking through retained production code. No mocked login or hardcoded catalogue qualifies.
3. Deliver the first real human workflow from connection through authoring/execution/observation/result on actual daemons. Keep it maintainable and extend it rather than replacing a throwaway prototype.
4. Obtain the user's visual/interaction critique on that running path and assign its necessary design corrections. This is also a product/architecture reconsideration point, not merely visual polish. Link any changed premise to affected upstream decisions and downstream work. Do not predict positive user approval or call agent walkthroughs human testing.
5. Complete the reviewed advanced outcomes and their **combinations**: reusable/published calls, multiple owners, direct/remote work, controlled adaptation, resource/tool improvement, knowledge/evaluated reuse and recovery as selected. A sequence of individually working screens is not integrated acceptance.
6. Use a further real compound-work review checkpoint, including S26/S28 or a justified equally demanding case, before declaring that the proposed product model works.
7. Run the named final full-system test-and-repair assignment, then close with accurate current docs and unresolved/approved follow-up work.

Sequence can be reorganized where dependency evidence demands it; explain the changed path. A frontend can reveal a necessary backend redesign, but it must not implement that semantic redesign privately in TypeScript. A backend cleanup may enable the interface, but it must not indefinitely postpone human use for unrelated perfection.

## Testing, commits and adoption

Each implementation boundary carries focused behavior tests and necessary compile/static/docs checks. Commit coherent changes before starting unrelated work; preserve checkpoints and other contributors. No automatic squash, rebase, hard reset or push. Known defects are fixed where found, not knowingly handed to the last prompt.

Name the final acceptance owner for complete Rust strict/workspace/application checks, frontend lint/type/build/unit/contract/browser checks, applicable accessibility/visual checks and real multi-daemon/authority/failure journeys. Avoid repeating the whole-system run per prompt. After final fixes, verify the integrated changed executable state; retain evidence from unchanged lanes only when the repository policy permits it. Do not weaken CI or assertions to shorten the process.

Handle prerelease compatibility deliberately. Do not support every obsolete historical encoding indefinitely. Do preserve the meaning and explicit treatment of currently supported definitions, stored results and accepted work. Schema changes need a concrete migration, drain/refusal or preservation decision, not “we will handle migration.” Rollback must account for any data written by the new version.

Review adoption by searching for old competing owners, helpers, representations, APIs, fixtures and documentation. A renamed façade over the old contradiction is not remediation. A second parallel implementation left for convenience is not completion.

## Review the generated prompts themselves

Give the program and factual packet to a reader who did not author them. Ask them to trace a demanding outcome from first user action to final accepted result, including a wrong permission, unavailable owner, changed future plan and retained uncertain work. They must identify which assignment produces every prerequisite and how it is tested.

Perform a forward and reverse audit: every reviewed outcome has a path; every major decision has adoption work; every code change has a user need or invariant; every test checks an independently meaningful expectation; every proposed deletion has a loss-and-gain account. Trace key claims to primary sources, not a circular set of summaries. Check required predecessors and usable interim states, not only the eventual architecture. In a separately labeled hypothetical, vary one consequential premise and ask the fresh reader which assignments would need reconsideration; do not alter real approvals based on a fictional premise. Check loops/dependencies, contradictory terminology, old conclusions surviving in later prompts, unavailable public APIs and “later” placeholders that hide missing value.

A required fact available only in the author's memory is a prompt defect. Fix the packet or assignment and have the affected path reread. Do not run another broad committee round when a focused correction resolves the uncertainty. Record the independence limitation if only one session performed the review.

## Final readiness and stop

The user brief must distinguish: verified current strengths; chosen improvements; actual losses or newly added obligations; preserved dissent; what still requires approval/proof; and the executable path to a real interface. No success claim is based on prose volume, unanimous agents or a prototype.

Commit checked planning/whiteboard work, validate references and deliver the proposal/evidence/program. Do not register the proposed implementation as active, rewrite production semantics, delete active review context or start code execution. User approval is the boundary between this plan and implementation.

**Completion means a defensible path from the real existing product to a stronger coherent product, with the difficult abilities retained or explicitly decided—not a promise that every future bug has been prevented.**
