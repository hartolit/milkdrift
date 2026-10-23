# What work are we asking the system to carry through?

**Consolidation, 2026-10-23.** [ADR 0052](../../../../../decisions/0052-canvas-authoring-and-method-experience.md)
selects reusable segments inside adaptive workflows and truthful execution/review claims for the
prepared side sprint. Adaptive method groups and scoped human-decision gates remain deferred
extensions. The comparisons below retain their rationale; deferral does not claim those larger
product questions have been solved.

The canvas is useful only if it helps someone get a result. The October 23 request for more product
debate asks what methods and workflows should actually do, beyond arranging tasks or implementing
their protocols. This discussion examines delegated work, review, completion and reusable improvement.
It complements [method authoring and use](method-authoring-and-use.md) and the
[daemon contract debate](../architecture/daemon-frontend-contract.md).

The selected [vision](../../../../../product/vision.md#work-starts-with-tasks-and-can-grow) already
includes automatic research, criticism and correction within explicit limits. Its product meaning
does not require every method to become an independent adaptive workflow. The alternatives below
are proposals from actual peer debate; no new method behavior or user validation is claimed.

## Give the system an outcome, not a clerical assignment

For research, a useful request is **Compare approaches, cite the evidence and recommend a Rust
experiment**. A person should be able to state that purpose once, provide relevant context and choose
a familiar procedure when one helps. A planner can then decide what research is missing; a synthesis
can identify a gap and request another bounded investigation. The person should not have to launch
each round, invent identities or reconnect the same known sources manually.

That does not make a method mandatory. A plain planning task is the smaller useful choice for
unfamiliar work. A fixed **Compare two approaches** method is useful when the person wants exactly
that repeatable procedure. Neither should imply a universal research algorithm or a hidden promise
to keep working indefinitely.

For coding, the connected example is setup → implementation → review → correction → verification.
The reusable setup segment creates an ordinary project structure in the chosen project. Review
compares the result with the requested behavior; finding a missing parser case can lead to a repair
and a check without the person operating each intermediate task. The canvas should explain that
cause: **Review found a missing case; fixing and checking it**, with the actual evidence available.

This is the desired experience. The [connected journey](../../../daemon-workflows-implementation/journey.md)
describes the selected controlled example; it is not proof that arbitrary real-model work will
produce the intended outcome. Ordinary project files and Git remain the home of project knowledge.

## Debate: what does delegated continuation permit?

The frontend already exposes **Let this model propose follow-up tasks** and a separate option to
retry refused proposals in [TaskPermissions](../../../../../../apps/web/src/components/TaskPermissions.svelte).
The [model-admission owner](../../../../../../crates/runtime/src/tasks/model_actions.rs) can accept
authorized batches automatically. A person could reasonably read “propose” as “show me suggestions
before doing anything”; that comprehension question needs review. The option is not missing.

**Hazel's proposal** is to explain the consequential choice at the task or method: it can create and
run follow-up work within the shown scope and allowance. Useful continuation should not require a
fresh approval for every research or repair task. The exact technical policy is the daemon's job;
the interface shows what the person is delegating and what will make the work stop or ask for help.

**The strongest objection** is that a broad “work until done” control can conceal substantial
discretion. Selecting a method must not silently widen permissions, transfer an objective to another
project or allow unlimited descendants. Use the existing authority and cumulative accounts; expose
material choices such as changing the objective, access or model when those choices are not already
delegated. Avoid both an expert policy form and a reassuring label that promises more than the
contract enforces.

Compare one research follow-up that stays within the requested question with a proposal to change
the question or write project files. The first should continue under its existing permission; the
second needs the applicable deliberate choice. A method author's preferred settings can guide the
consumer, but cannot grant authority on their behalf. Presets or a new autonomy taxonomy remain
unselected; clearer use of the existing choice may be sufficient.

## Review is work with a result

A reviewer that successfully says **The report is unsupported** has completed its execution. It has
not approved the report. The current [workflow completion logic](../../../../../../crates/runtime/src/tasks.rs)
uses task settlement, required obligations, a finish condition and pending admission state. It does
not judge whether a recommendation is convincing or code satisfies its brief.

Mira challenged an export named **Approved report** whose only gate was a successful reviewer.
Tarin's proposal to wait for required method work solves ordering, not substantive acceptance.
Hazel identified three distinct usable procedures:

| Procedure | What its result means | What happens after negative findings |
| --- | --- | --- |
| Produce a critique | The reviewer supplies findings for a person or another task. | The consumer chooses what to do; no implied approval or repair. |
| Carry out bounded review and repair | A deliberately authored reviewer/repair procedure can create future work and record its decision. | Continue under the selected scope; retain findings and repair history; ask or stop at the actual boundary. |
| Wait for a person's decision | An identified person decides a consequential question about the result. | Preserve the result and affected future work until an authorized answer. |

None of these requires the daemon to infer acceptance from arbitrary prose. A structured decision
would still be an attributable judgment, not proof that the content is correct. Display who or what
judged it and the supporting evidence. Successful tests establish their tested behavior; they do not
automatically certify the whole objective. No mandatory score, rubric or second objective-state
subsystem follows from this distinction.

The technical `automatic_repair` choice repairs refused model action proposals. It does not supply
a general substantive bug-fixing procedure. That requires authored review/correction work. Reusing
the same word for both would make a small checkbox appear to provide a core product capability it
does not establish.

Successful but inadequate work also needs improvement. Current
[correction adoption](../../../../../../crates/runtime/src/tasks/admission.rs) applies to a settled
Failed/Stopped original and a successful explicit correction. It is not the only legitimate reason
to improve a result. Create future improvement and rebind eligible consumers without relabeling the
earlier execution as failed. A single approachable **Improve this result** interaction may present
the appropriate real operation; shared admission must preserve its actual meaning and started work.

## Debate: a reusable segment or an adaptive method?

**Hazel's smaller complete design** keeps methods as repeatable segments inside a growing workflow.
A project-setup method expands locally; research and later repair can be ordinary work around it.
This satisfies the documented journey without making the method own every task subsequently added
by one of its copied planners. A saved review method may simply produce findings.

**Tarin's alternative** explores a method promising **Implement and review until acceptable**. It
would need a stable final deliverable, explicit continuation/release meaning and a truthful account
of dynamically created work. His concrete candidate is a final assembly task with a per-use input
that is supplied only when a declared decision selects a result. Structured batches could add repairs
and revise future inputs before release. Existing commands are building blocks, not evidence that
this method behavior works or is sufficiently expressive.

The discriminating case is a method with a planner and an initially known result producer. The
planner adds research; the reviewer later adds a repair. If completion only counts the original
copied tasks, the group may appear finished before the added work. If every descendant is absorbed
automatically, unrelated later work can change the group's meaning. Selecting the “latest child
result” is not a stable export contract.

After Hazel's challenge, Tarin revised his initial premise: adaptive **workflows** are explicit
intent; adaptive **method groups** owning descendants are an additional product choice. Keep both
alternatives visible. A stable collector does not by itself settle membership, completion, failure
or correction redirection. No new child scheduler, hidden recursion or universal loop node is
justified by calling the method adaptive.

The strongest objection to the smaller design is repeated surrounding plumbing: if every use needs
the same review/repair continuation, excluding it from the reusable unit may defeat the purpose of
methods. The strongest objection to the larger design is recreating a workflow engine inside each
method. Compare the same implementation-and-review case and count what the person must repeatedly
assemble, explain and repair. Do not decide from a project-setup example that never exercises the
disputed behavior.

## Human decisions should stop the relevant work

A preference-dependent question can deserve a checkpoint while independent research continues.
Current Wait is workflow-wide; its input coupling and pre-entry defects are recorded in the
[daemon discussion](../architecture/daemon-frontend-contract.md#pause-must-preserve-work-that-has-not-entered).
It cannot be presented as an already working branch-specific decision gate.

An ordinary task waiting on a missing required supplied input is a smaller existing building block.
It is not yet a complete human-decision feature: the system must bind an authorized answer to the
intended question and future work, distinguish it from unrelated context, and keep it discoverable
after reconnect. Compare that narrow boundary with a dedicated decision mechanism only if the
journey requires one. A global pause remains useful when the person actually intends to pause all
future entry.

No universal approval after every review is proposed. A person should return to the relevant
question and result on the same canvas, without reconstructing task IDs or manually restarting
unrelated branches. Notification and unattended-review behavior remain to be designed; neither
constant interruptions nor silent indefinite waiting is an established answer.

## Reuse improves by a deliberate choice

Mira and Hazel compared extracting a clean useful segment with extracting an entire successful run.
They favor the former as the baseline. Derive mechanical input/output boundaries, show what will
repeat, and let the person choose the actual procedure. Retrying a failure once does not establish
a reusable repair algorithm. Private inputs and old artifacts do not silently become library content.

A later improvement can be compared with the method by an authorized owner. Automatic promotion
after success would confuse a local outcome with a general preference and create the permanent
learning machinery the vision excludes. The [method discussion](method-authoring-and-use.md#improve-the-library-deliberately)
owns the authoring interaction and smaller manual alternative.

## Contributions and questions that remain open

**2026-10-23 — Hazel-20261023-a**, agent pseudonym `daemon_lifecycle_debate`, developed the research
and coding cases, challenged adaptive method membership and distinguished execution, proposal repair
and result quality. An initial missing-UI-control suspicion was withdrawn after Willow identified
the existing control and Hazel verified it. It is not retained as a defect.

**2026-10-23 — Mira-20261023-a**, agent pseudonym `method_authoring_debate`, challenged review-success
as approval and narrowed extraction to clean selected definitions. **Tarin-20261023-a**, agent
pseudonym `method_activation_debate`, proposed explicit export release and a stable adaptive-method
collector, then accepted the distinction between adaptive workflows and an additional method-group
contract. **Willow-20261023-a**, coordinating agent pseudonym, checked current source and synthesized
the exchange. These reviewers share context; agreement is not independent usability evidence.

No methods were executed and no new behavioral experiment or user observation was performed in this
round. The next comparison needs a negative review, a successful-but-inadequate result, a recurring
repair sequence worth reusing, and an unrelated branch continuing during a human question. It must
show whether the smaller design removes enough repeated work and whether users understand what
completion claims. The [whiteboard](../../README.md#overview) owns further action. The product design
remains open, including the method boundary, activation timing, result release and meaningful human
decisions; this discussion is not a finished implementation plan.
