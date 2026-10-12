# Status

Milkdrift's current ordinary workflow path is a durable daemon-owned task collection exposed through
the CLI, independent JSON requests and a standalone Svelte client. Prompt 01 of the
[implementation sprint](../development/virtual-office/daemon-workflows-implementation/README.md)
is the active assignment. Its handoff requires actual user review in prompt 02; completing this
slice does not authorize the remaining sprint.

## Implemented now

The public path creates tasks with client-local aliases, supplies text and captured text files,
starts configured external process/model capabilities, and retains results for later discovery.
Missing required inputs remain queued. Dependencies and named producer outputs are explicit;
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
platform verification and process lifetime protections remain their own boundaries.

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
Existing stores/services and Git preservation checkpoints must remain intact.

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

That checkpoint's three browser cases passed; an expanded seven-case run also covered missing-input
repair, queued editing/stopping and a deliberately dropped accepted POST response. Desktop and
phone-sized rendered layouts were inspected. The checkpoint had focused runtime, protocol,
persistence, native-path, clock, CORS and frontend checks. It was the requested early vertical
milestone, not the final retirement or full-system acceptance.

[Current contract versions](../reference/versions.md) are checked against their owning constants.

Executed checks and their environment limits belong in
[verification evidence](../development/verification-evidence.md). The sprint
[handoff](../development/virtual-office/daemon-workflows-implementation/handoff.md) records the final
prompt-01 commit and exact local review instructions when focused acceptance completes. The full
Rust/frontend gate remains the named prompt-07 assignment.
