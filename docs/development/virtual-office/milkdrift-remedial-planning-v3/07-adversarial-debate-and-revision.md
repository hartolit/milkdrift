# 07 — Challenge the combined design and the premises that support it

**Owner:** review coordinator who is not presented as independent from work they authored. **Output:** revised decisions, explicit remaining uncertainty and evidence-linked resolution. **Prerequisite:** usable 01–06 evidence/design packet, including 02's actual trial and working-goal decisions. This is combined review, not the first opportunity to criticize. **Stop:** a reviewed proposal; no production implementation.

Read [the deliberation protocol](deliberation-protocol.md), [context and preservation](context-and-preservation.md), the whole current factual packet, alternatives, ownership/notation/interface designs, and relevant [implementation](../../practices/implementation.md)/[documentation](../../practices/documentation.md) practices. A narrow review lens adds scrutiny; it does not remove surrounding context.

## Check upstream legitimacy as well as downstream implementation

Recheck the original intent against the current working goals and product meanings before evaluating the combined technical design. Use [whiteboard and decisions](whiteboard-and-decisions.md); follow high-impact claims to original sources. Record whether several apparent citations are just repetitions of one inference. Do not treat the coordinator's context packet or selected baseline as an unquestionable source.

The [early review-method trial](review-method-trial.md) should have exposed context/procedure failures before broad adoption. Inspect whether its actual corrections were used in later decisions. Do not substitute a claimed trial pass for examining its output. If necessary independence or evidence was unavailable, carry that limitation into final readiness rather than silently certifying it now.

Challenge unaccounted goal losses and features preserved only through stale intent. A material user tradeoff is not settled by a standards expert, architecture vote or repeated confidence. A new combined/hybrid design must pass as a whole, even when each component once received approval.

## Independent first positions, then actual debate

Assign real independent sessions where possible. Cover product value, current-design defense, structural alternatives, execution/recovery, federation/security, standards/notation, human operation, frontend engineering and migration. One agent may cover related lenses, but do not claim that repeated passes by the same author are independent endorsements.

Initial reviewers see the same facts and candidate design, but not each other's judgments. Each records their strongest material counterexamples, a credible defense of the challenged choice, the relevant user outcome and source/design reference, and what evidence would decide it. No finding-count target, no synthetic critics, and no requirement that a well-supported choice be rejected.

Publish initial positions and cross-examine on the existing whiteboard. Pair disagreeing premises, not merely job titles: a simpler representation versus lost expression, a service boundary versus user friction, a recovery mechanism versus blocked progress, a familiar shape versus wrong execution, or a browser convenience versus cross-owner privacy.

Every substantive response must revise the actual contract/plan or defend it against the case. “We will be careful,” “documented,” “covered by tests,” and “addressed” without the changed decision and proof are insufficient.

## Attack the relationships, not just their endpoints

Review the full path and neighboring dependencies using the worked traces. Important questions include:

- Does a proposed generic operation still express waiting, context, structured concurrency, accepted evidence and side effects, or merely hide them in text?
- Does the simplified method model preserve easy reuse **and** constrained public service access, without making publication a mandatory network bridge?
- Can an authorized agent create and revise useful work without asking for unrelated global authority or manually rebuilding the plan?
- Can separate workflow owners, peers and client connections coexist without identity collisions or invented shared storage?
- Do client/server representations and standard notation have one meaning, including when a graph is collapsed, edited or revisited under another run?
- Are approved result, completed invocation, verified candidate and running service still distinguishable where their lifetimes diverge?
- Can resource editing, retained-use protection, cancellation, service retirement and nested work all make progress together?
- Can a discarded result, unauthorized artifact or changed candidate reenter through context, publication or recovery?
- Does the frontend actually reach every promised operation and explain uncertainty, rather than emulate missing behavior?
- Does the migration preserve valuable work and converge to one supported design, or keep permanent competing systems?

Test positive capabilities as aggressively as refusal paths. A system that forbids every risky-looking scenario can still fail the user's purpose. A system that handles the examples only by special-case constants can still fail the general method.

Use the compound scenarios and have reviewers add supported variations of their own. For properties such as identity scoping, layout invariance, replay, or evidence binding, state the exact intended scope. Do not mechanically apply a property that would be wrong for a different side-effect contract.

## Use evidence that can distinguish designs

For factual disputes, write competing predictions, a bounded isolated experiment and observable results before execution. Use actual production paths where they exist; preserve source identity and raw results. A manual semantic trace is useful but not an executed test. A standard schema validator is not an execution oracle. An agent's successful explanation is not a human usability study.

Where execution is unavailable, record the decisive missing proof and block only dependent readiness claims. Do not treat absence of evidence as either an automatic defect or a reason to delete a difficult feature. Do not settle value/preferences by pretending they are experimentally proven facts.

Check for common-mode reasoning: did all reviewers inherit a mistaken “fact” from the common packet? Assign selected high-impact claims to be checked from primary source or user intent rather than another agent's summary. A shared packet prevents isolation; it must not become an unquestionable source.

## Make criticism change the plan

Maintain one `working/review-resolution.md` indexing substantive findings and whiteboard decisions. For each record the affected outcome/scenario, strongest objection, author response, changed text/owner/task, supporting evidence, residual uncertainty and actual reviewer result. The whiteboard overview remains the owner of topic status.

After a decision changes, mark dependent approvals **needs rechecking** with the changed premise and source/decision revision, following the whiteboard procedure. Recheck affected notation, API assumptions, frontend states, migration and implementation sequencing. Do not leave an old interpretation in a later phase merely because that phase already has a “passed” handoff.

Require a fresh-context reader to explain a difficult path and the alternative rejected, using the packet and source links rather than the author's private knowledge. Any needed oral correction belongs in the maintained explanation. This is a context-quality check, not a demand to reproduce a predetermined answer.

## Review the review's incentives

Challenge both forms of evasion:

**Preservation theater:** accepted ADRs and green fixtures are used to avoid reconsidering an incoherent relationship.

**Simplification theater:** hard user outcomes are removed, pushed into manual setup, or postponed indefinitely so a small new design can win.

Neither keeping nor changing many concepts determines the verdict. Every loss of meaningful ability needs its account and approval; every extra mechanism needs a purpose. The plan must not spend months rebuilding infrastructure while never exposing real use, nor rush a cosmetic UI that conceals the unsolved model.

## Root-cause lesson and closure

For important confirmed issues, ensure the proposed remedy changes the underlying owner/assumption or adds a genuinely discriminating check. Record why the previous reasoning or tests failed to expose it. Make narrow practice proposals only where they address observed recurrence; more rules are not automatically more quality.

Unresolved high-impact decisions need a bounded next investigation or explicit user tradeoff. Unrelated questions can remain parked with a real trigger. Reopen only when new evidence or consequences warrant it, not to accumulate review rounds. There is no arbitrary time/word quota, and no claim that review can prove the absence of all defects.

Commit coherent revisions with documentation checks. **Gate:** the combined proposal has survived actual opposing cases, preserved valuable abilities, corrected dependencies and retained its strongest uncertainty. A serious unresolved foundation cannot be certified ready merely because planning documents are complete. Pass the revised record to 08. Do not call the unimplemented redesign verified, or proceed around an unresolved foundational premise by labeling it an implementation detail.
