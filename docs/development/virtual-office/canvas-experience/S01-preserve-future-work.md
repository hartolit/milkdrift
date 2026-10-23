# S01 — Preserve future work through pause and temporary contention

**Assigned and completed locally 2026-10-24.** See the [handoff](handoff.md) for verified results and
limits. This completed assignment remains under the [side-sprint schedule](README.md).
Deliver a daemon that preserves accepted future work when a person pauses it or its project is busy,
with truthful state and usable controls in the existing production client. This phase repairs the
shared lifecycle before the canvas replacement; it does not wait for that replacement to expose fixes.

Read [AGENTS](../../../../AGENTS.md), canonical documents in their required order, the
[implementation](../../practices/implementation.md) and [documentation](../../practices/documentation.md)
practices, and [workflow](../../workflow.md). Read [ADR 0052](../../../decisions/0052-canvas-authoring-and-method-experience.md#the-daemon-must-preserve-future-work),
the [daemon debate](../whiteboard/discussions/architecture/daemon-frontend-contract.md), and the
[executed diagnostics](../../verification-evidence.md#daemon-lifecycle-design-diagnostics).
Inspect current source, consumers, manifests, tests and Git state before changing them. The scratch
diagnostics establish specific old behavior; replace their role with maintained production regressions.

## Complete one lifecycle boundary

The runtime owns frozen starts, readiness, entry and recovery. The capability host owns preparation,
worker/resource release and native access; storage owns atomic claims. Shared control admits the same
authorized changes from humans, CLI and model actions. Keep these owners and one scheduler.

An explicit human wait persists until authorized Continue. Binding unrelated input cannot clear it;
model-admission rejection waits remain scoped to their source. Missing required inputs keep their
ordinary readiness meaning. Preserve stale-command refusal and exact replay; a delayed Continue must
not accidentally answer a later question. Do not introduce a generic hold registry or branch decisions.

When pause arrives after a start freezes but before external entry, defer the exact accepted start.
Release prepared tokens, safe non-entry resource claims and worker ownership through their real
owners. Do not fabricate Failed/Stopped settlement to obtain cleanup or refund accepted obligations.
Resume must redispatch the same frozen identity, including after reopen. Recovery must not strand an
unentered Running record outside ordinary scheduling. Stop races retain their separate truthful result.
If non-entry or cleanup cannot be established, preserve the actual uncertainty; entered/uncertain work
never receives automatic re-entry. Recheck authority, selected generation and directory identity.

Represent known temporary native-claim overlap as typed contention through storage, host and runtime.
Wait with bounded scheduling work and fair progress rather than terminally failing the task. Keep
uncertain writers' claims protected. Corruption, an altered physical root, incompatible dispatch or
permanent denial must not become retryable busy state. A paused/busy task must not monopolize workers
or prevent unrelated eligible work from progressing. No artificial user-authored order is required.

Expose the reason and contextual Pause/Continue/Stop through existing public reads and production UI.
Name a blocker only when authorized; otherwise show **Project is in use**. Preserve the canvas and
current task. Do not claim that an acknowledged stop proves the external operation stopped. Finish
protocol/client/CLI/generated-browser adoption wherever the typed result changes their contract.

## Start at the real owners

- `crates/runtime/src/tasks/{admission,effects,scheduling,policy,model_actions}.rs`, `tasks.rs`, and
  `crates/capability-host/src/worker.rs`: transitions, recovery, ownership and polling.
- `crates/persistence/src/native_paths.rs`, `adapters/redb-store/src/native_paths.rs`, and
  `crates/capability-host/src/{native_paths.rs,registry/execution.rs}`: typed overlap and cleanup.
- `crates/control/src/tasks.rs`, `crates/control-protocol/src/tasks.rs`,
  `crates/control-client/src/tasks.rs`, `apps/daemon/src/host/tasks/`, and
  `apps/daemon/src/http/tasks.rs`: shared access.
- `apps/cli/src/command/workflow.rs`, `apps/web/src/components/CanvasWorkspace.svelte`, and
  `apps/web/src/lib/{presentation,transport}.ts`: supported controls and truthful diagnosis.

Follow moved paths and actual callers; remove superseded failure mappings rather than adding a second
client-specific interpretation. Preserve current supported stores, frozen definitions, receipts,
project bindings, native/managed protections and cumulative limits. Any required schema adoption must
be checked without resetting a store or weakening its refusal of unsupported history.

## Prove preservation and refusal

Maintain regressions for all three diagnostic sequences: unrelated input during explicit wait;
pause between frozen start and entry; and paused unentered reopen followed by Continue. Count actual
adapter entries and inspect frozen identities, not merely final UI labels. Add pause during successful
preparation, Stop/Continue races, repeated restart, authority/generation loss and entered uncertainty.
Assert resources and worker ownership are neither leaked nor released while effects remain uncertain.

Exercise competing project writers and reader/writer overlap, including aliases, a retained uncertain
writer and an unrelated ready task. A later release permits one exact entry; repeated busy observations
remain bounded. Test permanent refusal separately and verify hidden blocker details stay hidden.
Use production runtime/redb/host and actual daemon commands; fixtures may control clocks, adapter
responses and declared fault boundaries, never perform the scheduling or recovery being tested.

Run affected caller checks and the complete affected targets after focused regressions. Starting
targets are `milkdrift-runtime --test durable_runtime`, `milkdrift-redb-store --test native_path_claims`
and `--test task_workflows`, `milkdrift-capability-host --test registry`, control task-authority cases,
and `milkdrift-daemon --test control_plane`, all with `--all-features`. Discover the exact cases and
record them. Run changed protocol/client/CLI tests, scoped Clippy/rustdoc, public API review and
documentation contracts. In `apps/web`, run check, lint, unit tests, build and affected real-daemon
browser cases through its maintained npm scripts. Inspect the rendered wait/busy/recovery states.

The named [S05 final gate](S05-final-review-and-gate.md) owns the side sprint's full gate under the
explicit schedule. These focused checks must pass now; no known in-scope defect is handed to S05.
Update canonical status, API/operator explanations and evidence with the behavior actually verified.
Handoff tested commits, exact regressions, limitations and ordinary reproduction/use instructions.
Stop ready for [S02](S02-saved-canvas-authoring.md); do not implement methods or mark the main sprint done.
