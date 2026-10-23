# 04 — Method responsibility transferred to the canvas side sprint

**Superseded before implementation, 2026-10-23. Do not execute this as a separate phase.**
The user's consolidation request produced [ADR 0052](../../../decisions/0052-canvas-authoring-and-method-experience.md)
and the [canvas experience side sprint](../canvas-experience/README.md). Methods are a core part of
that complete experience, so their entire unimplemented responsibility transfers there. This is
neither completion nor a deletion of required behavior. The previous detailed prompt remains in Git.

| Transferred responsibility | Owning assignment |
| --- | --- |
| Saved authoring, direct boundaries and prospective shared admission | [Side S02](../canvas-experience/S02-saved-canvas-authoring.md) |
| Actual early-user prerequisite formerly required by 04 | [Side S03](../canvas-experience/S03-early-user-review.md), satisfying main 02 only with real evidence |
| Complete library/rights, visual method authoring/use, live activation, nested bounds, exact interrupted recovery, export release, local future edits and explicit cross-daemon import/export | [Side S04](../canvas-experience/S04-reusable-methods.md) |
| Actual method-reuse review, complete side gate and acceptance handoff | [Side S05](../canvas-experience/S05-final-review-and-gate.md) |

ADR 0051's selected local current-at-activation semantics remain, refined by ADR 0052. The main
[journey](journey.md#method-edit-and-interrupted-activation-side-s04) remains the combined scenario;
its local method segment is delivered by the side. No method-shaped mock interface or separate
partial backend phase substitutes for that responsibility.

After side acceptance, resume [main 05](05-remote-tools-and-retention.md) only when assigned. Main
06 and 07 retain actual integrated review and the later final gate. Do not rerun this old 04 or
mark its transferred work complete before the side handoff establishes coverage.
