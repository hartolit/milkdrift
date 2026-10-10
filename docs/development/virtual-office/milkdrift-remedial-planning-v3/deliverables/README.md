# Milkdrift planning proposal for review

**Revised after the reopened investigation:** the user did not accept `ff818e5` as completion.
The [follow-up critique](../reopened-critique.md) led to the deeper comparisons and changed proposal
below. U17/U18 approve product directions and U19 requires preserving legitimate existing adaptability;
architecture and implementation remain for review. The conversion-gated graph-edit loss is withdrawn.
The revised proposal retains versioned graph authoring/repair in the same semantic owner and engine.

The reopened work audited 24 packages at stated depth, investigated source/relationships/history,
worked concrete competing designs, ran isolated priority and compatibility runtime diagnostics, and changed recommendations
through opposing reviews. These documents propose a target and implementation sequence with the
stated compatibility contract. They do not authorize or claim implementation.

Start with the [decision brief](decision-brief.md). It distinguishes verified strengths, selected
planning improvements, tradeoffs requiring your decision and remaining proof. The other documents
supply the complete contract for that review:

| Review concern | Document |
| --- | --- |
| Purpose, concepts, preserved value and alternatives | [Product model and value](product-model-and-value.md) |
| Ownership, independent machines, authority, lifetimes and browser route | [Execution and federation](execution-and-federation.md) |
| Standard foundations, precise execution meaning and accessible authoring | [Notation profile](notation-profile.md) |
| Real screens/actions and API sources | [Production interface](production-interface.md) |
| Maintained Svelte engineering and checks | [Frontend practice proposal](frontend-practice-proposal.md) |
| Owner-level remedies, compatibility, active work and rollback | [Adoption plan](adoption-plan.md) |
| Actual review, observations, corrections, dissent and limitations | [Review and evidence](review-and-evidence.md) |
| Complete unexecuted route, prerequisites and blocked branches | [Implementation program P00–P09](proposed-implementation/README.md) |

Whiteboard decisions remain the owners of selected recommendations and their rationale. This packet
consolidates their consequences; canonical product/architecture/status/roadmap and practices have
not been rewritten. The implementation program is **not assigned**. Human checkpoints P03/P08 and
new browser/runtime/upgrade acceptance are future work, not passes earned by these documents.
