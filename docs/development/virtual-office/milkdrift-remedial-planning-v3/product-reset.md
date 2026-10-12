# Product definition reset — 2026-10-12

**Continuation supplied:** the user subsequently authorized finishing planning and technical choices.
The [current sprint](../daemon-workflows-implementation/README.md) is complete and ready for explicit
implementation authorization. [Vision](../../../product/vision.md) and
[ADR 0051](../../../decisions/0051-daemon-task-workflows.md) own the chosen product and design.
The three decision areas recorded below are resolved there: local library/snapshot expansion,
shared task admission and directional daemon-owned connections. Preserve the findings and prior
disagreements. This reset record is historical; its instructions to await another account no longer apply.

## Historical reset account

The user has asked to step back, preserve the existing work, and reconstruct the product before
deciding how to implement it. The [new primary account](vision-source-2026-10-12.txt) is the current
direction to develop, including its final correction about editable materialized methods. It is
not proof that all suggested mechanisms work. [U20](intent-source-excerpts.md#u20--product-definition-reset-and-reusable-review-practice)
records the accompanying instruction and its standing.

The previous decision packet and P00–P09 are **superseded as the current proposal**. Their source
findings, executed observations, alternatives and disagreement remain available. Do not execute or
incrementally patch that program into the new product. The user will return with a reconstructed
idea; this update incorporates the supplied direction and preserves questions for that conversation.
At that checkpoint it neither started a rewrite nor finished a replacement architecture.

## What the new account establishes

| Concept | Current intended meaning | Consequence for the previous proposal |
| --- | --- | --- |
| Daemon | A workflow's daemon persists, executes and exposes that workflow. Closing or changing a frontend does not stop it. Multiple clients observe the same owner. | Retain this outcome. A workflow spanning several workflow owners is not the desired model. No new shared-state federation is needed to explain remote tools. |
| Task / operation | Reusable behavioral instructions combined with inputs that may only become available when the task runs; output becomes later input or evidence. | A missing input value at definition time is normal pipeline behavior, not by itself a need for runtime code generation. Task terminology and data shape must be chosen from this account, not copied from the existing enums. |
| Split and compose | An agent can divide a brief into research assignments; several tasks execute concurrently; a later agent evaluates their results and decides the next work. | Splitter and compositor can be roles of ordinary model tasks. Which tasks are ready, which results exist and when composition can run still need explicit execution meaning. One visual node need not make model text the scheduler. |
| Evolving workflow | Begin with a useful partial plan, inspect results, then append or revise future work. Earlier execution remains inspectable and fixed. | Preserve adaptability as an outcome. A separate WorkCommitment entity, region language or particular reconciliation algorithm is not selected by this account. |
| Method template | A reusable authored procedure, protected from mutation by its consumers. Its owner can edit the reusable definition. | This is not an immutable-for-all-time template or an immutable execution. The final paragraph corrects the earlier suggestion that materialized work must remain static. |
| Method use | A scheduled reference materializes the template current at execution into local workflow tasks with origin information. The copied future work can then be adapted without editing the template. | This differs from both a definition-time pinned child call and an immediate independent workflow copy. Neither existing operation silently supplies the desired contract. Exact resolution/acceptance and storage shape remain to be designed. |
| Execution environment | An advertised local or remote tool surface. The workflow owns its purpose, procedures and documentation; the host enforces the access and limits it offers. | Remove the assumption that remote execution needs published workflow methods. Do not require a product-specific environment-documentation engine where ordinary tools/files suffice. Host acceptance, process state, cancellation and access enforcement are different from the workflow's explanatory notes. |
| Timeline and discovery | Show left-to-right progress, titles/descriptions, actual start/end times and clear pending/running/completed states; let agents find recent or named work without already possessing its ID. | Treat timeline as an inspection/authoring experience. Parallel dependencies cannot be replaced by one serial timestamp order, and queued work has no invented execution time. Search must retain the selected daemon/actor scope. |

The new account calls method availability “global” while excluding external workflow methods and
making workflow execution daemon-local. These are not necessarily contradictory: global may mean a
daemon-wide library, or portable templates copied between owners. It cannot silently mean live remote
workflow execution. Library ownership, sharing and update propagation remain a product choice.

The text's proposed SSH/mesh/onboarding sequence is explicitly exploratory. Its desired outcome is
usable persistent host connections and a frontend that can explain the environments its selected daemon
uses. Discovering a peer, trusting its identity, authorizing tool execution and authorizing frontend
inspection/configuration are distinct decisions. Showing a remote tool through the workflow owner may
satisfy visibility without granting direct peer administration. Automatic frontend authority inheritance
is therefore a candidate to examine, not an approved credential-distribution design. Do not silently
discard the desired seamless experience or impose the old grant UI as the new product definition.

## One concrete account, before choosing representations

A person asks daemon A to research a Rust animation project. An agent uses the goal to define three
research tasks with instructions and declared result locations. Their content is not yet known.
A composition task starts when the required research results or explicit failure
outcomes are available; it reasons over those inputs and proposes the next local workflow steps.

One next step refers to the project-setup method. Until that step activates, the user intends it to
refer to the reusable template rather than contain an old baked copy. At activation, the daemon must
establish which template body was actually materialized and which local tasks it produced. A retry
after an interrupted expansion cannot quietly choose a different body or insert the expansion twice.
That requires retained identity/content and a defined acceptance point; it does not by itself justify
the previous revision hierarchy or public versioning UI. The template's title alone is not enough to
explain a later execution if its contents have since changed.

The resulting local tasks can be repaired prospectively after criticism. Editing them does not mutate
the reusable template or rewrite completed tasks. The appropriate treatment of an already running task
still needs an explicit rule; the text is clearest about future versus completed work. A remote command
on B is a tool call owned locally as workflow work, with B enforcing its own accepted operation and
limits. The phone can disconnect throughout. Its return reconstructs the actual daemon state, including
parallel work and remote uncertainty, rather than reconstructing a workflow from client memory.

This trace expresses the supplied intent and exposes missing choices. It is not a claim that the old
code already performs it, a schema design, or a new implementation assignment.

## Historical questions resolved by the continuation

1. **Method library scope and publication of edits.** Is the library per daemon, or can templates be
   distributed/copied between daemons? Does saving an owner edit immediately affect every unmaterialized
   reference, or is there an explicit published/ready template? The stated preference is current-at-
   activation; any pinned alternative must be presented as a different behavior, not quietly substituted.
   Retain the requested protection against deleting scheduled references until an explicit alternative
   refusal/recovery policy is agreed.
2. **Materialization and future change.** What counts as activation; can expansion proceed partially;
   what may change after some expanded tasks start; and how are parallel outputs/failures handled?
   What admits task-proposed additions or changes, and what happens when proposed work is invalid or
   outside the agreed limits?
   Prefer deciding this on the concrete research/setup/repair trace before choosing source forms,
   transaction machinery or an autonomous-session entity.
3. **Connection and actor experience.** Does a frontend act through the selected daemon's authorized
   identity, connect to peers directly, or support both explicitly? Visibility should not require the
   person to reconstruct hidden topology, but the same peer link must not accidentally authorize a
   different user or client. SSH versus existing transports follows that decision, not vice versa.

These were the retained questions at the reset checkpoint. ADR 0051 now resolves them and the current
sprint assigns their implementation. Preserve the original alternatives below as evidence; do not
send this list back to the user or restart dependent planning.

## What survives, and what no longer constrains the design

Preserve actual source findings, bounded runtime/CLI/host observations, refusal cases and the history
of changes in recommendation. They remain evidence about the archived implementation. The observed
nested-cancellation inconsistency remains a real finding there; it is not automatically the first
task of a new product. Tested behavior does not establish a requirement to carry its mechanism forward.

Elin's source reread distinguishes concrete similarities from differences: existing
[task configuration](../../../../crates/blueprint/src/model/node.rs) and
[dispatch](../../../../crates/runtime/src/engine/dispatch.rs) already separate declared inputs from
their later values. [Pinned subworkflow configuration](../../../../crates/blueprint/src/model/structured.rs)
and [execution](../../../../crates/runtime/src/engine/structured/subworkflow.rs) instead select an exact
revision and create a child run, while [copy](../../../../apps/daemon/src/host/commands/authoring.rs)
creates a separate workflow. Neither establishes the newly described deferred local materialization.
[Peer HTTP access](../../../../apps/daemon/src/http/peers.rs) and
[serving authority](../../../../crates/capability-host/src/serving/client_authority.rs) distinguish actors;
those are current constraints to understand, not a settled design for the new connection experience.

The prior Program/GraphNative migration, WorkCommitment placement, ProgramAgreement, publication,
managed-resource inventory and evaluated-learning program are no longer selected requirements for the
replacement. Their usefulness must be justified from the new product rather than inherited from the
effort spent specifying them. U17's careful meaning of evidence and U18's desired reconsideration can
still inform the discussion without requiring those particular subsystems. U19 forbade silently losing
repair in the then-proposed migration; archiving the old code is not a claim to have migrated old data,
running work or external effects. Any actual transition obligations must be established if a transition
is later assigned. Nothing is deleted or deployed now.

## Review-method lesson and preservation

Rowan-20261010, coordinator agent pseudonym contributing on 2026-10-12, records the methodological correction: the prior review
found real source defects and improved local consistency, but its increasingly precise design was not
validation that the intended product had been understood. Repeated derivative documents and reviewer
agreement must not substitute for a stable user account. “No foundational blocker” was a verdict about
the selected candidate, not proof of product value. A planning sprint must be able to conclude that
the thesis needs reconstruction instead of producing a larger implementation program.

The requested reusable [cohesive review practice](../../practices/cohesive-review.md) captures the
useful methods and this failure mode. It includes a planning sprint that produces an implementation
sprint only after its product premises survive review, and a portable kickoff prompt. It does not
export Milkdrift's architecture or imply a measured guarantee against mistaken premises.

Delta and Elin reviewed this reset and the reusable practice after participating in the v3
investigation. Their review is informed by that work and exposure to its proposals, not a blind
independent review or evidence that the replacement product has been validated.

Before these documentation changes, both local preservation references were created at
`f821ce093742246390f17edc77252ceb75672f97`:

- branch `codex/milkdrift-pre-product-reset-2026-10-12`;
- annotated tag `milkdrift-pre-product-reset-2026-10-12`.

No checkout reset, deletion, push or production change accompanied those references. Keep the v3
sources and discussions available through the reconstruction; use this document as the current
interpretation, with earlier selections explicitly marked historical.
