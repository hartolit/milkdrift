# Status

Milkdrift's current ordinary workflow path is a durable daemon-owned task collection exposed through
the CLI, independent JSON requests and a standalone Svelte client. Prompt 01 of the
[implementation sprint](../development/virtual-office/daemon-workflows-implementation/README.md)
has completed its assigned review corrections locally: current CI consumers, required-input absence,
and cumulative execution time. The corrected source passes the complete strict static gate and
focused daemon, CLI and browser checks. Hosted quality run 227 failed on the earlier `e720bb9`;
a hosted rerun awaits publication of the local corrections. Actual user review in prompt 02 remains
pending, and these corrections do not authorize the remaining sprint.

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

The Svelte client provides named daemon connections, authentication/disconnection, discovery,
search, supplied-file submission, task editing/repair, result inspection and an accessible
left-to-right dependency timeline. Credentials stay in memory; connections have separate selection,
settings and caches. Exact-origin CORS, authenticated fetch streams, identity checks and snapshot
resynchronization belong to the production daemon/client path.

The task scheduler uses existing external preparation, final-entry and effect-lifetime mechanisms.
Durable native path claims coordinate declared project reads/writes, including aliases and
uncertain writers. Task storage reserves settlement/control headroom before accepting work.
Independent direct invocation, authenticated execution hosts, managed-resource generation checks,
platform verification and process lifetime protections remain their own boundaries. Cumulative
execution time includes simultaneous open attempts. Entry reserves enforceable adapter durations;
stop requests, uncertainty and restart cannot refund unresolved work.

## Limitations now

Automatic model-produced workflow changes, saved method templates/expansion, persistent connection
administration and configurable rolling expiry are later assigned phases. The first client can
submit and edit ordinary tasks; it does not implement those later product journeys. Current SSE
messages are bounded snapshot invalidation hints, not a replayable public task event journal.
Search scans a finite window and returns an explicit continuation; it is not the advanced indexed
inspection surface scheduled later in the sprint.

The current store format is a development cutover. Earlier graph, revision, proposal, controller,
structured-child, prompt-sequence, workflow-publication and evaluated-learning product paths are
retired. There is no migration or compatibility reader for those stores in the new executable.
Internal document format 21 also refuses earlier task records lacking duration reservations and
watermarks. Existing stores/services and Git preservation checkpoints remain intact. Generic managed
lifecycle operations lack an enforced total deadline and therefore refuse bounded task/peer entry;
their independent administrative owner remains available and tested.

Models remain external capabilities. No inference engine or model weights run in the core. A
controlled model test proves plumbing within its stated bounds; it does not establish real-model
quality, agent interoperability or paid-provider support. Linux process/managed evidence does not
establish another platform's containment or service behavior. Phone-sized browser checks do not
establish physical-phone or deployed HTTPS operation.

## Current validation/evidence snapshot

The first connected checkpoint, local commit `37aa8c4`, executed an actual trusted Rust process
against uploaded Cargo manifests. A real browser submitted a task, disposed the entire client,
reconnected with endpoint/credential only, discovered the task and displayed its retained report.
Changing from the root manifest to the runtime manifest changed the reported package, targets and
dependencies. CLI submission and exact replay used the same daemon owner. This establishes useful
input-dependent work, separately from the controlled model fixture.

The review correction passes all eight Chromium cases, frontend check/lint and 26 unit tests against
the unchanged production Svelte build. Desktop and phone-sized renders were inspected. Focused
checks include 26 durable runtime tests, 40 daemon library tests, 31 control-plane tests, two real
peer-daemon tests, independent host/managed protections and all 30 repository contracts. Required
absence and time-reservation regressions fail when their respective fixes are removed. The current
CLI/public-client evidence and five external-evidence fixture tests pass; controlled fixtures remain
non-qualifying. Default/all-feature capability and runtime API inventories were reviewed.

Clean commit `c9e9809` passes all 33 strict static checks with zero findings, including workspace
default/all-feature compilation, warning-denying Clippy/documentation and workflow/security scans.
This is local evidence; no corrected hosted run is claimed.

[Current contract versions](../reference/versions.md) are checked against their owning constants.

Executed checks and their environment limits belong in
[verification evidence](../development/verification-evidence.md). The sprint
[handoff](../development/virtual-office/daemon-workflows-implementation/handoff.md) records the tested
prompt-01 commits and exact local review instructions. The full
Rust/frontend gate remains the named prompt-07 assignment.
