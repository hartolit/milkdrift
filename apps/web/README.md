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

Choose **New workflow** for an unsaved canvas. **Add task** adds a draft card and opens its compact
left inspector. Write **What should this task do?**; its editable title is suggested from your
instructions. Select another task to change the inspector while keeping the canvas and camera.
**Start workflow** accepts the first task or connected group together. An empty canvas creates no
durable work. Within an existing workflow, **Add task** or **Apply changes** accepts the drawing
through one guarded request. Cards and wires remain drafts until that explicit action.

The visible **Model or agent** choice comes from this daemon. A sole available choice or the last
deliberate selection on the same authenticated connection can be preselected. A missing choice
is never silently replaced. **Models & agents** lists available models and configured processes;
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
retained named outputs, not the combined display preview. It adds the implied prerequisite.
**Use the outcome even if it fails** deliberately accepts the settled failure report; optional
absence is a separate choice. **Run after…** adds ordering without importing a result. Removing a
wire retains prerequisites still needed by other wires or an explicit ordering relationship.
New work and its inputs are submitted atomically; drawing a card never starts it before wiring.
When a draft producer exposes several results, the connection panel shows how they share its
daemon-advertised output allowance before applying that choice. Existing accepted output contracts
stay fixed. Accepted prerequisites do not record whether they originally came from a wire or an
explicit order; removing a result wire therefore retains its **Run after** relationship until you
deliberately remove that ordering connection too.

Queued missing inputs appear as sources that can be supplied in place. A shared source names its
affected future consumers before replacement. Started tasks keep their frozen versions. Missing,
restricted, expired and outside-page sources are distinct; the client never substitutes empty text
for unavailable content. Workflow and task discovery are paged, with explicit bounded loading.

The rail keeps the daemon switcher, workflow creation, recent workflows and **All workflows**
discovery. Browser Back/Forward restores location without submitting work. Each authenticated
connection retains up to eight canvas drafts in memory. A canvas bounds its staged changes to 32,
loaded tasks to 200, sources to 400, edges to 1,024, draft data to 2 MiB and selected-task cache to
8 MiB. Capacity refusal preserves existing work and offers explicit discard. No automatic splitting
of a large batch occurs. Disconnect, identity replacement and authority loss dispose of inaccessible
state. Browser reload loses unsent work; draft content and credentials never enter browser storage
or URLs. The supplied Milkdrift logo remains unchanged.
At the eight-draft limit, other workflows remain available as a clean view. **Clear cached details**
releases loaded detail without discarding drafts or accepted connections; select a task to load it
again.

Pan, zoom, fit and the accessible list operate on the same graph. A closable left inspector stays
compact at narrow widths. Layout is presentation; the secondary timeline uses actual starts and
settlements to show overlap. Queued tasks have no invented start time. Results have bounded previews
and explicit complete-content downloads; technical facts remain separate from ordinary authoring.
Access restrictions and uncertainty remain visible.

**Edit task**, **Create correction**, **Use correction for future work**, **Request stop** and
**Remove unstarted task** are contextual actions. Workflow actions expose wait, resume, completion
and prospective limits. Rejected model proposals can be reviewed and dismissed with repaired work
in the same atomic change; dismissal does not resume a separate human wait. Model continuation,
automatic repair and tools remain deliberate permissions. File tools select permitted project roots
and access modes supplied by the daemon, with an optional relative subfolder. Ordinary input
connections require no file-tool permission. Rust rechecks authority and native paths before entry.

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
contains only cached daemon names, origins and learned identity pins. A second connection has its
own actor, requests, cursor, search and selected workflow.
The client retains at most 16 connections. **Forget** disconnects a connection and removes
its saved non-secret settings, so another connection can be added without dropping existing connections.

If a submission reply is lost, use **Find submission receipt** or **Replay exact submission**.
The draft retains the exact destination and request across navigation. A confirmed receipt opens
its workflow if that draft is still current; it never pulls you away from another workflow or daemon.
A generation conflict keeps the draft for **Refresh workflow for review** and a new explicit
submission. After a complete browser restart,
authenticate and discover the accepted work through the workflow list and search instead.

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
`crates/control-protocol/src/tasks.rs`, regenerate from the repository root:

```sh
cargo run -p milkdrift-control-protocol --example export_web --features web-codegen
```

Do not edit `src/lib/protocol.generated.ts` by hand or add a second task semantic validator in
TypeScript. The client handles presentation, drafts and HTTP/SSE transport; daemon responses decide
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
Passing these focused checks is evidence for this client boundary; the sprint's final full-system
gate remains in prompt 07 and actual user review remains in prompt 02.
