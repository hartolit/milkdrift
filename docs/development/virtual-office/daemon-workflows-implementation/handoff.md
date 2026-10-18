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
left-side details and direct input connections. Prepared [02d](02d-canvas-and-direct-authoring.md)
owns the replacement and its necessary daemon support; it remains unassigned. Naming acceptance
and the substantive task journey remain open. Prompts 04–07 remain unassigned.
Prompt 07 owns the integrated full-system gate. No push, history rewrite, external service change or
paid model use occurred.

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

Source inspection confirms those fields in `TaskDefinitionEditor.svelte`, reached through the
compact composer's advanced section and the future-batch editor. `App.svelte` still combines a
workflow page with a separate composer; task details occupy a right-side or stacked inspector.
The previous correction reduced the first form but retained the protocol-shaped editing model.
The user's new left-panel requirement supersedes the older mockup's inspector placement.

Two delegated source/interaction reviews, with the conversation and proposed direction available,
challenged the ordinary and demanding paths. Inspection of actual owners identified required port,
named-result and queued-input read projections, the declaration-only output binding restriction,
native-path selection requirements and the race if accepted tasks start before wiring finishes.
The selected design resolves these in the existing owners; the ADR owns the rules and tradeoffs.
This review is planning evidence, not a working canvas demonstration or independent user acceptance.
The prepared 02d removes the product schema editors, retains their useful actions through direct
selection and keeps draft graphs local until one explicit guarded submission. No production code,
store, running service or dependency changed in this planning response.

| Finding | Owner and acceptance |
| --- | --- |
| Setup requires users to invent a local name and supply a technical identity. | [02a](02a-daemon-connection-identity.md) implemented address/credential setup, learned identity pin, durable shared name and authorized rename across clients/restart; the user's recheck is pending. |
| Original appearance is rejected and the supplied brand is absent. | [02b](02b-visual-design-and-brand.md) implements the supplied visual direction and real logo. The subsequent feedback rejects interaction usability; visual preference acceptance is not reported. |
| Too many authoring forms, disruptive navigation and unclear creation of multiple workflows. | Assigned [02c](02c-workspace-and-task-creation.md) implemented explicit New workflow/Add task actions and bounded navigation. Its subsequent user recheck rejects the remaining forms and page layout. |
| The workspace remains bloated; input/output relationships require protocol knowledge. | Prepared [02d](02d-canvas-and-direct-authoring.md) replaces the product authoring forms with a canvas, compact left selection and direct source/result connections, including the required daemon contracts. Not assigned or implemented; usability unverified. |
| No successful substantive journey has been reported. | Resume [02](02-early-user-review.md) after assigned 02d: create two workflows, connect useful inputs/results, add to one, closure/reconnect, switching, missing-input repair, failure and stop understanding remain open. |

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

Next sequence: assign 02d, finish the actual 02 review including naming and interaction rechecks,
then separately assign 04. The 02c implementation and this planning response preserve existing
stores and tested checkpoints. Its automated checks do not settle the actual user's rejection.
Preserve the completed 03 work and 04–07 numbering. Prompt 07 remains the sole integrated gate;
this partial feedback is not user acceptance of the journey or the proposed corrections.

## Tested checkpoints and evidence

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
remaining interaction. They are not verification of the prepared canvas replacement.

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

The latest bounded fixture root is:

`/tmp/milkdrift-prompt02c-user-review-20261018/`

It was started on October 18 for the 02c recheck with immutable daemon/fixture copies and a four-hour
lifetime. The preceding 02a review fixture had expired; its root
`/tmp/milkdrift-prompt02a-user-review-20261017-b/` and stores remain preserved. The new fixture uses
new development stores and endpoints, separate from automated browser/CLI evidence. The rebuilt
production client is served at `http://127.0.0.1:4173/`.
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
| Studio A | `http://127.0.0.1:38733` | `daemon-a/operator.token` |
| Studio B | `http://127.0.0.1:35229` | `daemon-b/operator.token` |

Paste the matching credential from its local file; never put it in a URL, screenshot or committed
document. The browser retains it only in memory. The two daemons have separate actors and stores.

For the naming recheck, connect Studio A in two independent browser tabs/windows. Open **Daemon
settings** in one, save a new name, and observe the other update within the next live refresh.
Reconnect and open a fresh client to read the same accepted name. The settings text should make
clear that renaming affects every client. Studio B should retain its own name and work; if the
names are equal, their addresses distinguish them. Report any confusing step; this remains an
actual-user check, not something automated tests can accept for the user.

To check restart, keep both browser tabs and run the maintained fixture control:

```sh
target/debug/connected-tasks control \
  --root /tmp/milkdrift-prompt02c-user-review-20261018 restart-a
```

The name should return unchanged after reconnect. A saved profile from the previous review can
remain disconnected; its last observed name is cached. Forgetting that browser profile does not
delete the daemon's work. These newly allocated endpoints use new development stores.

For input-dependent process work, choose **cargo-report · process.execute**, supply a brief and upload
the root or runtime `Cargo.toml`. Results describe the supplied manifest. Close the client while work
runs, reconnect and discover the retained result.

For the controlled automatic journey, choose **controlled-planner · model.generate**, enable
**Allow model to propose future work**, supply an animation-research brief and use instructions
starting with `Animation fixture role: planner`. Leave **Finish when required work settles** unchecked;
the model review requests finish. Observe parallel research, failed render research and its correction.
Select tasks to inspect frozen inputs, source admission and results. Use **Edit future batch** to stage
and preview a guarded batch. Conflicts retain drafts for explicit review; lost replies retain the exact
request identity.

If the fixture expires, preserve its root and choose an unused new root:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-cli --bin milkdrift -p milkdrift-evidence --bin connected-tasks
target/debug/connected-tasks serve \
  --root target/daemon-workflows/prompt02c-review-new \
  --daemon target/debug/milkdrift-daemon --lifetime-secs 14400
```

Read that root's `ready.json` for newly allocated URLs. The command refuses an existing root. Current
task documents use schema 3 and redb internal format 22 (physical schema 20). Only the exact preceding
19/22 store receives atomic metadata-table adoption; other old formats are refused. This preserves
workflow and receipt bytes but does not change frozen execution authority. The expanded Controller
preset can deny unfinished old work before its next entry; follow the [upgrade guidance](../../../operations/daemon.md)
and keep matching old binaries/configuration for work that must finish under its original grant.
Earlier failed roots, prior evidence stores and preservation Git refs remain intact. The handoff
leaves prompt 02 open and stops before prompt 04.

## Resume the visual and task review

The current 02c client is the rejected baseline. Implement 02d only when explicitly assigned before
asking the user to repeat the journey. Preserve the fixture roots and verify readiness at that time;
the recorded four-hour endpoints are not a promise of an indefinitely running service.

Reload the production preview to load the rebuilt client. Review the dark rail, warm text, supplied
logo, compact workflow header and side inspector against the visual direction. The mockup's obsolete
canvas, revision controls and event table are not present. The ordinary task timeline and public
daemon actions retain their meaning. The [frontend README](../../../../apps/web/README.md#maintain-and-verify)
owns shared style/component conventions for the later feature assignments.

First let the user create two workflows, add another task to the first, switch between them and
inspect the result without coaching through protocol fields. Record where they hesitate or become
confused. For the remaining substantive journey, submit instructions with optional context and a
Cargo manifest. Check its result, close the client while another task runs, reconnect and find that
work again. Switch to Studio B and back. Leave **Supply context now** unchecked under **Add context**
for a queued task, then use its inspector to supply the missing input. Check whether failed,
uncertain and stop-requested work communicate what actually happened. **Hide inspector** keeps
selection and edit drafts; on narrow screens **Workspace navigation** opens the rail.

The latest feedback above rejects the implemented 02c creation/navigation path and proposes the
canvas/direct-authoring replacement. After 02d, include connecting a file and a real task result,
then selecting other tasks without losing the canvas or opening protocol forms.
Record the user's own actions and remaining objections here. Passing agent checks does not settle
visual preference or complete this substantive review; no actual-user acceptance has been reported.
