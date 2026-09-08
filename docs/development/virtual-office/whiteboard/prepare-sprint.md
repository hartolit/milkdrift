# Prepare a sprint using the whiteboard

When asked to prepare a sprint or evaluate a topic, read [AGENTS.md](../../../../AGENTS.md) in
its required order and use the [whiteboard procedure](README.md). Choose an investigation that
answers a concrete question relevant to the requested work. Name its expected result and stop
condition; there is no quota of topics to review or create.

1. Read the overview and select relevant topics. If the board is empty or unrelated to the work,
   proceed with the requested planning.
2. Trace the selected question through current implementation, consumers, tests, and owning docs.
   Recheck old evidence; use existing focused checks or a small reproduction under ignored
   `target/` when useful. This review changes topic documentation and planning, not production code.
3. Add a dated contribution under your actual session pseudonym. Explain what the evidence
   supports, what remains uncertain, and the practical tradeoffs. Consider a smaller correction
   or keeping the existing design before proposing a larger replacement.
4. Update the topic's technical assessment. Put its state, next action, assignment link, and
   last-evaluation date only in the overview. Assigned execution progress belongs in the sprint/task.
5. Hand off what was learned and recommend a next action. A proposed assignment needs an outcome,
   responsibility, owner to assign, exclusions, checks, and stop condition. Keep unresolved context
   in the topic and link it from the plan.

Only mark work assigned when an actual sprint/task accepts it. Topics not investigated remain
available for later preparation. Stop at the requested review or planning result; use
[the office procedure](../README.md) for coordination and
[the verification policy](../../workflow.md#choose-verification-for-the-change) for checks.
