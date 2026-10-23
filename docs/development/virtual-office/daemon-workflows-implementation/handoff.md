# Daemon workflows implementation handoff

Prompt 03 is complete locally. The daemon accepts model-authored future tasks through the same
guarded batch transaction as humans, runs bounded model/file/process tools, schedules parallel work,
preserves failed history and adopts corrections for future consumers. CLI and Svelte expose the
complete batch, limits, wait decisions, frozen inputs and cause of follow-up work.

The user authorized proceeding with actual prompt-02 review still pending. That review has now
begun and produced the corrections below. Agent browser checks do not complete it. Subsequently
assigned 02a implements address/credential connection, learned identity pins and a shared daemon
name with authorized rename and exact recovery. Subsequently assigned 02b applies the supplied dark
visual direction and exact logo across the maintained client. The subsequent user recheck rejects
the interaction design: excessive forms, disruptive navigation and unclear workflow creation.
[02c](02c-workspace-and-task-creation.md) was subsequently assigned and implements those corrections.
The October 18 user recheck rejects that interaction as still bloated and asks for a canvas with
left-side details and direct input connections. Assigned [02d](02d-canvas-and-direct-authoring.md)
implements the replacement and its necessary daemon support; its focused verification is complete. Naming acceptance
and the substantive task journey remain open. The October 19 project/environment discussion produces
prepared [02e](02e-projects-and-execution-environments.md), subsequently explicitly assigned for
implementation. It adds daemon-owned project registration, guarded execution bindings and the shared
browser/CLI/public-client path. Main 04 is now superseded by the prepared canvas side sprint;
side S01 is complete locally, with its [handoff](../canvas-experience/handoff.md) owning the lifecycle
checkpoint. S02–S05 and main 05–07 remain unassigned. Side S05 owns its full-system gate; prompt 07 owns
the later integrated main gate. No push, history rewrite, external service change or paid model use
occurred.

## Actual user review and corrections

On October 17, the requesting user reported, “I added Studio A,” using the production Svelte preview
at `http://127.0.0.1:4173/`. The local source checkpoint was `9fe2915` with the completed prompt-03
build. This is the user's report of a successful connection, not an agent-observed click trace.
The desktop browser/client environment is known; its browser engine and physical-phone use were
not established. No task brief, uploaded input or useful result has yet been reported in this review.

The user found connection setup too complicated, questioned the local name and installation identity
fields, and requested a daemon-owned name whose changes persist across client GUIs. They strongly
disliked the current UI, supplied an earlier mockup as a rough visual guide while explicitly noting
its outdated features, and pointed out that the real logo was missing. They asked for corrective
prompts to keep the sprint aligned, rather than immediate implementation.

At that review, source inspection confirmed browser-local profile labels, manual identity entry and a literal `m`
brand mark. The installation identity is the daemon's stable `host_id`, not a device-registration
field. The authenticated session already returns it. Later phases previously included broad review
and repair but no explicit assignment for this visual direction or browser naming change. Phase 05
owns A-to-B execution-host relationships, a separate boundary.

After the 02b delivery, the user reported that the application is still too complicated: too many
input boxes and forms, poor navigation that shifts them away to add work, and no apparent way to
create multiple workflows. They questioned whether the design was thought through. The available
preview implementation is `90ce1ff`, handed off at `2840937`; this feedback is the user's report,
not a captured click trace or confirmation of which assets their tab had reloaded. It does not
establish acceptance of the colors, naming recheck or a completed task/result journey.

Inspection of the pre-02c source confirmed that **+ New task** on discovery called workflow creation
with one task, so the backend already supported multiple workflows. The selected-workflow screen exposed
ordinary addition only through **Edit future batch**, a full definition, alias, staging and JSON
preview. Discovery replaced selected work instead of offering a persistent workflow switcher. The initial
composer also always declared a required brief even though the daemon does not universally require
one. Runtime admission refuses empty workflows. These findings distinguish an undiscoverable,
burdensome client path from missing multi-workflow support. The selected correction and its tradeoff
are owned by [ADR 0051](../../../decisions/0051-daemon-task-workflows.md#browser-workspace-interaction).

After the 02c delivery at `f71be73`/`4f21982`, the user reported that the interface is still
“way too complicated and bloated.” They want the main workspace to be a canvas beside navigation,
clicking between workflows/tasks while selected information appears on the left instead of taking
over the view. They propose dragging input connectors to tasks and explicitly reject manually
authoring “Input binding name,” “Input policy,” “Supplied input name,” “dependency task identity or
batch alias,” “new output name” and “project path,” among other fields. This is direct user feedback,
not an agent-observed click trace or proof of the exact assets loaded in their tab. No completed
substantive output journey, naming acceptance or visual preference acceptance was reported.

Inspection of the pre-02d source confirmed those fields in `TaskDefinitionEditor.svelte`, reached through the
compact composer's advanced section and the future-batch editor. `App.svelte` combined a
workflow page with a separate composer; task details occupied a right-side or stacked inspector.
The previous correction reduced the first form but retained the protocol-shaped editing model.
The user's new left-panel requirement supersedes the older mockup's inspector placement.

Two delegated source/interaction reviews, with the conversation and proposed direction available,
challenged the ordinary and demanding paths. Inspection of actual owners identified required port,
named-result and queued-input read projections, the declaration-only output binding restriction,
native-path selection requirements and the race if accepted tasks start before wiring finishes.
The selected design resolves these in the existing owners; the ADR owns the rules and tradeoffs.
This review is planning evidence, not a working canvas demonstration or independent user acceptance.
The selected 02d removes the product schema editors, retains their useful actions through direct
selection and keeps draft graphs local until one explicit guarded submission. No production code,
store, running service or dependency changed in that planning response.

The user subsequently assigned 02d and added that selecting or assigning a model/agent to a task,
and adding a connection, are unclear. The implementation makes **Model or agent** a visible task
choice and adds **Models & agents** navigation with an explicit setup path. Current connections
remain operator-configured daemon profiles; this change does not add a dynamic installer, inference
or a new provider family. The setup guidance distinguishes these facts from selecting an existing
connection. No actual model/agent endpoint or paid invocation was supplied or exercised.

On October 19, the user asked where workflow work and generated project files live. They proposed
daemon-owned projects containing workflows, using the attached desktop-app sidebar as an organizational
reference, and questioned whether Podman-managed persistence conflicts with user-controlled folders.
They approved preparing the correction after the explanation that project files can remain in ordinary
host folders while Podman supplies an optional execution environment. This selects the product/design
direction; it is not a report that the current UI supports projects or an instruction to implement 02e.

Current source inspection found no project association in `WorkflowRecord` or workflow creation;
session `project_roots` only projects filesystem grants. Process working directories are fixed by
their profiles, native file tools address host paths, and managed workers accept one exact owned
volume at `/workspace`. A separate delegated reviewer, with the conversation and proposed direction
available, traced those boundaries and challenged the draft. The correction now requires one actual
project binding across file/process/container tools, distinct inheritance versus empty permissions,
exact replay before mutable-default resolution and honest directory-creation recovery. It preserves
trusted-process limitations and excludes managed lifecycle commands from model tools. This is source
and planning review, not executed project/Podman evidence or independent user acceptance.

The selected semantics and alternatives are in
[ADR 0051](../../../decisions/0051-daemon-task-workflows.md#projects-and-execution-environments).
[02e](02e-projects-and-execution-environments.md) was subsequently assigned to implement that boundary, including
current-store adoption, shared project settings, verified Podman attachments and a real Rust
create/edit/build/reconnect journey. The later method, remote and final-review prompts carry its
dependencies without changing their numbers. This planning correction changes no production code,
stores, running services or environment configuration.

On October 22, the user reported, “I'm not really sure what I'm looking at... It's all too complex
for a user and bloated...” and clarified that the concern is “The whole screen and navigation”.
No completed task journey or acceptance of a redesign was reported. A subsequent agent review of
source and existing screenshots identified competing navigation groups, two left panels crowding
the canvas and too many equally prominent controls as possible causes; these are agent diagnoses,
not additional user observations. Early review remains open, with no new production phase assigned.

On October 23, the user requested an expandable daemon → project → workflow hierarchy, empty task
placement before definition, physical input/output connections and a right-hand inspector opened
by clicking a task. They rejected repeated manual schema entry and navigation away from the work,
and asked for a design discussion, including possible daemon changes. The right-side direction
supersedes their October 18 left-side preference for the proposed replacement. The
[canvas authoring discussion](../whiteboard/discussions/product-model/canvas-authoring.md) develops
that interaction and a proposed distinction between saved unfinished authoring and executable tasks.
Durable unfinished work is a recommendation for debate; this request assigns no production changes.
The user then requested agent debate grounded in reducing manual labor and preserving an intuitive
flow. The [UX debate](../whiteboard/discussions/product-model/canvas-authoring-debate.md) records
three actual reviewers' initial arguments, peer objections and revised positions. It challenges
repeated model selection, whole-canvas readiness and protocol-shaped recovery without claiming
actual-user validation or assigning implementation.

The user then asked for further debate that challenges the daemon itself, including faulty or
unhelpful behavior that a frontend cannot reasonably hide. Three additional reviewers traced
lifecycle, command and connection owners and exchanged objections in the
[daemon/frontend discussion](../whiteboard/discussions/architecture/daemon-frontend-contract.md).
The coordinator executed three Rust runtime/store diagnostics; the
[evidence record](../../verification-evidence.md#daemon-lifecycle-design-diagnostics) owns their
observations and limits. This investigation identifies pause defects and debates narrower contract
changes; it changes no production behavior and assigns no implementation phase.

The user then emphasized that product design remains unfinished and requested further debates,
specifically about workflow methods and how people define and use them. Two new reviewers and
the lifecycle reviewer exchanged concrete authoring, activation and outcome cases. The
[method experience](../whiteboard/discussions/product-model/method-authoring-and-use.md) and
[outcomes discussion](../whiteboard/discussions/product-model/work-outcomes-and-delegation.md)
retain their proposals, revisions and unresolved alternatives. Prompt 04 is marked for reconciliation
before assignment; no method implementation, usability observation or new behavioral test was
performed in this round. The selected latest-at-activation default remains unchanged.

The user's next instruction requested a readiness judgment, consolidated design and a separate side
sprint. [ADR 0052](../../../decisions/0052-canvas-authoring-and-method-experience.md) selects the bounded
experience and [canvas experience](../canvas-experience/README.md) prepares its delivery. The debate
is sufficient for that scope; actual usability and execution evidence remain pending. Side S03 must
fulfill this sprint's outstanding 02 review; side S04 takes over all unimplemented 04 methods. Side
S05 has its own full gate, then main 05/06/07 remain. No side phase is assigned or implemented by
this planning action. Use the side schedule for the next correction, not the superseded main 04.

| Finding | Owner and acceptance |
| --- | --- |
| Setup requires users to invent a local name and supply a technical identity. | [02a](02a-daemon-connection-identity.md) implemented address/credential setup, learned identity pin, durable shared name and authorized rename across clients/restart; the user's recheck is pending. |
| Original appearance is rejected and the supplied brand is absent. | [02b](02b-visual-design-and-brand.md) implements the supplied visual direction and real logo. The subsequent feedback rejects interaction usability; visual preference acceptance is not reported. |
| Too many authoring forms, disruptive navigation and unclear creation of multiple workflows. | Assigned [02c](02c-workspace-and-task-creation.md) implemented explicit New workflow/Add task actions and bounded navigation. Its subsequent user recheck rejects the remaining forms and page layout. |
| The workspace remains bloated; input/output relationships require protocol knowledge. | Assigned [02d](02d-canvas-and-direct-authoring.md) implemented the canvas and connection contracts with focused checks. ADR 0052 now selects its replacement; prepared side S02/S03 owns delivery and actual review. Usability acceptance remains open. |
| Project location, daemon execution and Podman's role are unclear; related workflows need project grouping. | Assigned [02e](02e-projects-and-execution-environments.md) adds daemon-owned project registrations around user folders and consistent project-level environments. Physical Podman attachment qualification and actual user review remain open. |
| No successful substantive journey has been reported. | [Side S03](../canvas-experience/S03-early-user-review.md) fulfills outstanding main 02 after S01/S02 implementation: project/files, two workflows, useful inputs/results, saved unfinished work, deliberate Run scope, reconnect, switching, missing-input repair, failure and stop understanding remain open. |

The [retained mockup](assets/milkdrift-ui-sample.png) is byte-identical to the user's
`/home/hartolit/Desktop/milkdrift-ui-sample.png`; SHA-256 is
`6e4eaaf841727207fc1e3f92d3c11506ae1693d1ee73f76c9cfb969c0505f575`.
It is source design material, not a screenshot of the implemented app or an acceptance result.
The supplied [logo](../milkdrift-remedial-planning-v3/assets/milkdrift-logo.svg) remains the brand
source; 02b copies it unchanged into `apps/web/public/milkdrift-logo.svg`. Both copies have SHA-256
`abc0d1d6cf7b5c9f09efe2a263c0463906eb9f49caa8491aaa2959634cb76ea5`.
The mockup's old controls do not
authorize features. The original corrective planning changed no executable code; the subsequently
assigned 02a changes connection and naming behavior, and 02b changes the client presentation.

The [project sidebar reference](assets/project-sidebar-reference.png) is the unchanged October 19
attachment from `/tmp/codex-clipboard-6fa7640f-9192-45da-8023-b97f21a915b6.png`, SHA-256
`e480cbd34fc683a7ae06b75856435f29595bf6657f34e145ec247fc2f8fb64b3`. It illustrates grouping only;
its app controls and behavior are not Milkdrift requirements or implemented-product evidence.

The current sequence is side S01–S05, then main 05–07 when separately assigned. Side S03 fulfills
the actual 02 review, including project/files/environment, naming, interaction and model-assignment
rechecks; side S04 owns all former main 04 work. Preserve completed 03 and the existing numbered
history. The 02c checkpoint and 02d implementation preserve existing stores and tested checkpoints;
their automated checks do not settle the user's rejection. Side S05 owns its full gate and main 07
the later integrated gate. This partial feedback is not acceptance of the journey or its corrections.

## Tested checkpoints and evidence

Implementation checkpoint `bbae7a6` completes assigned 02e and its focused verification, with positive physical Podman
attachment qualification explicitly open under its host-prerequisite clause. The
[02e evidence](../../verification-evidence.md#prompt-02e-projects-and-execution-environments-verification)
records complete affected owner suites, 106 frontend unit cases, scoped Clippy/rustdoc, API review,
all 30 repository contracts and the real two-folder Cargo journey. All 37 browser scenarios have
passing evidence: 35 in the final full invocation, then the two repaired assertions in a focused
rerun on unchanged production assets. Default binaries and stores are retained; current local
review instructions are below. Prompt 07 retains the integrated gate and physical qualification
gap. No actual user acceptance is inferred from these checks.

The 02e planning correction was reviewed against the current project, task, process and managed-worker
owners, then reread by the delegated reviewer after the identified corrections. All eight documentation
contracts pass with `cargo test -p milkdrift-evidence --test repository_contracts --all-features documentation::`;
`git diff --check` passes. These were planning/documentation checks only. That planning checkpoint
used production code `25bcdda`, handed off in `a91e86f`; it established no project execution or
physical Podman evidence. Subsequent implementation evidence is recorded separately.

Implementation checkpoint `25bcdda` contains the tested 02d code and current owner documentation.
The implemented 02d replaces the product forms with one canvas and compact left selection, direct
file/text/result connections, visible task executor selection and contextual corrections. Daemon
metadata supplies real input/output choices, scoped source previews and project roots. The
[02d verification evidence](../../verification-evidence.md#prompt-02d-canvas-and-direct-authoring-verification)
records frontend check/lint/build and 88 unit cases; 50 durable runtime cases; 41 daemon library and
44 control-plane cases; adapter/host/protocol/native-client suites; the actual CLI journey; scoped
Clippy/rustdoc, API review and 30 repository contracts. The complete browser run executed 36 cases:
33 passed and three had test timing/cleanup failures. All six affected cases then passed on the
same populated fixture and final assets, covering those failures and final layout/pending-state
repairs. This is passing evidence for all required scenarios, not a single 36/36 full-suite pass.
Inspected desktop, phone and 200% text renders are under `target/prompt02d-visuals/final/`.
Automated fixture owners were stopped through maintained control and their stores retained.
The separate review instance below remains available. These checks do not establish user acceptance.

The 02d planning change was checked against the current source and the ordinary/parallel/correction
journeys. All eight documentation contracts pass in `target/prompt02d-planning-doc-contracts.log`;
diff whitespace checks pass. These checks establish documentation consistency only. No canvas code,
new contract or user-acceptance result was produced by this planning response.

The 02c implementation checkpoint is `f71be73`. It provides a persistent bounded workflow rail and
one compact composer for the first task of a new workflow or an ordinary addition to the selected
workflow. Instructions are
the only required writing field; optional context and files belong to that task. Eight in-memory
drafts per authenticated connection preserve their destination through navigation. Lost replies
retain the exact request, and stale generations require explicit review. The complete advanced
batch editor remains available for complex edits. These changes use the existing public daemon
operations and preserve the 02a naming and 02b branding work.

The [02c verification evidence](../../verification-evidence.md#prompt-02c-workspace-and-task-creation-verification)
records frontend check/lint, 89 unit cases, production build, all 22 production browser cases and
three final layout reruns. All eight documentation contracts and diff checks pass.
The browser checks cover two workflows, ordinary additions, task-local context/files, navigation,
focus, receipt/replay recovery, stale generations, explicit waits and authority loss, alongside the
existing advanced and automatic-work cases. The Rust fixture correction at `dcf5d17` passes five
unit cases and the actual CLI/daemon journey. Its controlled planner now consumes the task's actual
input brief instead of depending on the old composer's shared workflow input. Production runtime
semantics are unchanged. These are implementation checks; the subsequent user recheck rejects the
remaining interaction. They are not verification of the canvas replacement.

The 02b implementation checkpoint is `90ce1ff`. It replaces the pale theme and literal brand mark,
groups instructions/context/execution, adds responsive daemon navigation and a desktop side inspector,
and preserves drafts and keyboard focus when inspectors or batch editors open and close. The
[02b verification evidence](../../verification-evidence.md#prompt-02b-visual-design-verification)
records frontend check/lint, 67 unit cases, production build, all 16 browser cases, five final render
reruns, the live parallel-work check and eight documentation contracts. Screenshots under
`target/prompt02b-browser-final-results/`, `target/prompt02b-browser-render-results/` and
`target/prompt02b-browser-parallel-results/` were inspected at desktop and narrow widths, including
enlarged text, long content and failure. `target/prompt02b-uncertain-render/` retains an actual
interrupted-daemon render and a fresh-client check that a stop request remains uncertain. The
automated fixture daemons were then stopped; their roots and evidence remain preserved. No actual
user approval is inferred from these checks.

The 02a implementation checkpoint is `c4d7f39`. It includes the storage port/adoption, authority and
shared API, native client/CLI, Svelte setup/settings, recovery fixes and current owner documentation.
Checkpoint `7979c30` separates the unchanged runtime preparation tests to satisfy the repository's
source-size contract. [02a verification evidence](../../verification-evidence.md#prompt-02a-shared-daemon-naming-verification)
records the commands and limits: 41 daemon unit tests, 41 control-plane tests, storage/client/CLI
suites, 30 contracts, scoped Clippy/rustdoc, API review, 59 frontend unit cases and 13 production-browser
cases pass, with focused final reruns. These are implementation checks; the user recheck is pending.

Backend checkpoint `34564c9` contains shared admission/recovery, input/correction policy, cumulative
limits, model/tool execution, native path protection and public daemon projections. Checkpoint
`566fe72` adds CLI batch preview and the controlled automatic journey; `ede0147` presents final model
answers before retained provider metadata. Svelte checkpoint `2c94cad` contains the complete batch
editor, model/tool choices, provenance and verified repair/navigation fixes.
Checkpoint `0d9a774` additionally refuses model action policies on ordinary process capabilities.
Earlier checkpoints, including `37aa8c4`, `95a4dc5` and `c9e9809`, remain preserved. The prior
review handoff is available at `179e189`.

[Verification evidence](../../verification-evidence.md#prompt-03-evolving-task-acceptance) owns the
commands, logs and qualified claims. Focused owner suites, 46 runtime cases, eight production model
admission cases, 31 daemon control-plane cases, 30 repository contracts, API review, Clippy/rustdoc
and dependency audits pass. Frontend check/lint, 43 unit tests, build and all 11 Chromium cases pass.
Desktop and 390-pixel phone-sized views were inspected. These are local focused results, not hosted
CI or full-system acceptance.

The actual-binary report `target/daemon-workflows/prompt03-check-d/check.json` observes one initial
planner, three accepted model batches, eight retained tasks, native tools, actual research overlap,
an explicit failed outcome, correction/rebinding and restart with no duplicate work. The fixture
creates no descendants and sends no continuation batches. Transaction fault tests cover before/after
commit separately from the completed-work binary restart.

Controlled endpoints establish integration, not model quality or implementation of a real animation
system. No configured authorized real model was exercised. Native file tools require Linux and
bounded UTF-8 input; other platforms refuse them. Existing process profiles remain ordinary
capabilities. Methods, remote administration and rolling retention remain later assignments.

## Local review fixture

The latest bounded fixture root is `/tmp/milkdrift-02e-user-review-20261020-a/`.
It was started on October 20 for the project recheck with immutable daemon/fixture copies and a
four-hour lifetime. The preceding `/tmp/milkdrift-prompt02d-user-review-20261018-b/`,
`/tmp/milkdrift-prompt02c-user-review-20261018/` and
`/tmp/milkdrift-prompt02a-user-review-20261017-b/` roots and stores remain preserved. This fixture
uses new development stores and endpoints, separate from automated browser/CLI evidence. The
rebuilt production client is served at `http://127.0.0.1:4173/`.

That October 20 daemon/client checkpoint uses protocol 3.4. Current S01 uses 3.5; preserved review
daemons need their matching older client. The normal
review fixture offers captured-input and project-aware Cargo processes and two controlled model
choices. Additional canvas boundary executors are confined to automated `--canvas-cases` fixtures.
No project or workflow registration is seeded in this review instance. The existing
`daemon-a/animation-project/` folder contains `existing.rs`, and the project-aware Cargo profile
can serve multiple permitted folders.

This temporary root may disappear at host reboot; verify readiness rather than assuming it still runs. Its `ready.json`
records endpoints and process paths; `browser.json` is private. Start the independent production
static preview from the repository root when needed:

```sh
npm --prefix apps/web run preview -- --port 4173 --strictPort
```

Reload `http://127.0.0.1:4173/` to load the rebuilt client, then use **+** beside **Daemon** with the address
and matching credential below. No connection name or installation identity field is needed:

| Name returned by daemon | Daemon URL | Private credential file under the fixture root |
| --- | --- | --- |
| Studio A | `http://127.0.0.1:33961` | `daemon-a/operator.token` |
| Studio B | `http://127.0.0.1:42769` | `daemon-b/operator.token` |

Paste the matching credential from its local file; never put it in a URL, screenshot or committed
document. The browser retains it only in memory. The two daemons have separate actors and stores.

Choose **Add project → Use existing folder**, open the permitted `daemon-a` parent and select
`animation-project`. Its files remain in that host folder. In **Project settings**, choose
**project cargo** under **Execution environment**, deliberately enable **Allow project changes**
and save. Choose **New workflow → Add task**, select **controlled-brief-review** as the model and
write:

```text
Project fixture: Rust code
Create, edit and test a small Rust library in this project.
```

Starting that workflow runs the production model tool loop, native file tools and the same
project-aware Cargo process used by the automated journey. Inspect `Cargo.toml`, `lib.rs` and
`target/debug/deps/` in the selected folder; `existing.rs` stays intact. This fixture returns
controlled tool choices and does not demonstrate real-model quality. Create another workflow
under the same project, then use **Add project → Create folder** for a second project under the
permitted parent and choose the same environment. Identical workflow names remain distinct by
their project. Close and reconnect from a fresh client to find the same registrations and files.

The review environment provides a working host execution choice. Positive Podman project attachment
is not qualified on this host: the installed runtime refuses the pinned mount before entry. The
[physical evidence](../../verification-evidence.md#physical-managed-evidence-and-its-limit)
records that limitation and passing preservation/refusal regressions.

For the naming recheck, connect Studio A in two independent browser tabs/windows. Open **Daemon
settings** in one, save a new name, and observe the other update within the next live refresh.
Reconnect and open a fresh client to read the same accepted name. The settings text should make
clear that renaming affects every client. Studio B should retain its own name and work; if the
names are equal, their addresses distinguish them. Report any confusing step; this remains an
actual-user check, not something automated tests can accept for the user.

To check restart, keep both browser tabs and run the maintained fixture control:

```sh
/tmp/milkdrift-02e-user-review-20261020-a/tools/cargo-report control \
  --root /tmp/milkdrift-02e-user-review-20261020-a restart-a
```

The name should return unchanged after reconnect. A saved profile from the previous review can
remain disconnected; its last observed name is cached. Forgetting that browser profile does not
delete the daemon's work. These newly allocated endpoints use new development stores.

For input-dependent process work inside a project, choose **New workflow**, then **Add task**. Write what the task
should do and choose **cargo report profile** under **Model or agent**. **Attach file** captures the root or
runtime `Cargo.toml` and connects it to the configured manifest input. Under **Tools & autonomy**,
choose **No project tools or file access** for this isolated captured-input process. **Start workflow** accepts
the drawing. Results describe the supplied manifest. Close the client while work runs, reconnect
and discover the retained result.

For instructions alone, **controlled-brief-review** returns a controlled response based on the
submitted text. For the controlled automatic journey, select **controlled-animation-a** on Studio A
or **controlled-animation-b** on Studio B. Use `Animation fixture role: planner` as the instructions.
Choose **Add input** on that task and write the animation-research brief into its connected text
source. Select the task again and, under **Tools & autonomy**, enable
**Let this model propose follow-up tasks**. Open
**Workflow** and uncheck **Finish when required work settles** before starting; the model review
requests finish later. Observe parallel research, failed render research
and its correction. Select cards for results and contextual actions; the compact left panel keeps
the graph visible. Draw prospective tasks and connections, then use **Apply changes** for one guarded
submission. Conflicts retain the drawing for explicit review; lost replies retain the exact request.

If the fixture expires, preserve its root and choose an unused new root:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-cli --bin milkdrift -p milkdrift-evidence --bin connected-tasks
target/debug/connected-tasks serve \
  --root target/daemon-workflows/prompt02e-review-new \
  --daemon target/debug/milkdrift-daemon --lifetime-secs 14400
```

Read that root's `ready.json` for newly allocated URLs. The command refuses an existing root. Current
task documents use schema 3 and redb internal format 22 (physical schema 21). Exact preceding
20/22 stores receive additive project-table adoption; exact 19/22 stores receive both daemon metadata
and project tables. Other old formats are refused. This preserves
workflow and receipt bytes but does not change frozen execution authority. The expanded Controller
preset can deny unfinished old work before its next entry; follow the [upgrade guidance](../../../operations/daemon.md)
and keep matching old binaries/configuration for work that must finish under its original grant.
Earlier failed roots, prior evidence stores and preservation Git refs remain intact. The handoff
leaves prompt 02 open and stops before prompt 04.

## Resume the visual and task review

The 02c client is a rejected baseline. The rebuilt 02e client implements the selected project model
and retains the 02d canvas; it does not implement the selected ADR 0052 experience. Actual acceptance
resumes through [side S03](../canvas-experience/S03-early-user-review.md), after S01/S02 are assigned
and implemented. Prepare a working fixture for that source. The fixtures above remain evidence for
their historical stores and matching clients, not proof of the new design. Preserve their roots
and check readiness; a four-hour endpoint is not an indefinitely running service.

In the S03 review, first attach/create a folder on the named daemon, choose its environment and locate the
real files from a useful coding task. Reconnect from a fresh client and find the same project and
workflows. Verify that the user can explain the distinction between model selection and tool location.
Then create two workflows, add another task to the first, switch between
them and inspect a result without a protocol tutorial. Connect a Cargo manifest to the process task,
then connect its real result to a model task. Empty placement should keep focus on the canvas;
selecting a card opens the right inspector without moving the canvas. Include saved unfinished
authoring, deliberate Run scope and the usual-model action for unassigned tasks. Retain **Model or
agent** selection and contextual setup guidance. Record whether the distinction
between selecting a configured connection and asking the daemon operator to add one is clear.

Close the client while another task runs, reconnect and find saved ideas and accepted work again.
Switch to Studio B and back. Exercise real missing-input readiness and explicit Pause/Continue as
distinct conditions; unrelated input must not answer an explicit wait. Check whether failed,
uncertain and stop-requested work communicate what happened. Verify the narrow-screen drawer,
accessible list and equivalent connection actions without requiring precise dragging.

Review the supplied logo, dark rail, warm text and compact canvas controls against the visual
direction. The mockup's retired revision/controller controls are not product requirements. The
[frontend README](../../../../apps/web/README.md#maintain-and-verify) owns maintained component and
style conventions. Record the user's actions, hesitations and remaining objections here. Automated
checks establish behavior, not visual preference or usability acceptance; actual review stays open.
