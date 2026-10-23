# Making the canvas do the work

**Consolidation, 2026-10-23.** [ADR 0052](../../../../../decisions/0052-canvas-authoring-and-method-experience.md)
selects a bounded baseline from this exchange. The [side sprint](../../../canvas-experience/README.md)
retains actual-user comparisons as acceptance work; the arguments below explain why its choices
may need revision and do not remain competing implementation instructions.

This debate challenges the [canvas proposal](canvas-authoring.md) against the user's design
principles. The user's October 23 clarification asks reviewers to judge how the interface reduces
manual labor and creates a seamless experience. Putting raw data into forms leaves the person
doing the translation that a useful interface should perform. A technically valid design can still
fail that purpose.

The later [daemon/frontend debate](../architecture/daemon-frontend-contract.md) tests this same
standard against the runtime, persistence and adapter contracts. It records additional peer
challenges and pause diagnostics, including revisions to the earlier completion framing.

The [vision](../../../../../product/vision.md#the-client-makes-the-work-understandable) owns the
product intent. For this discussion, reviewers used these concrete tests:

- **Remove translation work.** A connection should turn a recognizable source and destination
  into the necessary bindings. The person should not encode the same decision again.
- **Preserve thought and place.** Placement, editing, connection and repair should keep the
  workflow, camera, unfinished ideas and relevant surrounding work available.
- **Reduce repeated decisions.** A visible, deliberately chosen default can remove repeated setup.
  Moving a selector into a smaller panel does not remove the work of using it on every task.
- **Reveal consequential choices when they matter.** Model identity, access, which output to pass
  and what starts running can change the result. Generated keys and receipt mechanics cannot be
  made into ordinary user responsibilities just because the backend needs them.
- **Count recovery and comprehension as effort.** A gesture that causes confusing repairs later
  may be worse than one clear choice now. Fewer clicks alone do not establish better interaction.

These criteria also apply to the coordinator's proposal. Hiding fields under Advanced, adding a
graph around the same forms or asking an AI to guess every decision does not satisfy them.

## Participants and basis

Three actual delegated agents received the user's design request, this UX brief and the existing
proposal. They formed initial positions before receiving one another's reasoning, then exchanged
objections. They share the same product context and proposal; their agreement is not independent
user evidence. No interface experiment or usability session was performed.

| Agent pseudonym, dated 2026-10-23 | Task and responsibility |
| --- | --- |
| Juniper-20261023-a | `ux_authoring_debate`: follow a person's idea through definition, execution and return. |
| Rowan-20261023-a | `ux_spatial_debate`: challenge navigation, ports, discoverability and equivalent pointer/keyboard/touch actions. |
| Linden-20261023-a | `ux_daemon_debate`: make the daemon absorb construction and recovery work while keeping consequential choices truthful. |

Willow-20261023-a is the coordinating agent pseudonym from the proposal. The following record
attributes reasoning to its contributor; it does not simulate votes or claim user acceptance.

## Must a person choose a model before drawing a relationship?

**Juniper's case for drawing first.** Someone can know that a comparison needs two investigations
before choosing their models. Requiring that choice first makes the person follow the system's
construction order. A tentative wire can express one limited intention: use this task's result
here. Resolving its mechanical details later can preserve the person's train of thought.

**Rowan and Linden's objection.** A visible wire may appear to promise a real data connection.
If every subsequent definition creates port repairs, sketching first merely postpones clerical work.
Keeping blank placement but enabling real ports after a model is chosen avoids that extra state.
Neither reviewer treats the current backend's limitations as sufficient reason to require this order.

The demanding example is a blank A → B → C sketch where A later becomes an extractor with
**Text**, **Images** and **Diagnostics**. Matching media types does not tell the system which content
the person intends. A chooser beside A can ask about those actual outputs; a generic binding form
would abandon the abstraction. Conversely, dragging an already named Text port to a named input
is a complete decision and must not ask for the destination again.

The counterproposal is to choose a usual model or agent once for the workflow, display its inherited
identity on tasks and allow overrides. That can provide real ports with less repeated setup while
preserving empty instructions and placement. This is a proposed addition, not an existing workflow
model-default contract. No reviewer supports silently replacing an unavailable model.

**After the exchange.** Juniper revised the preferred ordinary comparison to start with that
visible model/agent default, which often removes the need for tentative wires. Rowan gave more
weight to sketching first after Juniper's investigation/comparison example: the system's order of
definition should not dictate the person's order of thought. Both retained the model-unknown
connection as a candidate, using an open endpoint and **Choose output** instead of teaching a new
term such as provisional binding. Linden retained the concern that deferred repairs could cost more
than defining ports first. An adapter's declared default output can help only when it is a real,
visible ordinary-result choice; compatibility, output order or a familiar label cannot infer intent.

## Does one unfinished thought have to block useful work?

**Juniper challenges the original proposal.** Two defined tasks plus a disconnected blank idea is
an ordinary sketch, not necessarily an invalid workflow. Requiring the person to finish the third
card, delete it or move it elsewhere transfers bookkeeping back to them. An explicitly chosen run
scope could leave the thought where it belongs.

**Linden separates a real constraint from a product choice.** Admission must be bounded, deliberate
and atomic for the chosen work. It does not follow that every pending object on the canvas must
belong to that admission. The system should calculate required predecessors and explain the included
work visually. It must not secretly run every ready card or split a large plan into several commands.

The strongest objection is that selection can become an invisible execution mode. Clicking a card
to read its result must never quietly change what the next Run action will do. A visible count alone
also fails if the person cannot tell which two tasks it counts. The proposal needs a concrete scope
interaction that preserves one primary submission location.

The hard case adds an accepted, queued predecessor with an unapplied edit. Including its result
must not silently apply that edit. If it starts during review, the interface must preserve the
person's candidate and explain the actual change locally. Showing raw generations or demanding
whole-canvas reconciliation would move the daemon's work back to the person.

**After the exchange.** Rowan proposed keeping whole-plan Run as the ordinary default, with an
explicit **Choose tasks to run** action when scope needs changing. Juniper narrowed the selected-run
idea to that deliberate, temporary preparation on the same canvas. Linden withdrew the automatic
ready-subset interpretation of the first-round suggestion. An inspector click changes neither
scope nor execution. The primary action names the chosen tasks and changes; highlighting supplements
those names rather than replacing them. A dependency change invalidates or visibly revises the
candidate; the system cannot widen the submitted work behind the person's back.

For the edited predecessor P, the meaningful choice is **Use P before these changes** or **Apply
P's changes**, with the actual instructions or connection difference available at P. P is existing
work, not a new launch. A requested output that only exists in the edited definition cannot be
borrowed from the old one. If P starts before the edit is admitted, retain the candidate and explain
the running definition or future correction; do not switch alternatives silently. These cases are
why the reviewers retain scoped submission as a comparison candidate. A launch preview can itself
become the transaction-management interface the user rejected.

## Which work should disappear into the system?

**Linden's recovery example is grounded in current source.**
[`CanvasWorkspace.svelte`](../../../../../../apps/web/src/components/CanvasWorkspace.svelte)
currently offers Find submission receipt and Replay exact submission after a lost response.
[`canvas.ts`](../../../../../../apps/web/src/lib/canvas.ts) blocks editing until the pending request
is recovered. The existing exact lookup/replay operations can support a humane interaction:
**Checking whether work started**, followed by the confirmed result or a concrete unresolved state.
The person should not need to choose a protocol recovery mechanism. This does not make uncertain
external effects safely retryable.

**Linden's idle-workflow case.** When admitted work has settled but a saved idea remains, show
**Nothing running · 1 unfinished task**, or **Draft** if nothing has ever run. Completed cards keep
their outcomes. The workflow owner must reconcile this with completion rules; a frontend badge
cannot invent an open objective or change retained execution history. No task needs to be submitted
merely to keep a sketch alive.

Model or environment changes require a different judgment. Matching a label or port schema does
not prove that a replacement executor behaves the same. Linden explicitly qualified an initial
suggestion about automatic generation revalidation: absent an established equivalence contract,
preserve the task and say what changed, such as **Writing assistant was updated; review this task's
model**. Rechecking machine facts is automatic work; choosing a materially different executor is not.

Saved authoring removes the burden of remembering which ideas exist only in the current browser.
However, debouncing saves does not make their lifetime sustainable. The current finite receipt
accounting cannot casually be reused for an unlimited series of editing pauses. Linden identifies
the authoring save and recovery lifecycle as an unresolved daemon design obligation. The user should
encounter understandable storage pressure and a useful action, not receipt numbers or instructions
to manufacture a new request identity. Exact replay protection still applies.

## Can the quieter layout still make its actions discoverable?

**Rowan challenges hiding New workflow in a row menu.** Making it visible only in an empty project
helps create the first workflow and then hides the second creation. The proposed alternative is a
labelled **New workflow** action at the end of each expanded project's workflow list, with pagination
separate. Project and daemon menus retain less frequent settings. This costs some space; it should
be judged by whether a person can create a second workflow without hunting.

Connected ports and required empty ports should remain identifiable, with other real ports revealed
on selection, focus or wiring through a label such as **3 more outputs**. Connected wires retain
their actual named endpoints even when a task loses selection. At a dense crossing, selecting a wire
can dim unrelated wiring without moving the canvas. Information may become quieter; it must not
become impossible to recognize.

Click-output then click-input and tap-output then tap-input provide the same action as dragging.
An accessible connection chooser names **Draft announcement — Context**, not protocol IDs. Escape
cancels wiring or closes the inspector and restores focus to the card. Creation and movement must
not generate an accidental click that opens the inspector. These are proposed interactions, not
behaviors established by current tests.

## What would distinguish the remaining designs?

The reviewers propose comparing the same small journey in three forms: instruction-empty cards
with a visibly inherited model, genuinely model-unknown connections, and definition before wiring.
Place A → B → C, then make A a multi-output extractor. Ask what B will receive. Observe whether
the person recognizes the content, must reconstruct a forgotten relationship, or encounters
unexpected repair. This tests the cost of each order of thinking rather than declaring one intuitive
because an agent prefers it.

Then leave an unrelated blank thought on the canvas and start the useful chain. Compare whole-plan
Run with the explicit scope candidate. Include the edited queued predecessor P and inspect another
card before submission. The person should be able to explain which work starts, which P definition
is involved and what remains an idea. If the preview requires learning batch mechanics or makes
inspection feel dangerous, it has failed even if every command is technically correct.

Finally, create a second workflow under a populated project, distinguish identical names under two
daemons, reconnect to the unfinished arrangement and recover a lost Run reply. Observe hesitation,
unexpected navigation, reconstruction and recovery effort as well as errors. Repeat connection
actions with keyboard and touch. These are proposed discriminating observations; none were executed
for this debate, and agent agreement cannot substitute for them.

**2026-10-23 — Willow-20261023-a (coordinating agent pseudonym).** The exchange warrants concrete
refinements to the proposal: discoverable repeated creation, a visible usual-model choice,
automatic exact-request reconciliation and local explanations of changed work. It also reopens my
assumption that the entire canvas must be complete before useful work can run. I retain whole-plan
Run as the comparison baseline and record the explicit-scope alternative with its costs. Blank-card
wiring and scoped execution remain product choices to test, not features authorized by reviewer
agreement. Sustainable saving remains a daemon design obligation before implementation can promise
continuity. The [overview](../../README.md#overview) owns further action and assignment.
