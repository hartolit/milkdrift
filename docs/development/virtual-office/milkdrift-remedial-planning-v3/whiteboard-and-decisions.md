# Whiteboard organization and how an idea becomes a decision

This is a scoped procedure for the remedial planning sprint. Use it with the existing [whiteboard procedure](../whiteboard/README.md), [office procedure](../README.md) and [deliberation protocol](deliberation-protocol.md). Do not build a separate knowledge-management application, registry or generated bureaucracy.

## 1. Organize by the question being decided

During 00, the coordinator creates these discussion folders with short navigation READMEs where absent. Do not overwrite existing guidance or move unrelated topics just to obtain a tidy tree.

```text
whiteboard/
├── README.md                    # topic state, next action, assignment, last evaluation
├── discussions/
│   ├── README.md                # navigation and common procedure
│   ├── vision/                  # outcomes, purpose, acceptable tradeoffs
│   ├── product-model/           # user/agent actions and what they mean
│   ├── architecture/            # owners, contracts, systems and cooperation
│   └── implementation/          # alternative mechanisms for an owned responsibility
└── issues/                      # concrete suspected or demonstrated defects
```

The layer is not the assigned profession. One topic may need product, runtime, security and UI reviewers. Choose the home of the main decision and link its dependencies rather than duplicate its answer in every folder.

Examples of different questions: “Must a colleague be able to use a method without editing its internals?” concerns a product outcome; “What does invoking this method promise?” concerns the product model; “Which owner accepts it and holds authority?” concerns architecture; “Which transaction records that acceptance?” concerns implementation. These are illustrative questions, not preselected answers or four mandatory documents for one feature.

Existing implementation defects stay in their issue file; a larger design alternative can have a linked discussion. Existing topics may remain at their old paths until an actual investigation justifies moving them. On a move, preserve contributions and fix incoming links atomically. Do not keep two active copies or silently assign work to parked hardware topics.

## 2. Separate origin, standing and support

For each **consequential** statement, establish these independently. Do not annotate every ordinary sentence.

| Dimension | What to record |
| --- | --- |
| Origin | User's exact instruction; tentative user exploration; agent-derived interpretation; implementation observation; external primary source. Include a retrievable source and context. |
| Standing | Explicit current requirement; candidate; disputed; selected for this planning baseline; user-approved decision; superseded. Name who made the decision, its scope and revision. |
| Support | Source trace; observed result with source/environment; design reasoning; untested hypothesis. Include contrary evidence, conditions and the strength/limits of the claim. |

Implementation state is another factual observation: absent, partly supported, supported under stated conditions, or not inspected. It is not a vote on desirability. A requested ability can be only partly implemented. A fully implemented feature can still be based on an obsolete need.

An accepted ADR is a current repository commitment to respect during this planning operation and a design choice the review may challenge. Its acceptance is not proof of universal optimality. A proposed replacement does not become a current product fact because the coordinator selected it for the plan.

Do not treat “latest wins” as a mechanical rule for intent. Read whether the later statement actually supersedes a prior commitment, narrows its scope, offers a possible mechanism, or questions an assumption. When the user's preference cannot be established, retain alternatives and ask for the exact consequential choice in the final brief.

## 3. Give conclusions consequences, not decorative uncertainty

A substantive contribution identifies its actual session author and makes clear:

**Finding:** what the source or observation establishes.

**Interpretation:** what the contributor derives, including assumptions and the strongest countercase.

**Recommendation:** what they propose, what it changes, and what would reverse it.

**Dependence:** what other work may use this conclusion for now, and what still needs evidence or a user decision.

This can be concise prose; no compulsory four-section form is needed for a minor correction. Do not hedge established facts into meaninglessness or copy “this is my opinion” into every sentence. The safeguard is that an untested premise is used conditionally, not that the writer sounds uncertain.

Example of the expected distinction, **not an actual decision**: “The reviewed source has separate reuse and published-service mechanisms. I recommend one call interaction because it may reduce repetitive authoring. Whether that interaction explains invoke-only remote access is unresolved; no editor assignment may assume private internals can expand until that case is resolved.”

User approval permits a tradeoff within its scope; it does not prove the mechanism will work. Conversely, a measured failure need not invalidate the underlying user goal.

## 4. Promote a recommendation explicitly

Use one current-decision section at the top of the relevant topic. Dated real contributions remain below it. Use a stable human-readable topic/decision reference and a commit or local revision marker so later documents can name the premise they used. No new database is needed.

Before marking a recommendation **selected for this planning baseline**, the coordinator verifies:

- its precise outcome and originating sources are recoverable;
- important observations and inferences are not mixed together;
- a credible alternative, contrary case and value/loss account have been examined;
- required dependencies are valid within scope, or explicitly conditional;
- factual disagreements are addressed by appropriate evidence and value disagreements are not disguised as facts;
- the combined choice has a complete user path, implementation consequences and a way to test its claims;
- an actual review of the revised proposal occurred, with independence limitations recorded.

The current-decision section records the selected option, reasons, rejected alternatives, best unresolved objection, dependencies, downstream effects, evidence limits and reopening trigger. “Selected” is not “implemented.” Silence and majority votes are not approvals.

The coordinator may choose between adequately grounded technical proposals within current goals. A meaningful outcome loss, weaker protection or change to an explicit instruction remains **user decision required**. No downstream implementation relying on that tradeoff is certified ready. Genuinely independent ready work may still be fully planned.

## 5. Follow citations back to their ground

Three documents repeating one agent's inference are one inference, not corroboration. When a load-bearing claim is supported only by later summaries, trace its actual origin and mark the missing primary evidence. Preserve independent observations separately from independent interpretations of the same observation.

A decision must distinguish **requires** from **informs**. A necessary requirement or factual premise can block dependent readiness; a related example or background link does not automatically do so. Reference the specific statement/decision revision, not only a long document's filename.

A short dependency line is enough: “requires the restricted-call outcome at topic X/revision Y; uses the observed transport constraint at source Z; informs notation and action design at topics P/Q.” If a required premise is unresolved, show the conditional branch. Avoid declaring circular dependencies “resolved” merely because the documents cite each other.

## 6. Reopen exactly what a changed premise affects

A new source fact, credible counterexample, altered goal or implementation result can change a decision. Repeating a preference without new grounds does not require another round.

When grounds change:

1. Amend the current assessment, preserving the earlier reason and contribution history.
2. Identify dependent product meanings, architecture, diagrams, APIs, frontend states, migration assumptions and assignments.
3. Mark their acceptance **needs rechecking** in the review-resolution record, with the source decision reference and reason. This is neither automatic rejection nor continued approval.
4. Re-review the affected relationships and update their decision references and working views.
5. Refresh the incoming context index and handoff. A completed phase label cannot override changed premises.

The overview still owns topic scheduling fields; the resolution record only owns impact/recheck results. Do not copy that administrative state into every document. A current summary is a derived view referencing decisions, not another independent specification.

Changing a hypothetical in the review-method trial does **not** reopen real decisions. Record it in the labeled exercise only. A real counterexample discovered by the exercise can be promoted through the normal evidence procedure.

## 7. Keep the process proportionate and recoverable

Write enough surrounding context to explain the choice to a new agent. Keep routine corrections near their owner; open a topic only for a consequential dispute or broader question. Do not require a form for every line of code or a dedicated agent for every layer.

Before a specialist decides, they should be able to explain the incoming premise, outgoing promise, governing authority, relevant lifetime and another operation that interacts with it. Public design explanations are required; private reasoning transcripts are not.

No invented pseudonyms standing in for independent sessions, no backdated observations and no manufactured dissent. No claim that disagreement itself proves quality. Retain a well-supported design when it wins its comparisons.

After user approval, promote lasting decisions to their existing canonical owners during the authorized adoption assignment. Consolidate without creating new independent claims; retain references and dissent needed to understand the tradeoff. Close/remove temporary material only through the office procedure after its useful context survives elsewhere.
