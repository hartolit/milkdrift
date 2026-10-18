# Connected product acceptance

This is the executable acceptance account for the [implementation sprint](README.md), not evidence
that the behavior exists. [ADR 0051](../../../decisions/0051-daemon-task-workflows.md) owns the selected
semantics and defaults. Exercise the same growing workflow throughout phases 01–07; fixtures may
control external models, tools, clocks and interruption points, but the production daemon must own
admission, scheduling, expansion, authority, persistence and recovery.

The initial acceptance expectations were derived by a separate reviewing agent from the user's
account before seeing the chosen design. This document subsequently adopts ADR 0051, including its
complete nested-template snapshot at root activation and capacity-bounded lifetime receipts. Its
author's later review of the prompts is a nonauthor prompt review with that disclosed design exposure.

## Fixture and authority setup

Use fresh, explicitly named temporary workflow stores and project roots. Keep old stores, real
projects and running services untouched. Configure daemon A as workflow owner and B as a tool host;
the browser and CLI use A's public operations. Use separate observer, workflow editor, library owner
and connection administrator identities so the tests can distinguish permissions. A test identity
may combine roles only in a separate convenience scenario.

The principal brief is: “Develop a Rust-based approach to learned animation. Compare feasible model,
runtime and training-data approaches, record their limits, then create a small project and review it.”
Supply it as the workflow's named `brief` input. Configure a model and bounded permitted file/process
tools; an external endpoint without a declared allowance must refuse entry. Display the effective
ADR defaults before submission. Fixtures use a smaller bound only when testing its exhaustion.

Use controlled model responses selected by their declared input cases, not by hidden workflow state.
They may return result text, tool requests and structured action batches. The harness may pause an
external response or interrupt a process at a durable fault boundary. It must not create the research
children, apply model batches, make composition ready, populate results directly, expand templates,
reconcile uncertain commands, or carry browser state across a fresh-client test on the daemon's behalf.

Names such as `plan` and `research-runtime` below are readable test aliases, not chosen production IDs.
Discover and retain daemon-issued identities from public responses. Project paths are relative to
the authorized project root. Immutable result references and captured inputs accompany ordinary
files; a path's continued existence does not prove the bytes an earlier task used.

## First working journey: phases 01 and 02

In phase 01, open the production Svelte client from its independent origin, connect to A, authenticate
and submit an ordinary task with the supplied brief and configured model. Ask it for a concrete Rust
approach, tradeoffs and a next experiment. Inspect its actual immutable text result, selected inputs,
status and timestamps through the GUI. A real permitted coding-agent profile or configured model can
provide meaningful work; a recorded fixture is separate plumbing evidence.

At least one early task must produce useful input-dependent output through an authorized real model,
coding agent or trusted Rust process. For example, inspect a supplied real Cargo project and report
its actual targets/dependencies. Change the supplied input and inspect the changed result in Svelte.
An echo, canned model response or prewritten report cannot satisfy this acceptance requirement.

Close the client while the task runs. A must finish or expose its truthful failure without a browser
worker. In a new browser context with empty storage and no known workflow/task ID, enter A's endpoint
and scoped credential, list accessible workflows, select the work by title and inspect its result.
No local request recovery file is required. Lost submission replies can be recovered through the
daemon's bounded discovery and exact request-replay path without starting duplicate work.

Phase 02 is an actual user review of this production journey. Let the user submit their own meaningful
instructions and reconnect, and ask them to identify the result, its state and an available repair
action. Record observed behavior and concrete requested corrections, then implement and verify those
corrections. An agent-driven browser run or a fixture demonstration does not substitute for the user
checkpoint. Include the genuinely input-dependent result above in that review. Missing user
participation leaves that checkpoint open.

The October 17 partial review adds the prepared [02a](02a-daemon-connection-identity.md) and
[02b](02b-visual-design-and-brand.md) corrections. Before continuing to methods, have the user connect
with only an address and credential, recognize the daemon's shared name, and observe an authorized
rename in another client and after reconnect. Exercise wrong-installation refusal separately.
Recheck the real logo and the supplied visual direction in the working task/timeline/inspector,
including a narrow layout. The later [02c](02c-workspace-and-task-creation.md) correction also requires
the user to create two workflows, switch between them and add a second task to the first without
coaching through technical fields. Then finish the substantive journey above. These remain acceptance
requirements; the initial connection report and mockup are not evidence that the corrections work.

## Growing the same workflow: phases 03–05

Phase 03 extends the retained production task/client path with model-produced actions. Phases 04 and
05 add method and remote/retention behavior to it. For combined acceptance, create a fresh instance of
the same animation workflow and run the following graph without manually launching each next step.

| Task/use | Declared inputs and readiness | Required output and purpose |
| --- | --- | --- |
| `plan` | Supplied `brief`; configured model and permitted tools | A bounded plan result plus one action batch adding the three research tasks and `compose` below. |
| `research-models` | `brief` plus its inline model-comparison assignment | `research/models/findings.md` and immutable `findings` text result comparing model approaches. |
| `research-runtime` | `brief` plus its inline Rust/runtime assignment | `research/runtime/findings.md` and immutable `findings` result describing Rust integration and limits. |
| `research-data` | `brief` plus its inline data/animation assignment | A deliberate tool failure, retained diagnostic and failed task outcome; no fabricated findings. |
| `compose` | Explicit settled-outcome bindings for all three research tasks | `design/approach.md`, immutable `approach` result and a batch adding `research-data-fix`, `setup`, `review` and `remote-check` with declared references. |
| `research-data-fix` | `brief`, failed `research-data` outcome and relevant successful findings; `addresses` the failed task | `research/data/findings.md`, immutable `findings` result and explicit adoption as the failed research obligation's correction. |
| `setup` | Queued method identity; `project_name`, `brief`, `approach` bindings; required-success dependency on `research-data-fix` | Expansion of the current saved project-setup method into this workflow's ordinary tasks and named `project_manifest` export. |
| `review` | Required-success `setup.project_manifest`, `approach` and repaired data findings | `reviews/initial.md`, immutable review result and a batch adding a linked corrective task and verification task. |
| `remote-check` | Required-success `setup.project_manifest` and an explicit command/environment binding | A command on B, its durable remote-operation association and eventual immutable report or unresolved outcome. |
| `correct-project` | Review result and captured project files; `addresses` the review finding | Corrected project files and immutable change report; no changes to earlier task history or the shared method. |
| `verify-correction` | Required-success correction output plus captured corrected files | `reviews/verification.md`, a verification result and a finish request after required obligations are satisfied. |

The controlled research model responses request ordinary file tools to write their findings. All
three research tasks can run concurrently in their own writable directories. A bounded controlled
tool failure makes the third fail. Composition starts only once all three outcomes settle and sees
the failure explicitly. A separate required-success consumer of the failed output remains blocked
until the editor rebinds it to an accepted correction; it must never receive empty text as success.
Declare the research tasks' read/write roots and prove sibling-directory concurrency alongside
overlapping native-path serialization. Test path aliases, an ancestor-root writer and an uncertain
writer across restart; the production claim owner must prevent conflicting entry until settlement
or authorized reconciliation. These claims coordinate declared work, not out-of-band native writes.

The compositor's batch names explicit dependencies and input references. The daemon checks the
whole batch before acceptance and assigns the new IDs. Replay the exact accepted batch after losing
its reply: it returns the original result and creates no additional tasks. Interrupt between saving
the model response/pending admission and processing its batch: recovery processes that same record.
A response whose actions are rejected remains inspectable with the rejection reason.

Challenge a simultaneous human/model edit. Both begin from the same edit generation; one wins and
the other receives the current generation and affected IDs. No changes from the rejected batch apply.
An authorized actor refreshes and submits a reviewed replacement with a new exact request identity.
Also race a future-task edit against start: either the edit governs the durable start snapshot or it
is refused because that task has started. Inspect the frozen instructions, bytes/digests, capability
generation and authority basis through the public result/task inspector.

The first review reports a deliberate concrete defect in the generated project, such as an omitted
error case in its input parser. Its structured action batch adds `correct-project` and
`verify-correction`; tools perform the actual writes. The daemon continues within the common account.
Where a correction satisfies a failed required task, use the explicit adopt-correction action with
compatible outputs and named pending consumers. Assert that obligation redirection preserves both
histories; refuse adoption cycles, competing substitutes and changes to started consumers.
A final finish request may be accepted while required work remains, but the workflow cannot report
completion until the required results, remote operation and all pending admissions are settled under
ADR 0051. Rejected/no-progress responses, repeated correction failures and exhausted limits exercise
the selected stop/wait policy with visible reasons and authorized continuation actions.

## Method edit and interrupted activation: phase 04

Create a daemon-wide `project-setup` method through the library owner's public path. Its parameters
are `project_name`, `brief` and `approach`; ordinary tasks create `Cargo.toml`, `src/main.rs`,
`README.md` and `docs/architecture.md`, then validate the project and export `project_manifest`.
Use a nested local method for one reusable documentation task. This is test library content, not
special animation behavior in Milkdrift. Consumers can use it across workflows without edit rights.

Hold `research-data-fix` at its external response so `setup` remains queued. The owner saves a new
root body that adds a documented parser error case and saves a new nested documentation body.
Rename the method without changing its identity. The queued use still refers to that identity.
Release the external response and let the daemon make `setup` eligible.

At activation, assert that the consistent current root and all reachable nested bodies are selected.
Interrupt after the accepted expansion record is durable but before the flattened tasks/exports/
expanded marker commit. Reopen A and recover through its production startup path. The expansion
uses the recorded bodies and deterministic mapping, inserts each child once, and exposes no runnable
partial batch. Save a later library edit after recovery; the recovered tasks still show the accepted
content. A second separately queued top-level use must select the later current content.

Race removal/rebinding and stop against expansion acceptance. Removal before acceptance creates no
children; after acceptance the use is expanding and rebind/remove refuses. A workflow pause permits
materialization but no child entry. An accepted stop survives restart and materializes children as
stopped before entry, releasing unused reservations only after the commit. No test helper supplies
the stop state or releases the production reference holds.

Inspection must explain the selected root/nested origins and each copied task. Local edits to
unstarted copied tasks leave the library body unchanged and preserve valid exports/dependencies.
A forbidden edit to a started copy is refused. The group has no separate child workflow, execution
account or scheduling engine. Dependents of `project_manifest` wait for its actual producer.

Exercise an incompatible owner edit that adds a required parameter while a use is queued. Saving
reports affected uses; activation blocks with expected/supplied bindings, and the user can rebind,
replace the use or fix the template. Missing required inputs receive no silent defaults. Refuse
deletion while queued uses or saved nested references require the method. Deliberately remove or
replace those references, then delete it and retain any protected accepted snapshot. Refuse cycles,
depth greater than eight and expansions exceeding 64 flattened tasks or remaining workflow capacity
without committing partial copies. Also exceed 256 nested-reference occurrences using empty methods,
and exceed the 1 MiB canonical snapshot/import/export bound with large content. Refuse before loading
or flattening beyond these bounds; a small flattened task count does not exempt the input. Export
and import into another daemon create local identities and rewrite nested references without
transferring credentials or producing a live remote dependency.

## Remote operation and client independence: phase 05

B's administrator issues a finite inbound relationship credential for A; A's connection administrator
imports B's endpoint, expected identity and credential through A's public path. The GUI must explain
the separate authorities. Another authorized client of A sees the saved connection and advertised
tools without re-entering topology or obtaining B's secret. An observer may inspect only authorized
summaries; neither discovery nor tool use grants connection administration or direct B login.

Select B's tool using connection ID plus remote capability ID through A's public/model catalog.
Offer the same display name from a second host and prove correct selection in both Svelte and model
proposals, without private registry injection. Freeze the descriptor and connection generation at
start; a stale/revoked offer refuses entry and refresh cannot silently select another host. Secret
import retry after lost response reuses its staged identity/handle; changed secret bytes with the
same key conflict, and ordinary receipts/logs/reads contain neither secret nor usable credential.

The remote check runs a declared command against the prepared project in a B-owned workspace. Use
ordinary authorized file/tool transfer to prepare the remote inputs; local paths are not magically
remote files. The task receives a remote result/artifact through the owning public operation path.
For deterministic evidence, the controlled command appends one marker to a temporary output file
before its response is interrupted. This distinguishes “may have executed” from “did not execute.”

Break the connection after B accepts the command, close the phone client and restart A. A retains
the exact remote operation association and uncertainty; no fresh unsafe execution occurs. A new
client discovers the workflow and sees the unresolved operation. Restore connectivity and query/
replay that accepted operation. Assert one marker, the original operation identity and a truthful
terminal result. If the host cannot establish an outcome, retain uncertainty and require authorized
reconciliation instead of claiming success. Unreachable health is never execution evidence.

Request cancellation during a separate held command. Display stop-requested until B provides
termination evidence; a timeout leaves uncertainty. Replacement work is new linked work and cannot
repeat an unsafe effect merely because cancellation was requested. Rotate/disable/revoke the saved
connection, restart A and assert no new entry under the disabled or revoked authority. Accepted
operations retain their original association for query/cancel. Refuse connection removal while any
operation remains unresolved; re-enabling a connection does not replay commands automatically.

## Browser and retention assertions

Run browser evidence against the actual standalone Svelte build and daemon. Test approved-origin
preflight, authenticated fetch and fetch-based SSE; test denied origins, missing/revoked credentials,
cross-actor cursors and an expired cursor that recovers by obtaining a new authorized snapshot.
Native HTTP tests alone do not establish browser operation. Verify that tokens do not appear in
URLs, logs or persisted browser storage, and that disconnect/revocation clears inaccessible caches.
Switch between A and another daemon with similar workflow names and confirm isolated credentials,
settings, results and pending actions. Wrong installation identity must prevent mistaken attachment.

For the phone deployment lane, use operator-provided browser-trusted HTTPS and the configured private
listener. Check actual proxy stream flushing and reconnect, without certificate bypass or mixed
content. An unavailable trusted route is an explicit evidence gap, not a browser pass. Reconnect from
a fresh authorized browser without known IDs, search by the retained term “Rust runtime,” and open
the earlier findings. Search, list, inspector and stream must enforce the same daemon/actor scope.

The timeline shows titles, descriptions, actual start/end times, dependency links, parallel lanes,
queued/running/completed/failed/stopped states, stop requests and uncertainty. Queued work has no
invented execution timestamps. Method-derived copies retain origin links; corrective work links to
the exact findings it addresses. Search excludes inaccessible results and reports paging/truncation/
expiry without revealing hidden record counts.

Advance a controlled durable clock across the configured completed-history window. Unneeded old
details and artifacts expire; unfinished work, unresolved remote operations, pending inputs and
their transitive protected references survive. Expired result fetches report expiry. Ordinary
Markdown/Git project files remain untouched. Clock rollback must not force premature collection.
Exercise a small artifact capacity with all contents protected: new admission/upload refuses rather
than deleting required data. Exercise the compact receipt limit independently: exact accepted keys
still replay or conflict, and capacity exhaustion refuses new writes instead of forgetting keys.
The default 30-day history window is not permission for unlimited lifetime receipt accumulation.

Fill ordinary receipt capacity with a task and remote operation still active. Their reserved space
must permit result/uncertainty recording, settlement and reference release. Use server-issued action
identities to stop/revoke/reconcile and replay those requests without allocating unbounded new
receipts. Restart and verify those facts. The reserved administration slot must still admit an
authorized capacity increase and reserve its successor; ordinary admission cannot spend these slots.
Inspect the distinction between expired display history and command text retained for exact recovery.

## Different project and review/acceptance boundaries

Repeat the growth, method and corrective path with this brief: “Create a Rust CLI that imports a
public transport timetable and answers route queries.” Use a checked-in small public-format fixture
as supplied input; no live data service is needed. Research topics become input format, route search
and CLI usability. Outputs use `research/format/`, `research/routing/` and `research/cli/`; composition
writes `design/approach.md`. Supply a different project name and content to the same method identity.
Its copied tasks create an ordinary Rust CLI project, and review asks for a missing malformed-row
case. No animation title, path, task ID, model answer or accepted output is hardcoded into production.

Phase 06 is the second actual user review. The user performs the combined animation journey in the
production client, observes the failed research and automatic repair, edits a queued template, inspects
the expansion origin and remote uncertainty, and reconnects/finds an earlier result. Record behavior
problems, repair them across the daemon and relevant clients, and rerun affected cases. Review covers
understandability and control as well as visual presentation. The second brief challenges reuse; it
does not establish general model quality.

Phase 07 owns the full gate and final combined test-and-repair. Keep evidence separated into:

- Repeatable controlled-model/tool tests proving daemon decisions, browser behavior and failure recovery.
- Actual user observations and repairs proving the reviewed product journey was usable in that setting.
- Any authorized real-model run, with configured endpoint, allowances, inputs, outputs, failures and
  limitations. Controlled responses prove no claim about the model's judgment or output quality.
- Actual remote/HTTPS/Podman evidence, distinguishing loopback transport, real separate hosts and
  supported managed-resource behavior. Do not infer physical-host or power-loss coverage from a fixture.

Retain tested source identity, commands, discovered test cases and outcomes under the existing evidence
conventions. Fix concrete findings, then run the full gate and combined journey on the resulting code.
Passing isolated phases, an attractive screen, a fabricated successful state or an unavailable review
checkpoint is not completed product acceptance.
