# 02 — Early user review and repair

**Prepared, not assigned.** Prerequisite: prompt 01's real production slice and focused evidence.
This assignment requires actual user participation. It is a functional review and repair boundary,
not a design presentation or permission to replace working screens with a mock.

Read [AGENTS](../../../../AGENTS.md), canonical docs, [sprint README](README.md),
[ADR 0051](../../../decisions/0051-daemon-task-workflows.md), [journey](journey.md), the current
handoff and [cohesive review](../../practices/cohesive-review.md),
[implementation](../../practices/implementation.md), [documentation](../../practices/documentation.md)
and [workflow](../../workflow.md) practices. Confirm the reviewed source and actual startup procedure.

## Make the result reviewable

Run the built daemon/client in an isolated authorized development root, with the real configured
capability used in 01 or a clearly identified controlled endpoint. Give the user usable local URLs,
account setup and a short task brief with an input file. Do not make them know an internal workflow
ID, edit a database, reconstruct a request file or run source-only helpers to use the product.
A phone-sized browser demonstrates layout; claim physical phone use only when actually observed.
At least one reviewed result must come from the useful, input-dependent real model/agent or trusted
Rust tool operation required in 01. A controlled endpoint supplies fault/browser tests, not the sole
substantive content of this product-use checkpoint.

Ask the user to perform: connect to A; submit an animation-planning task with instructions separate
from its brief/input; inspect real output; close the client while another task runs; reconnect in a
fresh session and find both workflow and earlier result; switch to B and verify the difference;
resolve a missing input; inspect a failed task; request stop and understand whether it has stopped.
Observe whether the user can identify the owner, selection/permissions, next available action and
what persisted. Review intermediate results and authoring friction, not only appearance.

Record who actually reviewed, environment, brief, actions, concrete findings and limitations in the
single sprint handoff; retain redacted screenshots/logs under ignored evidence paths. Agent-only
browser testing cannot stand in for the user's review. If the user is unavailable, finish actionable
repairs and leave this review pending; do not mark it passed or proceed through the checkpoint.

## Repair the complete responsibility

Fix confusing or missing behavior at its owning daemon/protocol/client boundary. Examples include
poor discovery, missing capability choice, unreadable failed-input reason, stale result after
switching daemons or reliance on client-held state. Do not label a backend defect a future phase
merely because it is found during UI review. Keep the target task contract; a concrete user finding
may justify updating an engineering default with its reason and affected tests/docs.

Make coherent tested local commits during repair. Run affected owner tests and the frontend checks/
focused browser cases from 01. Add regressions for real failures; do not write tests that only assert
new wording. Rerun the repaired user action with the person when their finding concerned behavior or
understanding. Run documentation contracts and diff checks for updated instructions. No full gate is
required here; 07 owns it and CI remains unchanged.

Handoff the tested commit, actual user observations, repairs and any genuine unresolved decision.
Required unfixed usability/functionality keeps this checkpoint open. On success, update status and
verification evidence with the narrow observed result and stop before 03. Do not imply real model
quality, phone support or general usability from a controlled endpoint or a single observed session.
