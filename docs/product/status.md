# Status

Milkdrift's current ordinary workflow path is a durable daemon-owned task collection exposed through
the CLI, independent JSON requests and a standalone Svelte client. Prompt 03 of the
[implementation sprint](../development/virtual-office/daemon-workflows-implementation/README.md)
extends that path with model-authored future work, parallel research, correction and human batch
editing. The user authorized proceeding while actual user review in prompt 02 remained pending.
That review has now begun: the user reports connecting Studio A and rejects the setup friction and
current visual design. Completion of the substantive task journey has not been reported. The assigned 02a correction
implements shared daemon naming and address/credential setup; the user has not rechecked it yet.
The assigned 02b correction applies the supplied dark visual direction and real logo throughout the
client, with responsive navigation and a desktop task inspector. The user then rejected interaction
usability: too many forms, disruptive navigation and unclear creation of multiple workflows. The
assigned 02c correction implements distinct creation/addition actions, contextual authoring and
workflow navigation. The October 18 user recheck rejects it as still bloated and asks for a canvas,
compact left-side details and direct input connections. The assigned 02d correction replaces the
forms with that canvas and adds the daemon contracts needed by its connections. Its focused
verification is complete; actual user acceptance remains open. The October 19 discussion selects
daemon-owned projects, ordinary host folders and project-level execution settings. The resulting
[02e correction](../development/virtual-office/daemon-workflows-implementation/02e-projects-and-execution-environments.md)
implements the project boundary and has completed its focused checks, with the physical Podman
qualification gap below. The prepared [canvas experience side sprint](../development/virtual-office/canvas-experience/README.md)
now owns the next correction and takes over unimplemented main prompt 04's method responsibility.
Its early user gate must fulfill outstanding prompt 02. S01 implements the lifecycle repair and has
completed its focused verification. S02–S05 remain unassigned.
Side S05 owns its planned full gate; main prompt 07 retains the later final integrated gate. Hosted quality run 227
failed on the earlier `e720bb9`; no later hosted pass is claimed.

## Implemented now

Projects belong to one daemon and identify ordinary user folders. Public clients can attach a
folder, create one child in a permitted parent, rename a registration, change guarded defaults and
remove an unused registration without deleting files. Folder creation persists intent before its
external effect; unresolved ownership remains available for explicit reconciliation or abandonment.
New workflows require a project. Earlier workflows can be associated without rewriting accepted
definitions, results or exact receipts.

New tasks choose reviewed project defaults or complete explicit restrictions. Acceptance freezes
folder identity, access, tools and environment generation. Model-authored additions use the same
resolver. Native tools address host paths; project-aware processes use a pinned working directory,
and the managed adapter verifies an explicit `/project` attachment separately from its owned volume.
Changing defaults never silently changes accepted work. The browser groups projects and workflows,
offers bounded folder choices and keeps model selection separate from tool execution.

The public path creates tasks with client-local aliases, supplies text and captured text files,
starts configured external process/model capabilities, and retains results for later discovery.
Missing or explicitly absent required inputs remain queued. Dependencies and named producer outputs are explicit;
queued definitions can change under generation/version guards, while starts freeze their content,
selection and authority. Stop-requested and uncertain are distinct from proven stopped.

Exact request receipts survive a lost response and restart. Discovery spans authorized workflow
scopes without an initial workflow ID. Compact workflow summaries and paged task summaries keep
large captured inputs/results out of discovery responses. Selected tasks expose frozen evidence,
labelled text previews and complete bounded result downloads.

Opted-in model tasks publish immutable structured responses. Admission refuses model action policies
on other capability categories before accepting work. Control admits their typed batches
through the same guarded transaction as human edits; response admission, exact receipt and allocated
aliases survive restart. Invalid actions retain a bounded diagnostic and wait, or admit one ordinary
feedback task when its explicit policy and remaining allowance permit. Result prose cannot execute
commands. Successful corrections can be adopted for future consumers without changing failed history
or already frozen inputs. Required outputs and explicit settled outcomes distinguish successful work
from tolerated failure.

The capability host runs a configured model with its selected file/process tools and bounded calls.
Current authority and native path checks apply at tool entry. Workflow task, model-batch, execution
time, rejection streak and corrective-lineage limits govern descendants; a human can change future
limits without refunding spending. Completion requires an explicit finish condition and satisfied
required work. Empty ready queues do not imply completion.

The Svelte client connects using an address and credential, learns and retains the daemon's stable
identity, and displays its shared name. A changed endpoint identity refuses ordinary use until the
person deliberately forgets the old connection. Authorized settings and CLI renames share one
durable generation guard and exact actor-scoped receipt; restart cannot restore an old configuration
seed. Other connected clients refresh the name through existing observation, including empty searches.
Disconnected names are explicitly cached. Earlier browser labels are adopted without losing pins.

The Svelte workspace keeps tasks, text/file sources and result connections on one canvas, with a
compact left selection panel, pan/zoom/fit and an accessible list. **New workflow** opens an unsaved
canvas; **Add task** adds a card. One explicit submission accepts the drawing atomically. Instructions
suggest a title and **Model or agent** exposes the configured choice. **Models & agents** lists those
connections and explains the existing operator setup. Model/agent installation remains daemon
configuration, not an in-browser capability installer.

Adapter-owned authoring metadata supplies real input slots, output ports, labels and configuration
generations. A draft result connection adds its actual output declaration and implied prerequisite.
Retained named results may be selected even when the historical producer did not declare them;
shared admission verifies publication metadata and current source authority. Correction redirect
preserves the original media contract and finite output allowance. Bounded inline/shared-input reads
rebuild source cards, while started tasks retain frozen inputs. File tools use the accepted project
binding and explicit access mode independently of captured client files.
Before a local task receives artifact bytes, the host checks its accepted actor's current read
permission against the actual artifact, sensitivity and size. Capability execution permission alone
does not authorize those reads.

The canvas exposes prospective edits, correction/rebinding, stop, removal, wait/resume, completion,
limits and rejected-model-proposal dismissal through contextual actions. The former definition,
composer and JSON batch editor product paths are removed. Exact pending requests and up to eight
dirty drawings remain scoped in memory to the authenticated daemon, actor, project and workflow; navigation never
submits them. Graph/detail caches are bounded and off-page connections remain explicit. A full draft
cache still permits viewing other workflows. Conflicts retain the drawing for a new decision;
lost replies offer exact recovery. Spatial layout owns no scheduling or elapsed-time meaning;
a secondary timeline shows recorded start acceptance and settlement, including time before entry.
The daemon still owns all durable work.

An explicit human pause survives input supply and restart until authorized Continue. A start paused
before external entry retains its accepted definition, inputs and invocation while releasing safe
preparation resources. Known native project contention waits in the same scheduler and leaves room
for unrelated work. Entry intent is exposed separately from start acceptance. Pause, Resume and Stop
act directly in the existing canvas without submitting unrelated drafts or changing selection;
uncertain replies retain exact recovery. Replaying an earlier Continue returns its original receipt
without answering a later question.

The task scheduler uses existing external preparation, final-entry and effect-lifetime mechanisms.
Durable native path claims coordinate declared project reads/writes, including aliases and
uncertain writers. Task storage reserves settlement/control headroom before accepting work.
Independent direct invocation, authenticated execution hosts, managed-resource generation checks,
platform verification and process lifetime protections remain their own boundaries. Cumulative
execution time includes simultaneous open attempts. Entry reserves enforceable adapter durations;
stop requests, uncertainty and restart cannot refund unresolved work.

## Limitations now

The earlier [pause diagnostics](../development/verification-evidence.md#daemon-lifecycle-design-diagnostics)
describe the pre-S01 defects. Maintained runtime/host and public daemon regressions now cover their
repair, including uncertain cleanup and lost storage replies. This does not implement the planned
saved authoring, right inspector or method experience.

Host tools remain trusted processes; native claims do not sandbox them or lock out external
editors. Project-aware process profiles are configured by the operator. Fixed-directory and isolated
profiles retain their separate meanings and are not silently retargeted. Selecting a model does not
install a toolchain. Podman project attachments preserve the pinned source and refuse incompatible
host/runtime prerequisites. The local rootless OCI runtime has refused the fd-backed source mount
before workload entry; successful physical attached-project execution remains unqualified. See
[verification evidence](../development/verification-evidence.md) for the measured boundary and gap.

The user rejects the current screen and navigation as too complex and bloated. Canvas behavior and
model assignment have focused verification, but the usability review and substantive task journey
remain open. The requested tree, blank placement, direct ports and right inspector are developed in
the [selected design](../decisions/0052-canvas-authoring-and-method-experience.md) and prepared side sprint.
Saved authoring and the redesigned interaction are not implemented or accepted through actual user review.
Adding a new model or process still requires a qualified
operator profile, appropriate grants and daemon restart; the browser now exposes that setup path
rather than implying that a connection can be installed from a task's selector.
Drafts are memory-only and are lost on reload
or disconnect; accepted work remains daemon-owned. A new workflow still requires its first task.
Rename receipts have a fixed capacity of 1,024; new renames refuse at capacity while reads and exact
replay remain available.

Saved method templates/expansion, persistent connection administration and configurable rolling
expiry remain later phases. Current SSE
messages are bounded snapshot invalidation hints, not a replayable public task event journal.
Search scans a finite window and returns an explicit continuation; it is not the advanced indexed
inspection surface scheduled later in the sprint.

The current store format is a development cutover. Earlier graph, revision, proposal, controller,
structured-child, prompt-sequence, workflow-publication and evaluated-learning product paths are
retired. There is no migration or compatibility reader for those stores in the new executable.
Internal document format 22 refuses earlier task records lacking the current duration and model
admission facts. Physical schema 21 adopts only exact schema 19 or 20 stores with internal format 22,
adding daemon/project tables without rewriting workflow records or receipts. Earlier formats remain refused.
The expanded Controller preset changes its grant digest: unfinished work bound to an older grant
can be denied before its next external entry. Follow the [upgrade guidance](../operations/daemon.md)
and preserve matching binaries/configurations for old work. Existing stores/services and Git preservation checkpoints remain intact. Generic managed
lifecycle operations lack an enforced total deadline and therefore refuse bounded task/peer entry;
their independent administrative owner remains available and tested.

Models remain external capabilities. No inference engine or model weights run in the core. A
controlled model test proves plumbing within its stated bounds; it does not establish real-model
quality, agent interoperability or paid-provider support. Linux process/managed evidence does not
establish another platform's containment or service behavior. Phone-sized browser checks do not
establish physical-phone or deployed HTTPS operation.
Native model file tools currently require Linux; other platforms refuse that catalog. One model/tool
task supports at most 16 tool calls, bounded UTF-8 file content and a finite provider request envelope.

## Current validation/evidence snapshot

Side S01 passes 60 durable runtime cases, 52 daemon control-plane cases and the complete affected
host/control/client/CLI and scoped storage/peer checks. Frontend check/lint, 115 unit cases and build
pass. Its complete browser run passes 39 cases; the remaining two-client case passes a fresh focused
rerun after correcting only its overall test budget. All three new pause/contention journeys pass,
and desktop/narrow renders were inspected. Actual-binary integration, 30 repository contracts,
scoped Clippy/rustdoc and public API review pass. The
[S01 evidence](../development/verification-evidence.md#side-s01-pause-recovery-and-project-contention)
records the exact coverage and limits; S05 remains the side's full gate.

Prompt 02c passes frontend check/lint, 89 unit cases, production build and all 22 real-daemon
Chromium cases. The added cases cover separate workflow creation, ordinary additions, retained
drafts and focus, task-local context/files, exact recovery, explicit waits, stale generations and
authority loss. The affected Rust fixture passes five unit cases, scoped Clippy and the actual
CLI/daemon journey, retaining eight tasks and three accepted model batches without client
continuation. Desktop and narrow renders were inspected. These focused checks establish the
implemented behavior; the subsequent user recheck rejected the interaction. They do not validate the
current canvas replacement or complete the substantive user journey.

Prompt 02b passes frontend check/lint, 67 unit cases, production build and all 16 real-daemon
Chromium cases, with focused reruns for the final render corrections. Desktop and 390-pixel views
cover task authoring, long names/titles, inspector focus and draft retention, results, batch editing,
parallel work and failure. A separate actual-daemon interruption retains uncertainty and a stop
request without inventing settlement. Enlarged text and reduced-motion rendering were checked. Documentation
contracts pass. These focused checks do not complete actual-user visual or task review.

Prompt 02a passes 41 daemon unit and 41 control-plane cases, the affected storage/authority/client/CLI
suites, all 30 repository contracts, scoped Clippy/rustdoc and default/all-feature API review.
Frontend check/lint, 59 unit tests and production build pass; all 13 browser cases pass, followed by
focused reruns for the final phone wrapping and denied-storage corrections. Actual CLI/public-client
operations verify naming across restart, authorization refusal, exact recovery and preservation of
current task history. This evidence does not replace the pending user recheck.

The connected browser/CLI path executes a trusted Rust process against uploaded Cargo manifests,
closes and reconnects the client, and discovers retained input-dependent results. A separate controlled
model journey creates eight tasks from one planner through three accepted batches, proves overlapping
research, preserves a failed result, adopts its correction and completes a rebound consumer. Binary
restart and transaction fault tests cover different recovery boundaries; neither claims exactly-once
external effects or real-model quality.

Prompt 03 passes all 11 Chromium cases, frontend check/lint, 43 unit tests and production build.
Desktop and 390-pixel phone-sized renders were inspected. Focused Rust suites cover the changed
owners, including 46 durable runtime cases, eight production model-admission cases and all 31 daemon
control-plane cases. The application/connected-task harness checks and 30 repository contracts pass.
Default/all-feature API inventories for six changed libraries were reviewed. Affected-package
Clippy/rustdoc and dependency audits pass; the audit prompted a Rustls patch update.

The earlier prompt-01 correction checkpoint `c9e9809` passed the complete strict static gate.
Prompt 03 used its assigned focused checks; it does not claim that older full-gate result for the
new source. No hosted rerun or completed actual user acceptance is claimed.

[Current contract versions](../reference/versions.md) are checked against their owning constants.

Executed checks and their environment limits belong in
[verification evidence](../development/verification-evidence.md). The sprint
[handoff](../development/virtual-office/daemon-workflows-implementation/handoff.md) records the
tested checkpoints and exact local review instructions. Side S05 owns the next full Rust/frontend
gate; main prompt 07 owns the later integrated gate after the remaining main work.
