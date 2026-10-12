# Milkdrift web client

Connect this standalone Svelte client to a Milkdrift daemon to submit a task, watch its actual
execution and return to its retained result. The daemon owns admission, execution and history.
The browser can close while work continues. A fresh browser needs the daemon URL, installation
identity and scoped bearer credential; it discovers permitted workflows without a recovery file
or a previously known workflow ID.

The task form supplies instructions, a named `brief` input and an optional captured UTF-8 `file`
input. Select a capability offered by the connected daemon. The form shows the daemon's effective
workflow limits before submission. A configured trusted process can inspect a supplied project;
model capabilities receive the same named context through their maintained adapter.

The timeline orders dependency columns from left to right. Parallel tasks share a column, and
queued tasks show no invented start time. The list offers the same inspector and actions. Future
edits carry the observed workflow generation and task version. A requested stop remains separate
from proof that execution stopped. Results retain content and selected-input digests. Failed,
forbidden, expired and uncertain responses come from the daemon and remain visible.

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

Choose **Connect a daemon**, enter a connection name, daemon origin, installation identity and
credential. The client checks the identity returned by the authenticated session and sends that
identity on subsequent task requests. Credentials stay in memory. Disconnect aborts requests and
observation streams, clears the credential and removes that connection's data. Browser storage
contains only connection names, origins and installation identities. A second connection has its
own actor, requests, cursor, search and selected workflow.

If a submission reply is lost, use **Find submission receipt** or **Replay exact submission**.
The current form retains the exact request while it is open. After a complete browser restart,
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

Install the matching browser once with `npx playwright install chromium`. Browser tests produce
desktop and phone result screenshots under ignored `test-results/`; traces are disabled because
they can capture bearer headers and credential form values. Unit/component tests cover connection
separation, credential disposal, fragmented observation framing and accessible refusal messages.
Passing these focused checks is evidence for this client boundary; the sprint's final full-system
gate remains in prompt 07 and actual user review remains in prompt 02.
