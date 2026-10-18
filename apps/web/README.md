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

Choose **New workflow** to compose its first task, or **Add task** within a workflow to add to that
workflow. Both use the same composer alongside the existing work. Write **What should this task do?**;
the editable title is suggested from those instructions. A new workflow uses that title unless you
change its name in **Advanced task options**. Nothing is created until submission.

Choose an **Executor** from the connected daemon's permitted capabilities. A sole available executor
or your last deliberate choice on this authenticated connection can be selected for you; the choice
is always visible. A missing or removed choice is never replaced with another executor automatically.
Instructions alone are sufficient for authoring; an executor that needs particular inputs can still
refuse work that does not supply them. Its requirements are not inferred from its name.

**Add context** supplies a task-local `brief`; **Attach text file** captures a task-local UTF-8 `file`
input. Adding either to another task does not replace values used by earlier queued tasks. Captured
files are bounded by the daemon's advertised limit, currently 16 KiB, and the daemon checks combined
instructions and selected input text. Under context, uncheck **Supply context now** to deliberately
queue a task awaiting its own required context. The inspector can later supply that input.

**Advanced task options** retains named inputs, dependencies, output contracts, project paths and
workflow limits. Open **Model continuation and permitted tools** there to opt in to future-work proposals and,
separately, automatic feedback after a refused proposal. Select file tools only with declared paths
and select process tools from configured capabilities. The daemon rechecks the category and grant;
the browser does not authorize a capability by showing it. Turn off **Finish when required work
settles** when the model will explicitly request completion after creating its follow-up work.
An ordinary addition sends one guarded change; it never implicitly resumes a waiting workflow,
changes its completion choice or enlarges its limits.

The dark workspace keeps a daemon switcher, **New workflow**, the current workflow and up to eight
recent workflows in the left rail. **All workflows** opens paged discovery and search; the recent
list is not the whole collection. On narrow screens, **Workspace navigation** opens the rail.
Browser Back/Forward restores location with authenticated reads, without submitting work.
The supplied Milkdrift logo is used unchanged.

Each authenticated connection retains up to eight task drafts in memory. Switching workflows or
opening settings preserves them. **Keep draft**, **Close composer** and Escape hide the composer;
reopen it through **Add task**, **New workflow** or the rail's **Drafts** list. **Discard draft** frees
its slot. At capacity, new drafts refuse without dropping existing work. Disconnect, authority loss
and identity replacement dispose of inaccessible drafts. Reloading the browser loses unsent drafts;
neither draft text nor credentials are saved in browser storage or URLs.

The timeline orders dependency columns from left to right. A shared time axis also shows actual
overlapping starts and settlements, with open bars for tasks without recorded ends. Queued tasks
show no invented start time. The list offers the same inspector and actions. Future
edits carry the observed workflow generation and task version. A requested stop remains separate
from proof that execution stopped. Results retain content and selected-input digests. Failed,
forbidden, expired and uncertain responses come from the daemon and remain visible.
Workflow discovery and task lists display bounded pages with explicit navigation. Selecting a
task fetches its full inspector separately. The inspector sits beside the timeline on wide screens
and stacks below it on narrower screens. **Hide inspector**, or Escape from inside it, preserves
the selection and unfinished edit; **Show inspector** returns focus to its last focused control.
Results can be collapsed without losing their content. Technical task facts and dependencies have
a separate disclosure. Large inputs and results show a labelled preview;
**Download result** requests the complete retained text from the daemon and displays any refusal.
If the current grant denies retained content, the inspector labels that restriction while keeping
the permitted lifecycle visible. Non-UTF-8 output retains its artifact identities and byte digest
with an explicit notice that it cannot be shown as text.

**Edit future batch** stages complete additions, revisions, input supply, removals, stop requests,
correction adoption and explicit wait/resume/finish decisions. Task definitions expose named input
policies, dependencies, required obligations, output contracts and scoped project paths. Required
producer outputs wait for success; optional outputs wait for settlement; a **Settled task outcome**
binding explicitly accepts failure. Inspect the exact batch preview before **Submit reviewed batch**.
The daemon validates and accepts the complete batch atomically. A preview itself sends no changes.
Finish or close an advanced editor before navigating away from its workflow; the client keeps it
open instead of silently discarding the draft. An unconfirmed change must be recovered before leaving.

If a concurrent human/model edit wins, the editor retains the draft and requires a refreshed
generation and explicit review. Reload affected task definitions before staging their new versions;
the client never merges or retries a refused semantic edit automatically. A lost reply instead
holds the exact request unchanged for **Replay exact batch**. The JSON draft editor uses the same
typed public change array as the CLI and supports editing any staged change before preview.

**Create linked corrective work** prepares a new task linked to the original. Its corrective link
keeps replacement work waiting for the original to settle. An uncertain or stop-requested task is
not evidence of a safe replacement. Once correction succeeds, choose **Adopt a successful correction**
and select the future consumer bindings to redirect. Historical failures and frozen started inputs
remain inspectable. Workflow limits are available in a disclosure below the task workspace.
That panel reports cumulative admissions and elapsed execution beside the ceilings. The response
decision panel links source tasks, immutable response digests, receipts and allocated child aliases.
For a refused response, stage **Dismiss a response decision** with the reviewed replacement changes;
this is an explicit decision through the same batch path, with no hidden permission expansion.
**Change prospective workflow limits** and **Resume prospectively** are separate staged actions.

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
panels. It loads before component styles in `src/main.ts`. `src/workflow.css`, imported by
`WorkflowView.svelte`, owns the timeline, execution lanes and inspector. Extend these owners when
adding a surface so forms and status cues remain consistent. Keep labels alongside state colors,
allow long names to wrap, and preserve mounted editor content when toggling the inspector.
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
The configuration names each daemon's URL, identity and credential, the `cargo-report` capability,
two captured Cargo inputs and expected report terms. Backend launching, process fixtures and fault
orchestration stay in Rust. The TypeScript tests only operate and inspect the client.
When the harness supplies `model_capability` and `model_expect`, the suite also exercises the
controlled external model through the same task form and verifies the retained response.
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
