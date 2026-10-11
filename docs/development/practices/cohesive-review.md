# Cohesive product and implementation review

Use this practice when a project's purpose is changing, important parts no longer fit together,
or a substantial implementation sprint needs a better-founded plan. It applies to a new idea as well
as an existing codebase. Its output may be a retained design, a reconstruction, a smaller product,
or a decision to stop and redefine the idea. Producing an implementation program is not compulsory.

The method is portable. In another project, identify its actual product owner, authoritative records,
source/evidence locations and authorization boundary before starting. Do not import this repository's
language, architecture, safety invariants, folder hierarchy or deployment assumptions as universal
requirements. Here, use the [practice selector](README.md), [workflow](../workflow.md) and existing
[whiteboard](../virtual-office/whiteboard/README.md); they retain their own responsibilities.

## Start with a falsifiable account of the product

Write the shortest concrete account that lets the owner recognize the intended product: who uses it,
what starts their work, what they do, what result matters, and what it deliberately leaves to other
people or systems. Use one ordinary example and one demanding variation. Explain necessary terms
through those actions before choosing nouns, modules or schemas.

Separate the desired outcome from a suggested mechanism and from an explanation invented by a
reviewer. For consequential premises, retain the original source, current standing and supporting
evidence separately. An approved behavior is not approval of one architecture. A tentative user
sketch is not a verified mechanism. Existing code is evidence of an implementation, not proof of need.

Challenge the account early. What real situation would make it useless, contradictory or needlessly
burdensome? Can an ordinary existing tool, a narrower feature or a changed procedure solve the need?
What would make the owner prefer that alternative? Distinguish conceptual plausibility from actual
user demand and observed usefulness. If those have not been tested, say so.

**Stop dependent design when the owner does not recognize the account, central terms have incompatible
meanings, or a required outcome lacks an agreed meaning.** Preserve the competing interpretations and
the smallest decision or observation needed next. Do useful independent work, but do not bury the
missing product decision under a complete architecture. Restarting from a clearer premise is a valid
planning result, not an embarrassment to conceal with more scope.

## Use a planning sprint to earn an implementation sprint

For substantial uncertainty, assign a finite investigation before assigning implementation. Its scope
is to decide what should be built, compare how, and produce complete implementation assignments only
where the premises hold. Set the authorized observations, effect/spending limits, desired checkpoints
and stop boundary. Review effort should follow consequential uncertainty, not a target number of
prompts, reviewers or documents. Do not recursively create a planning sprint for every minor step.

Use this sequence, revisiting only dependencies invalidated by new evidence:

1. **Recover intent and inspect reality.** Read primary user accounts, actual source and consumers,
   tests and relevant history. For a new project, inspect analogous real workflows and available
   tools instead of inventing a codebase. Mark inspected behavior, mapped-only areas, inherited claims
   and unknown relationships honestly. Reuse the existing record rather than creating an inventory
   that looks like an audit but contains no comparison.
2. **Try one consequential question early.** Work a real disputed case through the review method
   before distributing a preferred product narrative. Let it expose a wrong premise or a missing
   stakeholder. Do not wait for every subsystem report to discover that the product account is wrong.
3. **Compare complete alternatives.** Use the same inputs, independently stated expected outcomes,
   permissions and failures for the current approach, a smaller complete correction and a materially
   different alternative where it could change the choice. Do not invent three designs for every
   component. Explain the strongest case against the preferred option and what would reverse it.
4. **Trace interactions and consequences.** Follow a meaningful operation across its real boundaries:
   producer promises, consumer assumptions, state ownership, authority, interruption, recovery and
   inspection. Try a realistic future requirement and an adoption case. Count human decisions and
   maintenance obligations, not just files or arrows. A refusal is not sufficient if permitted work
   cannot complete. Preserving playback while removing editing is a capability loss, not transparent
   compatibility.
5. **Challenge, revise and reread.** Exchange concrete objections, run warranted discriminating
   observations, change the actual proposal, then have a challenger reread the changed decision and
   affected implementation assignments. Record supported dissent even when a choice is made.
6. **Decide or reframe.** Deliver the chosen outcome, evidence, costs, unresolved value decisions and
   complete adoption/implementation route; or explain why product reconstruction is needed. A map,
   investigation plan or list of future audits is not the completed investigation.

This is a loop with a stopping rule. If successive rounds repeat the same claims without changing
evidence, predictions, alternatives or decisions, stop and identify the missing premise. Continue
when a specific unresolved fact can change the outcome; expand scope only for a demonstrated
dependency within authorization. Do not use a time or word limit to label unfinished work complete.

## Make evidence discriminate between explanations

State what an observation could establish and what competing results would mean before running it.
Choose the smallest authorized exercise that answers that question. Use actual owners for the rules
under examination; controlled dependencies may supply inputs but must not supply the behavior the
product is supposed to provide. A script that manually orchestrates a workflow does not demonstrate
that the product owns that orchestration.

Inspect test expectations as well as assertions. A test may preserve an accidental restriction.
Establish desired behavior from the product account and relevant real constraints before using the
implementation's answer as the oracle.

Distinguish source findings, executed observations, inherited documentation, modeled alternatives
and unperformed experiments. Retain negative and inconclusive results, the source/environment of an
observation, limitations and counterevidence. Follow major citations to their original basis: several
reports repeating one inference are one line of support.

For consequential human judgments, retain enough of the reviewed material, criteria, reviewer
relationship, reasoning and limits to establish what the judgment supports. Predeclared comparisons
and retrospective judgments have different bases. Preference in a finite case does not establish
universal superiority. Reviewer agreement does not establish independent corroboration. Evidence,
adoption and permission to act remain separate.

## Organize criticism without manufacturing a committee

Use actual separate reviewers when independent work can improve the result. Give them primary sources,
constraints and neighboring facts before the preferred synthesis where practical. Have initial positions
recorded before mutual influence, then let reviewers engage with the strongest opposing case. Record
authorship and prior exposure; a fresh context is not automatically independent evidence. With one
reviewer, describe sequential passes honestly. Do not simulate votes, personalities or consensus.

Give a reviewer a concrete responsibility: challenge product value, defend a difficult working case,
develop an alternative, trace a failure boundary, or reconstruct the whole combined experience. A
review of documents agreeing with one another is not a review of their shared premise. In a separate
pass, challenge a consequential assumption shared by the current proposals; use another reviewer
where available.

An objection should name the situation, affected outcome, source or counterexample, and consequence
for the recommendation. Resolve factual disputes through evidence where possible. Bring consequential
value choices to the owner with alternatives, costs and a recommendation; do not repeatedly request
approval for already authorized internal work. Do not manufacture disagreement or a deletion quota.

## Keep one current account and reopen its dependencies

Use the project's existing discussion space. Organize questions by purpose, user-visible behavior,
system ownership and implementation mechanism when useful; these are reasoning layers, not four
mandatory documents per feature. Keep one current answer and preserve dated substantive contributions.

A compact decision record needs: the question and originating need; actual evidence and limits;
credible alternatives; selection and reason; strongest unresolved objection; affected dependencies;
and a condition that would reopen it. Distinguish proposed, selected, approved, implemented and
verified. Do not turn every sentence into a tracking row.

When a premise changes, update the current decision, its downstream proposal and its assignments.
Keep superseded evidence with a clear historical label; a stale ready-to-run prompt must not remain
the apparent current instruction. Recheck only affected dependencies. Do not rewrite history to make
an earlier recommendation appear consistent with the new one.

Writing is a tool for locating disagreement, not evidence of correctness. Prefer removing a derivative
summary over adding another. A reader should be able to find the current product account, why it is
believed, what could be wrong and the next decision without reconstructing a long chat.

## Hand over work that can actually be executed

When implementation is justified, each assignment owns a complete user or system responsibility:
implementation, actual producers/consumers, refusal and recovery paths, supported-data treatment,
removal of superseded paths, documentation and relevant verification. Name prerequisites and a usable
result. Foundational questions such as whether the feature belongs in the product or who owns it
must not become “investigate whether this exists” tasks for implementers.

Plan usable intermediate states, real human use where relevant, and combined-system review after
the parts work together. Earlier acceptance does not make a design immune to later evidence. Tests
must cover the promised operation and meaningful counterexamples; a final gate cannot excuse known
broken intermediate obligations. Use the project's actual verification policy rather than importing
another project's commands or demanding a full suite for prose-only work.

For a replacement, establish real users, data, active work, integrations and effects that need a
transition. Preserve only justified obligations; do not invent a migration engine for nonexistent
users. Archiving code preserves knowledge, not live-state continuity. Compatibility losses require
an explicit decision rather than being hidden in a converter's refusal.

Commit coherent checked progress when authorized. The handoff names what was actually investigated,
what remains uncertain, current decisions and their evidence, actual checks and limits, and the exact
next action. If unfinished, identify the continuation point without a completion claim. If the
product thesis is unresolved, say so and stop dependent implementation. Finishing a planning sprint
does not authorize implementation, deployment, spending or a successor sprint.

Where another reader is available, ask them to reconstruct a demanding path and its prerequisites
from the proposed assignments alone. Put necessary context known only to the author into the
handoff; label a sole-author check honestly.

## Portable kickoff prompt

Replace the bracketed fields with the new project's facts. Scale the work to the uncertainty.

> Conduct a planning sprint for [product/problem and owner]. The intended result is [user outcome],
> with [known constraints and explicit exclusions]. You may [authorized inspections, experiments,
> documentation and commits]; stop before [implementation or consequential action boundary].
>
> Begin from the user's account and one ordinary plus one demanding example. Distinguish desired
> outcomes, proposed mechanisms, inherited assumptions and unknowns. Treat existing code, tests and
> vision as evidence to investigate. Do not assume retention or replacement is the right answer.
>
> Use the existing records at [locations]. Audit actual review depth, then perform the consequential
> missing investigations. Test the method early on a real dispute. Develop credible alternatives on
> the same case, inspect source/consumers/history where relevant, and trace the human experience,
> cross-system obligations, failures, future changes and adoption consequences.
>
> Use independent initial criticism where practical, disclose exposure, preserve dissent and
> negative evidence, and reread the revised proposal. Repeated documentation or agreement is not
> validation. Reopen dependent conclusions when a premise changes. Ask me for consequential product
> choices with alternatives and a recommendation; continue independent authorized work.
>
> If the product account fails, preserve what was learned and return the concrete redefinition
> needed. Otherwise deliver a grounded proposal and complete implementation sprint with usable
> checkpoints, real-use review and appropriate verification. Do not defer foundational design to
> implementers. Keep checked progress and an honest handoff; stop at the stated review boundary.
