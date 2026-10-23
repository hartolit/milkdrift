# S04 — Define, reuse and improve methods on the canvas

**Prepared, not assigned.** Prerequisites: S01–S02 complete and the actual-user gate in
[S03](S03-early-user-review.md) satisfied. This phase absorbs the complete former
[main prompt 04](../daemon-workflows-implementation/04-local-methods.md), including backend, clients,
rights, expansion, recovery and cross-daemon content copies. Do not leave a second method assignment.

Read [AGENTS](../../../../AGENTS.md), canonical documents in order, the
[implementation](../../practices/implementation.md) and [documentation](../../practices/documentation.md)
practices, [workflow](../../workflow.md), [side schedule](README.md), S03's tested handoff and the
[main journey](../daemon-workflows-implementation/journey.md). Follow
[ADR 0052](../../../decisions/0052-canvas-authoring-and-method-experience.md#methods-are-reusable-procedures-in-this-same-workspace)
with [0051's expansion contract](../../../decisions/0051-daemon-task-workflows.md#methods-and-activation).
Inspect current code, consumers, tests, manifests and Git; do not treat old prepared APIs as existing.

## Deliver reusable work through the reviewed workspace

A person can select useful task definitions or start a clearly labelled method canvas, define its
input/result boundary physically, save it, and use it in another project workflow without recreating
bindings. Preserve tree, right inspector, camera, contextual choices and accessible equivalents.
Derive boundary mechanics through shared contracts; expose meaningful source/result/start-after
choices without parameter syntax, identifiers or schema forms. Explain unsupported extraction locally.
Copy procedure definitions, not private results, historical failure/adoption links or inferred loops.
Project-specific paths require an authorized consuming-project meaning; never silently relocate them.

Keep incomplete library editing in S02's bounded conditional authoring owner. Update installs a
complete validated usable body through an exact-receipted command; autosave never changes queued uses'
template. A contextual chooser inserts a live reference with a read-only current preview. Editing
the library is explicit; local future-copy edits and deliberate owner updates stay distinct. Try a
method by deliberately running an ordinary use in a chosen project; preview performs no execution.
Support comparison of local improvements for a reviewed owner update without automatic promotion.

## Complete the shared library and activation owner

Blueprint owns template/interface validation; control owns authorized library/use/import/export
operations; persistence/redb owns saved bodies, reference holds and atomic expansion; runtime owns
eligibility/materialization. Protocol, public client, daemon, CLI, Svelte and model discovery all use
those owners. Extend `crates/blueprint/src/task.rs`, `crates/control/src/tasks/`,
`crates/runtime/src/tasks/`, `crates/persistence/src/task.rs` and `adapters/redb-store/src/task.rs`
with narrow method modules. Follow `crates/control-protocol/src/tasks.rs`,
`crates/control-client/src/tasks.rs`, `apps/daemon/src/host/tasks/`,
`apps/cli/src/command/workflow.rs` and `apps/web/src/components/CanvasWorkspace.svelte` through callers.
No registry service, child scheduler or frontend expansion engine supplies the feature.

Implement daemon-wide identities, names, owner/use rights, save guards and content digests. Discovery
and affected-use inspection disclose only authorized information. Refuse deletion while queued uses
or saved nested references need the method; report protected references without exposing hidden work.
Renames retain endpoint identity. Unknown/missing parameters, incompatible or removed exports and
unavailable capabilities yield localized repair while retaining the queued use and its connections.

Apply ADR 0052's root-input/order eligibility and reviewed execution-choice rules. Compatible saved
content follows live until acceptance; different models/generations, wider access or changed location
require refreshed choice. Project settings use the shared reviewed-generation guard. Templates grant
no access and carry no implicit author folder. At accepted expansion, retain effective settings,
bindings and the consistent root-plus-nested snapshot; each ordinary task start still freezes bytes.

Persist exact bodies/digests, origin chain, deterministic identity map, reservations and reference
holds at acceptance. Atomically commit every copied task, export map, receipt and expanded marker;
no partial runnable copy exists. Restart completes the accepted snapshot, never newer content or a
second copy. Save/delete/use/edit/start races serialize through their real owners. Apply 0051's
remove/rebind refusal after acceptance, paused materialization and Stop-before-entry treatment.
Enforce cycle/interface/capacity checks and all nested bounds incrementally, including empty uses;
the 64-task expansion bound is distinct from ordinary 32-action batches and shares workflow limits.

Derive export release dependencies once under ADR 0052, including correction satisfaction and explicit
early results. Preserve stable outer wires through eligible local edits; started copies remain frozen.
Show actual origin, selected content, parameter bindings, exports and local changes. Group status
describes copied obligations, not substantive approval or arbitrary dynamic descendants. Permitted
copied planners may still add ordinary workflow work; retain its cause without a second budget/runtime.

Export/import complete bounded nested bundles under content-read authority. Allocate new local IDs,
rewrite internal references and retain provenance. Transfer no secret, grant or live host relationship.
Resolve local models/tools explicitly beside affected tasks; unresolved mappings block use. Reimport
creates another copy; updating an existing library method is a separate guarded reviewed edit.
Preserve supported stores, old receipts, frozen work and protected inputs with checked schema adoption.

## Verify actual use, interruption and repair

Exercise owner save versus consumer denial; interface rename versus replacement; all root-input/order
readiness cases; project/model-generation/access changes; local edits versus library bytes; default
and early export release; negative review without false approval; and deletion/reference races.
Reopen the real store before acceptance, after snapshot acceptance, during materialization and after
commit before reply. Count children/entries and verify exact replay/conflict, origin and exports.
Cover Stop/remove/rebind/pause races, changed nested content before/after acceptance, cycles, depth,
occurrence/byte/task/account limits including many empty nested references, and retained holds.

Run the main animation setup/correction and timetable CLI variations with the same method body.
Controlled models choose a method through ordinary catalog/context and submit a use; fixtures never
expand it. Browser cases create/extract/edit a method, delay activation, change its interface, repair
the connection, inspect materialization and edit a still-unstarted copy. Prove a second-daemon import
with local mapping and no transferred authority. Product source contains no fixture-specific brief.

Compile all affected callers; run blueprint/control/runtime, persistence/redb, protocol/client/CLI and
daemon control-plane suites, discovering and recording added method cases. Extend existing durable
runtime and `tools/evidence/tests/connected_tasks.rs` production journeys. Run scoped Clippy/rustdoc,
default/all-feature public API review, generated-wire checks and documentation contracts. Run frontend
check/lint/unit/build and method browser cases; inspect desktop/narrow/keyboard interaction. Fix all
in-scope findings now. [S05](S05-final-review-and-gate.md) owns the scheduled full gate and final review.

Handoff coherent tested commits, actual evidence, operator instructions, schema/protocol adoption and
remaining qualification limits; update canonical status/architecture/reference docs. Stop ready for
S05. Main remote tools, rolling retention and the later combined review/gate remain unfinished.
