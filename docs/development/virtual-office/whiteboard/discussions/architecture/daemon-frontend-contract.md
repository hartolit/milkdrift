# A daemon contract that supports understandable work

**Consolidation, 2026-10-23.** [ADR 0052](../../../../../decisions/0052-canvas-authoring-and-method-experience.md)
selects the contract changes and their boundaries. The [side sprint](../../../canvas-experience/README.md)
owns repair/adoption assignments. S01 was assigned on October 24; its
[handoff](../../../canvas-experience/handoff.md) records implementation and verification. The
diagnostics below describe the earlier source, while canonical status owns current behavior.

The daemon must earn its interface. If supplying an unrelated workflow input releases a decision wait,
normal progress invalidates an unchanged edit, or drawing another wire changes an existing result
allowance, a prettier frontend cannot provide a coherent experience by itself.

This discussion extends the [UX debate](../product-model/canvas-authoring-debate.md) after the user
explicitly asked reviewers to challenge the daemon's design and implementation too. Its standard
remains the user's work: reduce manual translation, preserve context, remove repeated decisions,
and make consequences understandable. Current code is evidence of what exists, not proof that
its policies are useful. The [vision](../../../../../product/vision.md) and
[architecture](../../../../../architecture.md) retain ownership of selected product meaning and
system boundaries; this discussion proposes corrections and records evidence, not implementation.

## What the daemon and frontend should each promise

A person should be able to leave the client while admitted work continues, return to its actual
state, and change future work without managing scheduler or storage machinery. The daemon already
owns admission, readiness, external execution and recovery. Saving an incomplete arrangement would
add a preservation responsibility; it should not turn sketches into fake executable tasks.

| User action or expectation | Daemon responsibility | Frontend responsibility |
| --- | --- | --- |
| Place, arrange and inspect work | Supply authorized identities and relationships; preserve saved content if that feature is adopted. | Gestures, camera, selection, contextual inspector, keyboard/touch access and immediate local feedback. |
| Connect two tasks | Own binding meaning, adapter compatibility, output limits and derived readiness; validate authoritatively. | Let the person choose recognizable content and destinations, show compatible targets, preserve an unfinished choice. |
| Run or apply a change | Accept one deliberate bounded change atomically, checking actual starts, sources, authority and relevant versions. | Explain what the action includes, retain exact pending intent and avoid silently changing its destination or scope. |
| Pause or encounter a busy project | Preserve eligible future work and distinguish temporary waiting from failure, stopping and uncertain effects. | Explain what is waiting and why, without requiring artificial dependencies or correction tasks to repair scheduling. |
| Lose a reply or reopen the client | Retain authoritative acceptance and exact recovery evidence. | Perform bounded exact reconciliation automatically and display confirmed facts or remaining uncertainty. |
| Repair an invalid operation | Return a bounded, authorized explanation tied to the affected task, connection or setting. | Keep the surrounding work intact and present the meaningful local choice. |

“Thin client” means no competing scheduler or semantic authority. It does not mean rendering DTO
fields as forms, forbidding useful client state, or requiring a server round trip for every pointer
movement. Conversely, a client cannot compensate for a missing relationship contract by guessing
what old dependencies meant.

## Pause must preserve work that has not entered

Three diagnostic cases were executed against the production control/runtime/redb path at `a19b5f0c`,
using a controlled executor. The [evidence record](../../../../verification-evidence.md#daemon-lifecycle-design-diagnostics)
describes the fixture, command and limits. The assertions reproduce current behavior; they are not
acceptance tests endorsing it.

| Sequence | Observed result | Assessment |
| --- | --- | --- |
| Wait for explicit approval, then supply an unrelated input | The question disappears and scheduling becomes possible. | Confirmed pause/input coupling; conflicts with the current decision-wait experience. The intended reply semantics need clarification. |
| Freeze a start, accept Wait, then execute its dispatch | Task becomes Failed with no external entry; Continue does not redispatch it. | Reproduced lifecycle defect for a pause that should preserve future work. |
| Freeze a start, accept Wait, reopen the store, recover, Continue, tick | Recovery and tick return no dispatch; task remains Running with no external entry. | Reproduced runtime recovery liveness defect. The production worker path explains why ordinary polling does not repair it. |

[`BindInput`](../../../../../../crates/runtime/src/tasks/admission.rs) clears both the human question
and model-admission wait. [`Wait`](../../../../../../crates/blueprint/src/task.rs) carries only a
question; it cannot identify which input would answer it. The
[earlier ADR](../../../../../decisions/0051-daemon-task-workflows.md) describes waiting for a required
input and resuming after an authorized reply. That is the strongest defense of automatic resumption,
but there is no reply identity in the implemented command. The browser instead says **Wait for a
decision** and offers **Resume workflow**. Attaching unrelated context cannot reliably mean approval.

Hazel initially proposed separating wait causes more broadly. After challenge, the smaller
recommendation is to preserve an explicit question until Continue, keep model-owned diagnostics
scoped to their source, and add a bound answer only if the richer reply feature is selected.
Missing required task input already has its own readiness behavior. A generic hold registry is
unnecessary for this correction.

[`execute`](../../../../../../crates/runtime/src/tasks/effects.rs) currently settles a paused,
unentered start as Failed. Its recovery path omits a paused Running start without making it queued;
[`workers`](../../../../../../crates/capability-host/src/worker.rs) recover once before ordinary tick
polling, which selects queued work. Continue clears the question but does not restore that candidate.

The proposed promise is to defer the exact accepted start while non-entry is established. Ash
challenged simply copying the existing capacity-deferral branch: a pause can arrive after successful
preparation has acquired resources. Deferral must release those prepared resources and worker
ownership correctly while preserving the frozen definition. This is daemon lifecycle work, not a
reason to ask the person to make a correction task. Entered or uncertain work remains a different
case; no automatic re-entry is proposed.

## A busy project is not necessarily a failed task

Elm traced project defaults into whole-root file claims. Hazel then followed an overlap through
[`redb claim acquisition`](../../../../../../adapters/redb-store/src/native_paths.rs),
[`host mapping`](../../../../../../crates/capability-host/src/native_paths.rs) and
[`runtime preparation`](../../../../../../crates/runtime/src/tasks/effects.rs): the conflict becomes
`InvalidDocument`, then `BoundaryBeforeEntry`, then terminal Failed. This combined path was
inspected, not exercised as a concurrent project journey during this investigation.

Two tasks in a write-enabled project can contend even when the person's intended work is parallel.
The current frontend does offer none/read access overrides; the finding is not that overrides are
missing. The poor outcome is requiring people to draw extra order arrows or create corrective work
to compensate for temporary allocation contention.

Propose a typed resource-busy result and bounded waiting before entry, with current authority checked
when work can proceed. Retain the protected claim of an uncertain writer. Identity changes,
corruption and permanent refusal must not be mislabeled as transient contention. A permitted blocker
may be named; otherwise **Project is in use** is sufficient.

The separate policy question is whether project membership should imply all its usual file/tool
access. Compare current inheritance with a visible usual no-file/read/write choice for mixed research
and coding. Automatically guessing access from instructions is not an acceptable shortcut. Neither
that preference decision nor a new default setting is required to fix temporary contention handling.

## Progress should not make an unrelated edit stale

Human edits compare the workflow's storage generation in
[`admission`](../../../../../../crates/runtime/src/tasks/admission.rs). That value also advances for
retained reports and readiness explanations. The existing
[`model admission`](../../../../../../crates/runtime/src/tasks/model_actions.rs) instead uses a
decision generation that excludes ordinary progress. A person writing a new synthesis task while
research reports progress can therefore receive a whole-workflow conflict without their intended
definition changing. The browser's refresh/review path compensates for this contract choice.

This is an established design limitation, not evidence of corrupted execution or a measured frequency
of interruptions. Hazel challenged Ash's first narrowing: reports can include settlement and output
publication, which a particular decision may depend on. The revised recommendation is to let all
clients use meaningful authoring guards, allowing unrelated observation changes while checking the
affected task's version/start, referenced sources and current authority. Existing decision generation
is a credible starting mechanism, not a proven complete policy or permission to merge arbitrary edits.

The smaller frontend alternative repeatedly refetches and retries after comparing relevant facts.
It makes every client reconstruct a concurrency policy and leaves another race. The shared owner
should decide whether a change still means what was requested. An edited task starting must still
refuse that edit, with an explanation at that task.

## A wire needs one durable meaning

Current definitions require both a data binding and the producer's identity in an explicit dependency
set. The browser combines independent ordering and implied ordering when saving; after reload it
cannot distinguish them. Removing a saved data wire therefore retains a **Run after** relationship.
Correction adoption uses a different heuristic and can remove the old dependency when no remaining
input references it. The source paths are
[`definition validation`](../../../../../../crates/blueprint/src/task.rs),
[`client construction`](../../../../../../apps/web/src/lib/canvas.ts),
[`readiness`](../../../../../../crates/runtime/src/tasks/inputs.rs) and
[`correction adoption`](../../../../../../crates/runtime/src/tasks/admission.rs).

This is information loss in the current contract. A helpful connection endpoint cannot recover the
person's original intent from the merged set. Ash and Elm recommend deriving data prerequisites from
bindings while retaining genuinely independent ordering explicitly in new definitions. Readiness,
cycle checks and corrections must use the same meaning. Success, optional absence and settled
outcomes remain distinct; deleting a field blindly would change their behavior.

The smaller alternative retains current fields and stores owned authoring provenance. It is viable
only if it does not become a second authoritative graph. Existing accepted definitions cannot be
assigned invented provenance: preserve historical meaning and make an explicit prospective choice
where an edit encounters ambiguity. The discriminating case is data-only versus data-plus-independent
order, followed by reload, wire removal and correction. They must produce different intended results.

## A connector should not conceal a changed output promise

[`CanvasWorkspace`](../../../../../../apps/web/src/components/CanvasWorkspace.svelte) divides a
producer's output allowance across the distinct connected result names. Its
[test](../../../../../../apps/web/src/components/CanvasWorkspace.test.ts) explicitly expects a first
2 MiB declaration to become 1 MiB after connecting a second output. The sum-of-declared-maxima rule
is in blueprint validation; actual aggregate publication has a separate runtime bound.

Elm's counterexample is a 1.5 MiB first result and a tiny second result. They can fit the aggregate
limit yet exceed the new equal share. This is a source-predicted outcome, not a measured external
producer incident. Moving the same allocation into Rust would preserve the user's surprise.

Debate stable adapter-owned per-output ceilings under an actual aggregate bound against stable
finite allocation with an explicit local refusal when another connection cannot fit. Capacity and
reservation must remain truthful. The narrower refusal may be sufficient; forcing people to allocate
bytes every time they draw a wire may instead justify changing the contract. Compare asymmetric
outputs and both connection orders before selecting that policy.

Compatibility is also incomplete. Process authoring advertises stdout/stderr as arbitrary binary;
model additional inputs expose no accepted media-family contract, while model materialization rejects
unsupported binary with no safe mapping. See
[`process ports`](../../../../../../adapters/local-process/src/authoring.rs),
[`model ports`](../../../../../../adapters/model-provider/src/adapter/authoring.rs) and
[`materialization`](../../../../../../adapters/model-provider/src/adapter.rs). The combined
stdout-to-model journey was not executed here. The source establishes that the current metadata
cannot tell the frontend all the restrictions its adapter already knows.

Expose the actual adapter-owned rules and reject known incompatible bindings through shared admission.
Recheck concrete bytes when executing: a media label cannot prove valid text or useful model input.
Do not duplicate adapter rules in Svelte, relabel arbitrary bytes as text or silently insert a
conversion. The user should see an actionable explanation beside the incompatible endpoint.

## Read and error contracts should support the action being performed

A cold page of up to 50 task summaries leads the
[`canvas loader`](../../../../../../apps/web/src/components/CanvasWorkspace.svelte) to request each
task's details in a four-request pool. Summaries contain merged dependencies; details supply actual
bindings and also load result previews through the
[`read owner`](../../../../../../apps/daemon/src/host/tasks/reads.rs). Known unchanged tasks are cached.
The [stream](../../../../../../apps/daemon/src/http/tasks.rs) nevertheless invalidates every two
seconds, and the client refreshes session, projects, discovery and selection. This is source-visible
read amplification, not a measured latency or scale defect.

Propose a narrow bounded topology/availability projection from the existing workflow owner: named
ports, real relationships, authorized identities and explicit off-page references. Inspecting content
can remain separate. Keep snapshot reads and existing SSE unless measurements justify more; a new
graph store or public event journal does not follow. The strongest alternative is the current cached
detail reads if their cost and consistency prove adequate for the intended workflows.

Errors also lose useful meaning. The public envelope has retryability and details, but
[`browser transport`](../../../../../../apps/web/src/lib/transport.ts) drops them. Common admission
failures become generic conflict prose through
[`error mapping`](../../../../../../apps/daemon/src/host/read_model.rs). Preserve existing fields and
return a small set of typed issues with authorized task/port/change locations where the owner knows
them. The client can then preserve the drawing and focus the actual problem without parsing prose.
Ash did not establish a task-create/change producer for every error mapped to HTTP 409; no duplicate
execution incident is claimed from that status mapping alone.

The strongest shared conclusion is to improve the current admission path. A new mandatory
prepare/validate/commit protocol would introduce candidate lifetimes, another recovery state and a
second race; commitment must still revalidate. An optional read-only validation operation can be
considered if needed, but no race-free preview or universal error framework is proposed.

## What genuinely needs new persistence?

Saved unfinished authoring needs a bounded lifetime separate from accepted task revisions. Current
queued revisions retain prior accepted definitions and stop after 128; exact receipts also consume
finite durable capacity. Saving on every editing pause through these commands would eventually
exhaust them. That is an incompatibility with proposed autosave, not an existing autosave defect.

Keep unfinished work under the existing workflow/control owners, with explicit save/recovery and
retention rules. Coalescing writes alone does not supply that lifetime. Explicit save checkpoints
with honestly unsaved local edits remain the smaller alternative, at the cost of more bookkeeping.
Neither raising a cap nor discarding exact replay evidence establishes sustainable editing.

Hazel also revised the earlier completion framing: the existing daemon already accepts new work
after its admitted tasks complete. Display admitted-work status alongside draft presence rather
than making every blank card an unmet runtime obligation. No new Closed/Idle state machine is
required unless people need to seal a workflow. Preserve completed history while showing
**Nothing running · 1 unfinished task** when that is the useful combined observation.

## Questions that determine the next design

1. **Which current behavior deserves repair before new features?** Pause preservation and resource
   contention handling protect an existing user action. Model-unknown wiring and explicit run scope
   remain usability comparisons; they should not become prerequisites for those repairs.
2. **Can the same deliberate change survive incidental progress?** Compare an unchanged queued
   edit across an unrelated progress report with an edit across its own start or source change.
   Removing meaningless conflicts must preserve meaningful ones.
3. **Can a fresh client explain and edit the same connection?** Use data-only and independent-order
   cases, asymmetric outputs and known incompatible media. If a convenience endpoint leaves the
   underlying ambiguity, the contract itself needs to change.
4. **Does the proposed backend addition remove work or add another lifecycle?** Prefer a small
   semantic correction or read projection to a new graph service, hold registry or transaction
   protocol. Retain current cached reads if a representative measurement shows them adequate.
5. **What must remain a human choice?** Model identity, access, content selection and execution
   scope remain visible. Generating keys, recovering receipts and allocating temporary resources
   should not become forms or artificial workflow edges.

## Contributions and limits

**2026-10-23 — Hazel-20261023-a (agent pseudonym; `daemon_lifecycle_debate`).** Traced lifecycle
owners, commands, frontend labels, tests and production worker recovery. Peer objections narrowed
the wait redesign and qualified prepared-resource cleanup; completion can compose with draft
presence without a new lifecycle. Root, not Hazel, executed the diagnostic cases.

**2026-10-23 — Ash-20261023-a (agent pseudonym; `daemon_command_debate`).** Traced guards, errors,
receipts and connection provenance. Revised the guard recommendation after Hazel distinguished
progress from relevant settlement facts; challenged an unnecessary new transaction protocol and
the assumption that capacity deferral can be copied unchanged after preparation.

**2026-10-23 — Elm-20261023-a (agent pseudonym; `daemon_connection_debate`).** Traced adapters,
bindings, publication bounds and project defaults. Challenged moving equal-share allocation into
Rust without changing its meaning. Hazel's contention trace strengthened the project-resource finding;
the default-access preference remains separate from temporary-resource handling.

**2026-10-23 — Willow-20261023-a (coordinating agent pseudonym).** Traced read/observation consumers,
executed the three runtime/store diagnostics and synthesized the exchanges. All reviewers received
the user's UX principles and prior proposals, then exchanged actual objections after their initial
source findings. They share context and provide no independent usability acceptance. Only the
three diagnostic sequences have new executable evidence here; the other cited paths are source
findings and proposed comparisons. The [overview](../../README.md#overview) owns assignment and
further action. This paragraph records the October 23 investigation; see the S01 handoff for the
subsequently assigned production repair.
