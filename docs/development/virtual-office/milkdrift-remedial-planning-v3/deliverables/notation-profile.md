# The proposed workflow diagram language

The proposal uses activity-diagram conventions to make work, inputs, decisions, concurrent
branches and reusable calls understandable. Milkdrift's daemon remains the owner of what the
diagram means and what was accepted. This is a review-facing summary of
[N1](../../whiteboard/discussions/implementation/executable-notation.md); that topic owns selection,
dissent and later changes. This summary follows N1/r2 and detailed NP2, selected by the planning
coordinator after actual semantic criticism and focused rereads. It is not user approval for
implementation or a standards compliance claim.

An operator may start with a goal, a saved workflow, a structured outline or the diagram itself.
All routes must lead to the same daemon-validated definition. The proposal is grounded in the
official UML 2.5.1 and BPMN 2.0.2 specifications; exact inspected sections, PDF identities and
limitations are recorded in the [standards evidence](../working/standards-evidence.md). It selects
neither a UML execution engine nor general BPMN/XML interchange. Standards familiarity is a
reason to test the design, not proof that people or models will use it correctly.

## What the diagram says

| A person sees | They should be able to predict |
| --- | --- |
| An action with named input/output ports | Which capability operation runs, what it consumes and which result it promises. |
| An ordered decision with labeled alternatives | Which condition wins when several are true, and what the default does. |
| A parallel split and labeled completion rule | Which work is isolated; whether progress waits for all, any completion, first success or a quorum. |
| A result-composition action | Whether results are collected, selected in branch order, or judged by an external operation. |
| A time or signal wait | The durable evidence that resumes work and the signal's exact delivery scope. |
| A bounded repeat | Its pinned body, condition, limits and what happens when the limit is reached. |
| A collapsed call | Its exact owner/version and which inspection rights are available. Collapse never changes execution or grants access. |
| An explicit result/end | The scoped outcome, distinct from evidence that every external effect has stopped. |
| A pending edit beside accepted history | What is only proposed, what has been adopted, and which old facts remain unchanged. |

Control and typed-data connections have distinct meanings and accessible labels. The interface
does not use color alone to communicate them. Every graph action has a keyboard and outline/form
equivalent. Validation retains unfinished edits and explains the exact failing relationship.
Owner, version, permission and connection state stay visible where they affect the next action.
Choice/merge kind marks remain visible at every selectable scale; small overviews aggregate
regions explicitly. A compound join/composition card shows its separate statuses. Authorized
private-service inspection retains its public-invocation breadcrumb and clears on lost read rights.

## A real expression gap and its proposed remedy

The current production validator refuses two exclusive alternatives converging directly into one
continuation. A controlled Rust construction probe confirmed this refusal; it did not find a
stalled accepted workflow. [The case and reproducible source](../working/merge-construction-evidence.md)
show a successful separate-terminal control and the refused shared continuation.

The proposed remedy is one bounded choice region: exactly one selected route can continue, and
only its declared results flow into the shared next step. It stays in the same workflow and
authority scope. The candidate executable representation pairs a Branch with an ExclusiveMerge;
the implementation must compare that with a canonical structured-choice representation before
settling the public API. The behavioral obligations are fixed in the
[detailed profile](../working/notation-profile.md#owned-remedy-for-exclusive-convergence).
Data leaving an alternative for shared continuation must use those declared selected outputs,
so an unnoticed wire from an unselected route cannot strand the next task. Optional absence is
explicit. A compatible pending step within an already selected route remains repairable while
accepted earlier work and the declared result contract stay fixed.

A nested parallel loser can retain uncertain physical work even after the selected route has a
valid result. The merge permits ordinary progress without clearing that loser's resource/account
obligations. Its selected result, occurrence and continuation eligibility require one atomic
retained transition, preventing stale output or duplicate continuation after restart. These
requirements came from actual review; they remain future implementation proof obligations.

An existing alternative packages the choice as a pinned child workflow with separate terminals,
then continues in the parent. That is useful when a reusable child is wanted, but adds a separate
definition, execution, interface and context boundary. The frontend cannot silently introduce
that boundary merely to draw a merge. Until the selected remedy is implemented, the UI must show
the real refusal and cannot claim direct convergence works.

## The mistakes this proposal must prevent

“Any completion” cannot be presented as “first successful answer.” “First in branch order” cannot
mean “fastest result.” Cancelling unfinished work cannot mean its external effects have stopped.
A private service cannot imply an expandable internal graph or editable authority. Moving a box
between host regions cannot move accepted work or establish peer trust. A newer definition cannot
rewrite an earlier occurrence.

The [modeled transition cases](../working/notation-profile.md#proposed-transition-examples) make
those differences testable. They include invalid convergence from concurrent branches, selected-arm
failure, uncertain losing writers, scoped signals, repeat limits, permission changes and live repair.
A separate authoring/review exercise uses equal-information packets and held-out variants; it
remains planned because no paid model evaluation was authorized.

## What implementation must prove

The diagram and outline must reload the same accepted operation contracts, dependency meaning,
exact pins, bounds and outputs. The daemon owns every conversion with semantic consequences.
Unknown constructs cannot disappear on save. A picture export is not executable interchange;
any later import/export feature needs a finite supported mapping and explicit refusals.

The implementation program must complete the choice boundary through blueprint validation,
runtime/recovery, saved events, public API/CLI, Svelte, examples and positive/negative evidence.
Existing definitions, receipts, histories and active work retain exact meaning. Current production
behavior, proposed behavior and missing proof remain separate throughout adoption. This summary
adds no approval beyond the current [N1 decision](../../whiteboard/discussions/implementation/executable-notation.md).
