# Milkdrift web client

Connect this standalone Svelte client to a Milkdrift daemon to submit a task, watch its actual
execution and return to its retained result. The daemon owns admission, execution and history.
The browser can close while work continues. A fresh browser needs the daemon URL and scoped bearer
credential; it learns the daemon's name and identity from its authenticated session and discovers
permitted workflows without a recovery file or a previously known workflow ID.

The daemon owns its name. **Daemon settings** lets an authorized administrator rename it for every
client; readers without that permission see the name without a rename action. Concurrent edits
retain the draft for a new decision. A lost reply offers **Find rename receipt** or **Replay exact
rename**, preserving the original request. Connected clients refresh names through the existing
bounded observation stream, even when workflow discovery is unavailable or has no results.

This browser saves the endpoint, learned identity and last observed name, never the credential.
Disconnected names are explicitly cached. Older profiles retain their endpoint, identity pin and
local selection ID; their former nickname is replaced by the daemon's name on authentication.
If a saved address points to a different daemon, reconnect stops before loading work. Check the
address, deliberately forget the saved connection, then connect afresh. **Connection details**
explains the stable identity; it identifies the daemon, not a device, and is not a setup field.

Choose **Add project**, then attach an existing folder or create one child in a permitted parent.
The chooser reads directories on the named daemon, including when the browser is on a phone.
Attaching preserves files and Git state. New projects initially grant no write tools. In **Project
settings**, choose a configured execution environment and deliberately enable **Allow project
changes** when coding work needs it. The selected environment governs file/process tools; **Model
or agent** remains a separate choice for inference. Unavailable environments explain their setup
requirements without silently choosing host execution.

Within a project, **New workflow** creates an empty saved workspace. **Add task**, or dragging a
task onto the canvas, places a blank card without opening its details or interrupting the selected
task. Select the card to open its inspector on the right. Write **What should this task do?**;
its editable title is suggested from your instructions. Cards, sources and unfinished wires save
to the daemon without becoming executable work. **Saved**, **Saving…** and **Not saved** distinguish
the checkpoint state. A failed save retains the local candidate and offers **Retry save**; a
concurrent save asks which version to retain rather than overwriting another client's work.

**Run pending plan** submits the saved plan after daemon review. **Choose tasks to run** selects a
smaller group while other ideas remain saved. Review includes required predecessors and any
conflicting candidate choices; **Run reviewed plan** accepts that reviewed selection together.
Accepted task definitions and receipts remain immutable. The canvas retains its placement and
selection, and its saved draft identities map to the accepted tasks.

The visible **Model or agent** choice comes from this daemon. New cards have no automatic
assignment. **Usual model…** deliberately applies a model to unassigned cards and future cards on
this canvas, with the affected cards highlighted. A missing or changed selection requires review.
**Models & agents** lists available models and configured processes;
**Add model or agent** explains the existing operator setup and links its maintained guides.
Connections are configured once on the daemon computer, then shared with permitted clients.
The browser does not install agents, load weights or write provider credentials. See the
[model setup guide](../../docs/guides/local-model-endpoint.md) and
[process setup guide](../../docs/guides/local-process.md).

Instructions alone are enough for a model task. To add context, create a text or captured file
source, then drag its connector to a task or use **Connect to…**. **Add input** on a task starts
the same operation. A configured process may require a particular input: its real slots come
from the daemon, and a contextual choice appears when several destinations are possible. Binding
keys and task identities are generated. Captured files are UTF-8 and bounded by the daemon's
advertised upload limit, currently 16 KiB; capturing a file grants no daemon filesystem access.

Connect a task's real result to use its successful output. The client offers actual declared or
retained named outputs, not the combined display preview. The daemon derives the result prerequisite.
**Use the outcome even if it fails** deliberately accepts the settled failure report; optional
absence is a separate choice. **Run after…** adds ordering without importing a result. Removing a
wire retains prerequisites still needed by other wires or an explicit ordering relationship.
New work and its inputs are submitted atomically; drawing or saving a card never starts it.
The daemon constructs executable inputs, output declarations and independent ordering from the
saved drawing and current capability contracts. Existing accepted output contracts stay fixed.
New ordering distinguishes **After success** from **After completion**. Older accepted dependency
arrays retain their existing meaning; they do not establish the original author's intent for an
independent ordering wire. Editing an unstarted task shows the daemon's projected connections so
removing one has an explicit prospective meaning.

Queued missing inputs appear as sources that can be supplied in place. A shared source names its
affected future consumers before replacement. Started tasks keep their frozen versions. Missing,
restricted, expired and outside-page sources are distinct; the client never substitutes empty text
for unavailable content. Workflow and task discovery are paged, with explicit bounded loading.

The rail groups projects beneath their daemon and workflows beneath their project. Identical labels
do not merge registrations or drafts. **Earlier workflows** lists retained work without an
association; explicitly selecting a project changes only later submissions. Accepted definitions
and original receipts stay unchanged. **All accessible workflows** also discovers work whose
project settings this connection cannot inspect.

Expand a daemon, then a project, to find its workflows without leaving the canvas. Branches have
bounded pages, search and an explicit refresh. **All workflows** provides broader discovery;
**Manage connections** holds removal of saved connection settings. Browser Back/Forward restores
location without submitting work. Workflow observations refresh workflow data; a separate name
hint refreshes session metadata without reloading unrelated project forms.

One saved canvas holds up to 200 pending tasks, 400 sources and 1,024 connections in 2 MiB.
Up to 600 positions save independently, so moving a card does not invalidate a reviewed plan.
A Run remains bounded to 32 actions and is never silently split. The browser bounds retained
unsaved or pending canvases to eight; saved canvases can be evicted from its cache and loaded from
the daemon again. Selected task details have a separate 8 MiB cache. **Clear cached details**
releases those details without discarding saved work or accepted connections.

Disconnect, identity replacement and authority loss dispose of inaccessible browser state.
Saved unfinished work survives browser reload and daemon restart; an unconfirmed local checkpoint
is not promised durable. Credentials and canvas content never enter browser storage or URLs.
Tree expansion preferences and cached connection settings are local presentation state. The supplied
Milkdrift logo remains unchanged.

Project setup separately retains at most eight drafts per authenticated connection, one for each
project or new-project form. Back/Forward restores the matching draft, including an exact request
whose reply is unconfirmed. **Discard project draft** frees unsent changes; pending requests require
receipt recovery. Actor changes clear these drafts and invalidate old reads and mutation replies.
Removing an unused registration preserves unrelated drafts. A saved empty workflow is already
associated with its project; removing a registration remains subject to the daemon's reference checks.
Start new work in a registered project.

Pan, zoom, fit and the accessible list operate on the same graph. At narrow widths, the right inspector
uses the available height for editing and temporarily hides the canvas toolbar and Run actions.
**Done** closes it and restores those controls with the same selected card and camera.
Saved positions remain presentation; the secondary timeline shows recorded acceptance
and settlement intervals, which can include preparation and waiting. A frozen start can remain
queued before external entry; its original acceptance time is preserved. Results have bounded previews
and explicit complete-content downloads; technical facts remain separate from ordinary authoring.
Access restrictions and uncertainty remain visible.

**Edit task**, **Create correction**, **Use correction for future work**, **Request stop** and
**Remove unstarted task** are contextual actions. Pause, Resume and Stop act directly without
submitting unrelated canvas edits or leaving the selected task. Lost replies use the same exact
receipt recovery as authoring; an unresolved submission must be recovered before another command.
Preparing and waiting before entry remain distinct from external execution. Project contention shows
**Project is in use** without exposing another actor's work. A paused workflow stays paused when
an input is supplied; Resume is deliberate. Workflow actions also expose completion
and prospective limits. Rejected model proposals can be reviewed and dismissed with repaired work
in the same atomic change; dismissal does not resume a separate human wait. Model continuation,
automatic repair and tools remain deliberate permissions. New tasks inherit reviewed project
settings or select an explicit restriction in **Tools & autonomy**. Accepted tasks display their
recorded execution environment and access; changing project defaults does not change them. A stale
settings draft requires review. Native file tools use host paths while a Podman process uses
`/project`. Trusted host tools are not sandboxed by file claims. Ordinary input connections require
no file-tool permission. Rust rechecks authority and native paths before entry.
Corrections keep the source task's environment, tools and access as an explicit new selection,
reviewed against the current project generation. New defaults cannot silently widen the correction.
If the original environment is no longer available, submission refuses until the user reviews a
different selection. Hidden execution selections cannot be copied into a correction.

Project settings show the actual folder and availability. Rename changes its label, and removing
an unused registration preserves the directory. If a creation reply is lost, recover the exact
receipt or replay the same request. An unresolved creation offers explicit attachment after
inspection or abandonment; neither deletes files. Moved or replaced folders become unavailable
instead of being silently recreated.

Concurrent changes or a task starting can refuse a prospective edit. The drawing remains for a
new explicit decision after refresh. A lost response instead preserves the exact target, payload
and request identity for receipt lookup or exact replay across navigation. Neither recovery path
silently expands permissions, resumes waiting work or increases limits. Started definitions,
failed history and frozen inputs remain unchanged when a successful correction redirects future work.

## Run locally

Use Node.js 24 or newer and the locked dependency tree:

```sh
cd apps/web
npm ci
npm run dev
```

The development server listens on `http://127.0.0.1:5173`. Configure that exact origin in the
daemon's `serving.browser_origins`; `localhost` and `127.0.0.1` are different origins. An empty
origin list disables browser access. The daemon's [README](../daemon/README.md) owns daemon and
account setup. Use a new explicitly selected development store for this sprint, preserving old
stores and running services.

Choose **Connect a daemon**, enter the daemon origin and
credential. The client learns the authenticated daemon identity, checks the saved pin on reconnect
and sends that identity on subsequent requests. Credentials stay in memory. Disconnect aborts requests and
observation streams, clears the credential and removes that connection's data. Browser storage
contains cached daemon names, origins, learned identity pins and tree expansion preferences. A second connection has its
own actor, requests, cursor, search and selected workflow.
The client retains at most 16 connections. **Forget** disconnects a connection and removes
its saved non-secret settings, so another connection can be added without dropping existing connections.

If a submission reply is lost, use **Find submission receipt** or **Replay exact submission**.
The draft retains the exact destination and request across navigation. A confirmed receipt opens
its workflow if that draft is still current; it never pulls you away from another workflow or daemon.
A generation conflict keeps the draft for **Refresh workflow for review** and a new explicit
submission. After a complete browser restart, authenticate and discover both accepted work and
saved unfinished canvases through the workflow tree, list and search.

## Host independently

`npm run build` produces static files in `dist/`. Serve them at the root of an independently
hosted site; no Node application server, CLI subprocess or access to the daemon store is needed.
`npm run preview -- --port 4173 --strictPort` serves a local inspection build. Add its exact origin
to the daemon configuration when using it.

For phones or LAN use, serve both the static site and daemon endpoint through browser-trusted
HTTPS. Point the daemon's HTTPS proxy at its configured private listener, allow the static site's
exact origin, forward the bearer and `x-milkdrift-daemon` headers and disable buffering for
`/v1/workflows/stream`. Keep the stream response open and flushed. Verify this through the actual
proxy before relying on live updates. Loopback HTTP is the development exception. The client
refuses plaintext LAN endpoints and an HTTP daemon from an HTTPS page; certificate bypass is not
a supported setup.

## Maintain and verify

`src/style.css` owns shared colors, typography, spacing, controls, focus, navigation and authoring
panels. It loads before component styles in `src/main.ts`. `src/canvas.css` owns the canvas workspace, execution lanes and inspector. Extend these owners when
adding a surface so forms and status cues remain consistent. Keep labels alongside state colors,
allow long names to wrap, and preserve draft content when toggling the inspector.
`public/milkdrift-logo.svg` is the exact supplied brand asset, copied from
`docs/development/virtual-office/milkdrift-remedial-planning-v3/assets/milkdrift-logo.svg` at the
repository root; change its display size in CSS without changing its paths or colors.

Rust owns transport DTOs and protocol negotiation. After changing their declarations in
`crates/control-protocol/src/`, regenerate from the repository root:

```sh
cargo run -p milkdrift-control-protocol --example export_web --features web-codegen
```

Do not edit `src/lib/protocol.generated.ts` by hand or add a second task semantic validator in
TypeScript. The client handles presentation, checkpoint candidates and HTTP/SSE transport; daemon responses decide
readiness and outcomes. Fetch-based SSE sends bearer headers, and an expired cursor discards that
connection's continuation and refreshes its authorized snapshot.

Run the frontend checks from this directory:

```sh
npm ci
npm run check
npm run lint
npm run test:unit
npm run build
npm run test:browser
```

The browser suite uses a production static build and two real daemons started by the repository's
Rust evidence harness. Supply its private configuration file through `MILKDRIFT_BROWSER_CONFIG`.
For the complete canvas suite, start a fresh fixture with **serve --canvas-cases**, as documented
in the [Rust harness guide](../../tools/evidence/README.md). The additional executors are isolated
test cases; ordinary user-review fixtures omit them. The suite fails setup if those required cases
are absent. Use a fresh root for a complete run: the controlled endpoint has a finite call allowance,
and repeated suites must not rely on a previously consumed fixture.
The configuration names each daemon's URL, identity and credential, the `cargo-report` capability,
two captured Cargo inputs and expected report terms. Backend launching, process fixtures and fault
orchestration stay in Rust. The TypeScript tests operate the client, inspect public responses and
invoke the Rust seed command for bounded retained-state scenarios; they do not write daemon stores.
When the harness supplies `model_capability` and `model_expect`, the suite also exercises the
controlled external model through the same canvas and verifies the retained response.
Prompt 03 requires `planning_capability` from that Rust harness. Its automatic journey submits only
the initial planning task, observes daemon-created research/composition/correction, checks recorded
parallel intervals and the failed original, and asserts that the browser sent no continuation
batch. The fixture controls external responses and delays; it does not schedule child work.

Install the matching browser once with `npx playwright install chromium`. Browser tests produce
desktop and phone result screenshots under ignored `test-results/`; traces are disabled because
they can capture bearer headers and credential form values. Unit/component tests cover connection
separation, credential disposal, fragmented observation framing and accessible refusal messages.
Passing these focused checks is evidence for this client boundary. The canvas side sprint's S05
owns its full-system gate; main prompt 07 owns the later integrated gate. Actual user review remains
open and is scheduled in side S03 against main prompt 02's criteria.
