# Status

Milkdrift's current ordinary workflow path is a durable daemon-owned task collection exposed through
the CLI, independent JSON requests and a standalone Svelte client. Prompt 03 of the
[implementation sprint](../development/virtual-office/daemon-workflows-implementation/README.md)
extends that path with model-authored future work, parallel research, correction and human batch
editing. The user authorized proceeding while actual user review in prompt 02 remained pending.
That review has now begun: the user reports connecting Studio A and rejects the setup friction and
current visual design. The substantive task journey remains unreviewed. The assigned 02a correction
implements shared daemon naming and address/credential setup; the user has not rechecked it yet.
The assigned 02b correction applies the supplied dark visual direction and real logo throughout the
client, with responsive navigation and a desktop task inspector. Its user review remains open.
Prompt 04 waits for those corrections and the
completed early review. Prompt 07 retains the final integrated gate. Hosted quality run 227
failed on the earlier `e720bb9`; no later hosted pass is claimed.

## Implemented now

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

The client also provides authentication/disconnection, discovery,
search, supplied-file submission, task editing/repair, result inspection and an accessible
left-to-right dependency timeline. Its future-batch editor previews additions, revisions, removals,
input supply, correction/rebinding, stop, wait, finish and limit changes together. Conflicts retain
the draft for review; lost replies retain the exact request identity. Parallel lanes use actual start
and end times, and selected follow-up tasks expose their source admission. Credentials stay in memory;
connections have separate selection, settings and caches. Exact-origin CORS, authenticated fetch
streams, identity checks and snapshot
resynchronization belong to the production daemon/client path.

The task scheduler uses existing external preparation, final-entry and effect-lifetime mechanisms.
Durable native path claims coordinate declared project reads/writes, including aliases and
uncertain writers. Task storage reserves settlement/control headroom before accepting work.
Independent direct invocation, authenticated execution hosts, managed-resource generation checks,
platform verification and process lifetime protections remain their own boundaries. Cumulative
execution time includes simultaneous open attempts. Entry reserves enforceable adapter durations;
stop requests, uncertainty and restart cannot refund unresolved work.

## Limitations now

The dark workspace, real brand, reorganized forms and responsive inspector are implemented, but
visual fit, simpler setup and shared naming still need the user's usability recheck.
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
admission facts. Physical schema 20 adopts only prompt-03 schema 19 with internal format 22, adding
daemon metadata without rewriting workflow records or receipts. Earlier formats remain refused.
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
tested checkpoints and exact local review instructions. The full
Rust/frontend gate remains the named prompt-07 assignment.
