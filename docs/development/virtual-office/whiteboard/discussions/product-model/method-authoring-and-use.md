# How people define, use and improve methods

**Consolidation, 2026-10-23.** [ADR 0052](../../../../../decisions/0052-canvas-authoring-and-method-experience.md)
now selects root-input activation, reviewed execution choices, export release, visual extraction and
library editing. The [side sprint](../../../canvas-experience/README.md) takes over complete methods
from main 04. Early snapshot customization and method-owned adaptive descendants remain deferred;
the debate below preserves alternatives and evidence that could reopen the selected baseline.

A method should let someone reuse a useful way of working without rebuilding its connections,
re-entering project details or learning a template language. The October 23 request reopens this
experience for debate. The canvas and daemon investigations did not settle the core product.

The [vision](../../../../../product/vision.md#methods-capture-reusable-preferences) owns selected
intent. The [original account](../../../milkdrift-remedial-planning-v3/product-reset.md#one-concrete-account-before-choosing-representations)
explicitly asks for current saved content when a queued use activates, followed by editable local
future tasks. [ADR 0051](../../../../../decisions/0051-daemon-task-workflows.md#methods-and-activation)
and [prompt 04](../../../daemon-workflows-implementation/04-local-methods.md) select detailed expansion
rules. Methods remain unimplemented in the current task path. The proposals below are comparisons,
not a new selected contract or implementation assignment.

Reviewers used the user's criteria from the [UX debate](canvas-authoring-debate.md): remove mechanical
translation, preserve thought and place, reduce repeated decisions, and expose consequential choices
where they matter. A reusable graph around parameter forms would repeat the rejected design.

## Start from work someone would repeat

The ordinary case is **Compare approaches**. A person supplies a question and reference material;
the method investigates alternatives, composes a recommendation and reviews its evidence. They use
it for animation research today and a different engineering question tomorrow. The reusable value
is the procedure and its expectations, not today's three topic names or previous result bytes.

The demanding case starts from a useful coding workflow containing a failed implementation,
corrective work, a private customer brief and files in its project folder. Saving everything that
happened would preserve an accident-filled history, not necessarily a procedure anyone should repeat.
The [outcomes debate](work-outcomes-and-delegation.md) also asks whether a fixed graph can express
the adaptive review/correction behavior the person actually wants.

### Debate: extract useful work or author a separate template?

**Mira's proposal** starts with selecting useful tasks and choosing **Save as method**. Show the
selected procedure and its boundary on the existing canvas. Incoming connections become candidate
method inputs; outgoing connections identify candidate results. The system generates identities
and internal bindings. The person names recognizable purposes such as **Question**, **Reference
material** and **Recommendation**. One incoming source used by several tasks can remain one input.

The important choices appear at highlighted boundary objects:

- Supply this context for each use, or deliberately retain it as reusable content.
- Expose this result to consumers, or keep it internal.
- Preserve an external ordering condition as **Start after**, rather than inventing a data input.

Internal task inputs that need the same chosen value receive it through derived connections. This
does not require interpolating user-written variable syntax into instruction strings. If distinct
uses have different meanings despite equal values, they remain distinct inputs. Labels alone cannot
establish semantic equivalence.

**Willow's objection** was that extraction can turn into a cleanup wizard for retries, private
artifacts and absolute paths. Mira narrowed the proposal: select procedure definitions, not result
states or execution history. Captured project data is not retained by default. A chosen corrected
definition can supply future instructions without copying its historical correction target. Selecting
both the failed task and its correction must not silently synthesize a retry loop. Present the two
future steps or let the person select the useful one. A path-dependent task needs an explicit
consuming-project interpretation or a local explanation that it cannot yet be made reusable; matching
folder names is not authorization to relocate it.

**The smaller alternative** is a fresh method canvas with input and result boundary cards, using the
same task editor and connection gestures. It avoids extracting accidental history but makes people
reconstruct work they already did. Keep scratch authoring available; compare extraction against it
using the same repaired workflow before deciding how much automatic conversion is worthwhile.

The daemon must validate any extraction accepted into the library through the same public path as
ordinary method saves. The frontend may suggest a boundary; it cannot secretly own the portable
meaning of templates or omit inaccessible content while claiming to have copied the entire procedure.

## Separate editing the procedure from using it

Library editing is an intentional context with a clear **Editing method: Compare approaches** title.
Task selection still uses the right inspector; editing individual tasks must not replace the canvas.
Inserting a method into a workflow uses a contextual chooser for the current daemon, without adding
another mandatory level between project and workflow. A library view is useful for deliberate
maintenance; every use need not navigate through it. Duplicate names retain daemon and method origin.

**Save method** or **Update method** changes shared reusable content. **Apply changes** in a workflow
changes that use's eligible local work. A consumer's right to use a method does not imply a right to
edit its shared body. Read-only users can still understand the inputs, results and relevant effects
they are choosing, within their permissions.

This distinction exposes another design obligation: the selected library Save installs a complete
validated body, while the proposed canvas lets people leave unfinished ideas. Saving an incomplete
editing draft must not change what queued uses will expand. Compare explicitly unsaved editing
against bounded saved authoring under the existing owners, using the same lifetime discussion as
[workflow drafts](../architecture/daemon-frontend-contract.md#what-genuinely-needs-new-persistence).
Autosave of editing progress and making a complete method available are different consequences;
they must be named accordingly. A new publication/version hierarchy does not follow from that need.

**Try this method** should create an explicitly identified ordinary use in a chosen workflow/project
and use its normal Run action. Its tools have real effects. An inspection preview never executes.
Testing on a separate folder is an explicit project choice, not an automatically promised sandbox.

## A queued use follows a changing method

Before activation, show a method card with its actual external inputs/results and **Uses latest saved
method when ready**. Its content preview is a view of what is saved now, not a promise of the exact
future execution. After expansion, show the actual copied tasks grouped by origin, with the selected
content available in details. Expanding or collapsing the visual group never changes the execution
contract or converts a reference into a copy.

### Debate: live improvements versus predictable connections

**The activation review** favors retaining latest-at-activation, because it is explicit user intent
and lets a procedure improve before delayed work reaches it. A compatible instruction correction
can then benefit pending uses without repeated approvals. The alternative of pinning content when
placed is more predictable but contradicts the selected default; it cannot be introduced as a mere
implementation convenience.

Mira challenged whether stable connector names solve the problem. They do not: an owner can change
the meaning of a text output without changing its type. The revised recommendation distinguishes
mechanically detectable incompatibility from trust in the method author's purpose:

| Owner change while a use waits | Proposed experience |
| --- | --- |
| Rename a method or an existing endpoint | Preserve identity and attached wires; update the visible label. |
| Improve internal instructions without changing exposed requirements | Follow the saved improvement at activation; retain the content actually selected. |
| Add a required input, remove an attached export, or otherwise make an existing connection incompatible | Block that use locally and retain its wires. Show the affected endpoint and repair choice; relaxing a requirement need not block it. |
| Change model, tool access or execution location | Compare against the consuming project's reviewed choice and current authority; never silently widen access or redirect files. Which permitted model changes need renewed human review remains a product question. |
| Keep the same type but change what a result means | Do not claim automatic compatibility proof. The author is changing a promise; affected-use visibility and trust matter. |

Stable endpoint identities are a proposed mechanism, not an existing template schema. Deleting and
recreating a similarly named output cannot automatically redirect an old wire. A shared daemon check
must protect these meanings for CLI and model callers as well as Svelte. Owner-save impact should
name only authorized affected uses and give a bounded count for protected others.

The demanding comparison is a queued **Compare approaches** use connected to a report consumer.
The owner renames its result, then changes it from a recommendation to raw notes, then adds required
evaluation criteria. These are three different situations; treating all as harmless latest content
or all as mandatory approvals loses the intended benefit. The precise change-review boundary remains
unselected. No additional save-confirmation screen is assumed necessary for every edit.

### What makes a use ready to activate?

Tarin found that the selected documents describe delayed dependencies and the acceptance point,
but do not fully specify the eligibility rule. Does expansion wait for every required root input
to have a value, or can it establish local children while some of those children wait? The former
keeps the live reference current longer but can delay independent work. The latter exposes work
earlier but selects the template sooner. Both must distinguish a supplied binding to a future result
from the result actually being available. A use with no outstanding gate may expand immediately.

Compare a method with one research task ready now and another needing tomorrow's brief. Waiting
for all inputs and expanding now yield different parallelism and different selected template
content. This is a product choice, not an incidental scheduler detail. The preview and customization
promise below depend on it; the exact rule remains open rather than silently inherited from tasks.

## When can a person customize one use?

The selected design permits edits to unstarted copied tasks. It does not promise an editing window:
an eligible child can start before the browser even receives the expanded graph. Merely telling the
user to edit after expansion makes speed and luck part of authoring.

Mira and the activation reviewer compared the same case: change the reviewer's instructions for one
future use while the reusable method should remain unchanged.

| Alternative | What the person can do | Cost or unresolved consequence |
| --- | --- | --- |
| Keep the selected live use | Change its boundary inputs before activation; edit copied children only while still queued. | Smallest contract, but cannot guarantee advance changes to its internal procedure. |
| Explicit **Customize a copy** before submitting | Bring the currently selected procedure into editable authoring, then deliberately Run it. | Proposed addition: this copy stops following library changes. It must not silently replace ordinary method use. |
| Materialize a use while withholding child entry | Inspect and edit accepted local work before releasing it. | Needs defined shared control/recovery behavior. A workflow-wide pause does not itself request early activation and also affects unrelated work. |

The reviewers favor comparing the explicit draft-copy alternative if early customization is a real
need. That preference is not a change to the selected contract. Converting an already queued use
would additionally need guarded replacement against concurrent activation; it is a separate, more
expensive case. A competing reading is that per-use inputs should express all expected variation,
leaving internal edits as an occasional later repair. The decisive observation is whether ordinary
uses repeatedly need internal edits that cannot be expressed without an oversized parameter surface.

The selected ADR does allow an already accepted expansion to materialize during a workflow pause
without child entry. That is distinct from requesting early activation of a particular queued use;
visual group expansion performs neither operation. The existing runtime pause defects remain
separately recorded in the daemon debate.

## A method result needs a clear release condition

Suppose **Review report** contains a writer and a reviewer. The writer's report exists while review
is still pending. If every outer wire follows its producer immediately, a consumer can bypass the
reason the method exists. If every output waits for the whole method, useful early downstream
processing becomes impossible or unnecessarily delayed. Inspecting an already available result is
separate from releasing it to downstream work and can remain available before that release.

The activation reviewer initially favors waiting for required method work by default, with deliberate
early exports such as **Draft report**. The author defines an export's content source and release
condition once on the result boundary; consumers do not repeatedly rebuild the internal dependencies.
The strongest alternative is producer-only release with explicit gates only where necessary. The
selected ADR already distinguishes producer dependencies and declared completion dependencies, but
the author's interaction and default are not settled.

Mira and Hazel sharpened the objection: successful reviewer execution is not approval. A reviewer
may return **This report is wrong** successfully. **Available after review finishes** describes order;
**Approved report** promises a decision. Neither a label nor a green task supplies that decision.
The [outcomes debate](work-outcomes-and-delegation.md#review-is-work-with-a-result) compares ordinary
critique, explicit decisions and bounded corrective continuation without parsing prose into control.

Likewise, nested template expansion can flatten one consistent snapshot as selected, but nesting is
not a loop or an independent running workflow. If a copied planner later adds more work, the fixed
copy group and export map do not automatically include those descendants. The meaning of an adaptive
method's final result remains a substantive open product question.

## Improve the library deliberately

A locally improved use can offer **Review changes for method**, showing changed instructions,
connections and boundary expectations. An authorized owner chooses which improvements become a
shared update; an ordinary consumer may retain a separate copy only through an explicitly permitted
copy operation. Do not automatically promote success, corrective history or private project content
into the shared template. One successful execution is not evidence of general usefulness.

The smaller alternative is opening the method and editing it manually with the useful run beside it.
A specialized comparison is justified if it removes real repeated reconstruction. It must not grow
into automatic lesson extraction or a permanent learning service. Project-specific procedures can
remain ordinary files in Git as the vision specifies.

Cross-daemon reuse remains an explicit content copy with local model/tool mapping, as selected in
the ADR. It is neither a live update subscription nor remote workflow execution. The importer should
repair unavailable capabilities beside affected tasks while preserving the copied procedure, rather
than fill out a second global configuration form. Exact capability compatibility and authorized
content transfer remain daemon responsibilities.

## Contributions and remaining tests of the design

**2026-10-23 — Mira-20261023-a**, agent pseudonym `method_authoring_debate`, developed visual boundary
extraction and scratch authoring, then narrowed extraction of repair history and the early-copy
proposal after challenge. She identified the lack of a guaranteed post-expansion editing window.

**2026-10-23 — Tarin-20261023-a**, agent pseudonym `method_activation_debate`, traced the selected
activation contract and challenged live interfaces, export release and local customization. Peer
objections narrowed claims about semantic compatibility and approval; he also exposed the incomplete
activation eligibility rule and revised the adaptive-method premise after Hazel's challenge.

**2026-10-23 — Hazel-20261023-a**, agent pseudonym `daemon_lifecycle_debate`, challenged whether
templates capture adaptive work, whether reviewer success establishes approval, and how future
descendants relate to exported results. **Willow-20261023-a**, coordinating agent pseudonym, traced
primary intent and current owners, challenged extraction and release timing, and synthesized this
record.

These are actual peer exchanges with shared context, not independent user evidence. No method
implementation, interactive prototype or usability exercise was executed. Compare the same repaired
workflow through extraction and scratch authoring; the same delayed use through owner edits; and the
same one-off customization through each alternative above. The [whiteboard](../../README.md#overview)
owns further action. This round does not settle the whole product or make prompt 04 ready to execute.
