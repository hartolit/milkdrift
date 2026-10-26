# S02 — Saved authoring and a continuous workflow canvas

**Complete locally 2026-10-26.** The assigned implementation and focused verification are complete;
[handoff](handoff.md) records the evidence and S03 boundary. Prerequisite: S01's lifecycle repairs and focused checks. Deliver the
complete saved-authoring experience selected in [ADR 0052](../../../decisions/0052-canvas-authoring-and-method-experience.md),
through the real daemon and its public clients. This owns the authoring, connection and edit-contract
changes needed for that experience, not a visual shell over the old semantics. S04 owns methods.

Read [AGENTS](../../../../AGENTS.md), canonical product/architecture docs, [README](README.md),
[implementation](../../practices/implementation.md), [documentation](../../practices/documentation.md),
[workflow](../../workflow.md), S01's handoff and the ADR before editing. Inspect source, current
consumers, tests, manifests and Git. Preserve the supplied visual direction/logo, daemon names,
project ownership, working model continuation and current supported stores. Keep one coordinator
for shared files and Cargo jobs. [S05](S05-final-review-and-gate.md) owns this side sprint's full gate.

## Complete the shared authoring boundary

| Responsibility | Starting source and consumers |
| --- | --- |
| Task contracts, admission and readiness | `crates/blueprint/src/task.rs`; `crates/runtime/src/tasks/{admission,inputs,capacity}.rs` |
| Authorized construction and source resolution | `crates/control/src/tasks.rs`; `crates/control/src/tasks/{sources,projects}.rs` |
| Stored authoring, transactions and capacity | `crates/persistence/src/task.rs`; `adapters/redb-store/src/task.rs` and `task/capacity.rs` |
| Public protocol and all callers | `crates/control-protocol/src/tasks.rs`; `crates/control-client/src/tasks.rs`; `apps/daemon/src/http/tasks.rs`; `apps/cli/src/command/workflow.rs` |
| Actual adapter ports and media | `adapters/local-process/src/authoring.rs`; `adapters/model-provider/src/adapter/authoring.rs` |
| Canvas, navigation and transport | `apps/web/src/App.svelte`; `components/{CanvasWorkspace,WorkflowCanvas}.svelte`; `lib/{canvas,transport}.ts` under `apps/web/src` |

Create a discoverable empty workflow and one bounded latest authoring document under its existing
owner. Incomplete cards, sources, intended wires and candidate edits remain distinct from accepted
tasks; saving or arranging them cannot schedule work or consume task admission. Implement the ADR's
conditional checkpoint operation, generation/content recovery and independent position version.
Coalesce saves with one request in flight and one bounded newer candidate. Never mark newer content
Saved because an older response arrived. Conflicts preserve the candidate rather than overwrite it.

Keep exact receipts for creation and Run/Apply. Admission atomically checks the saved authoring
generation, accepts only the reviewed scope and updates its draft-to-task mapping. Preserve remaining
ideas and references to accepted work without duplicating accepted definitions. Lost Run replies
automatically recover the exact command before another submission; save recovery uses its distinct
conditional-write meaning. Keep identity, actor, project and workflow scope through navigation.

Implement the ADR's finite authoring envelope, advertised by Rust, latest-byte accounting, source
protection and replacement headroom. At capacity keep reads/control and the last saved content usable;
offer explicit removal/shortening. Drafts do not expire by age before main-sprint retention. Adopt
actual supported stores transactionally without rewriting frozen definitions or canonical receipts;
unsupported formats refuse before mutation. Update versions, readers, fixtures and generated clients.

## Make the canvas perform the mechanical work

Build the persistent daemon → project → workflow tree, daemon-scoped search, bounded lazy loading
and explicit off-page references. Expanding a branch changes visibility only. Every expanded project
offers New workflow. Selecting a workflow restores its canvas; settings stay beside their owner.
Empty placement opens no inspector and steals no text focus. Clicking opens the right inspector;
selection, editing and wiring preserve camera and nearby work. Keep results first on completed tasks.

Save a deliberately chosen usual model with authoring preferences, visibly inherited by new cards
and overridable per task. Offer the ADR's explicit scoped action to apply it to existing unassigned
draft tasks as well as new cards; highlight those tasks and preserve already selected choices and
accepted definitions. The default-only action affects new cards. No silent replacement follows an
unavailable or changed capability. Connect actual ports directly; resolve genuine ambiguity beside the endpoint.
Model-unknown cards can retain distinctly unfinished wires, which cannot execute. Resolve them only
under the ADR's meaningful-choice/default rule, preserving intent when a contract changes. Text,
captured files and authorized retained results use the same gestures; files grant no native access.

Complete the daemon contract behind those gestures: distinguish independent ordering from data
prerequisites, preserve legacy accepted predicates explicitly, and keep correction/cycle/readiness
checks consistent. Adapter metadata and shared admission reject known media incompatibility while
entry checks actual content. Stable per-output ceilings share the existing aggregate reservation;
adding a connection cannot shrink another promise. Update artifact accounting and correction checks
together. Do not implement these semantics only in TypeScript or silently reinterpret old records.

Run covers the shown pending plan. Choose tasks to run creates a deliberate named scope, including
reviewed necessary new predecessors; inspecting a card changes neither scope nor execution. Incomplete
included work blocks that submission; never omit a hidden subset or split a large command. Existing
predecessors with unapplied edits expose Use current task versus Apply these changes. Mixed changes
use one Apply changes action; started work stays frozen and retains a future-improvement path.

Use affected semantic facts for human edit guards; pure progress, layout and readiness explanations
must not force irrelevant conflicts. Preserve real start/source/authority/project/capability conflicts.
Return bounded typed issues at authorized objects and retain transport retryability/details. Supply a
bounded topology projection without requiring result reads for every card; avoid unrelated reloads
on workflow hints. Retain bounded caching/SSE without inventing an event journal or performance claim.

Remove superseded memory-only authoring/recovery and form paths from ordinary use. Preserve the
programmatic CLI and technical diagnostics. Provide equivalent keyboard/touch connections, accessible
list actions, a narrow-screen drawer, focus return, enlarged-text and reduced-motion behavior. No
dummy method controls, another graph authority, scheduler or branch-decision product belongs here.

## Prove the complete experience

Use production public operations and actual Rust runtime/store owners to establish:

- Empty workflow/cards survive reconnect; two workflows retain separate drafts, model choices and
  positions. Nothing runs on placement/save; useful instructions-only work runs deliberately.
- Drawing before choosing a usual model needs only one scoped selection for unassigned tasks;
  already selected models and accepted definitions remain unchanged.
- Text/file/result connections reconstruct after reconnect. Unfinished wires and incompatible media
  explain repairs locally; authority loss preserves durable work but clears inaccessible client data.
- An unrelated blank idea remains while an explicitly chosen connected subset runs. A predecessor's
  candidate edit is never applied implicitly; start races preserve frozen work and the user's intent.
- Concurrent checkpoint writes, late/lost replies and changed local candidates never falsely report
  Saved or overwrite newer content. Run recovery remains exact after another client's later edit.
- New data/order semantics and older accepted predicates remain truthful through wire removal and
  correction. A second output preserves the first ceiling; actual aggregate overflow still refuses.
- Ordinary progress does not conflict with unrelated edits; real concurrent changes do. Capacity,
  protected-source deletion, scoped denial, cycles and partially loaded topology refuse usefully.
- Current-store adoption preserves accepted records/receipts; restart recovers drafts and mappings.
  Keyboard and narrow-screen journeys retain canvas, focus and all equivalent operations.

Compile affected Rust owners/callers; discover and run focused admission, readiness, storage/adoption,
authority, daemon/client/CLI and recovery suites, including durable-runtime and control-plane cases.
Regenerate protocol types; review changed public APIs; run scoped Clippy/rustdoc and affected examples.
Run frontend check/lint/unit/build and the production browser suite, inspecting desktop/narrow renders.
Run documentation contracts and diff checks. Fix in-scope defects now, without substituting controlled
content for later user evidence. Record actual checks/limits in canonical verification evidence,
update owning docs and the single side-sprint handoff, and commit coherent checked units when assigned.
Stop ready for [S03](S03-early-user-review.md); focused verification does not establish user acceptance.
