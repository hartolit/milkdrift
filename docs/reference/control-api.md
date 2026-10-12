# Local control API 3.0

Use this API to submit ordinary tasks, discover permitted workflows and inspect retained results.
All clients use the daemon's owner queue. The standalone [Svelte client](../../apps/web/README.md),
[CLI](../../apps/cli/README.md) and [Rust client](../../crates/control-client/README.md) project the
same operations. Independent capability serving uses the separate invocation routes below;
remote hosts use the [peer protocol](peer-protocol.md).

## Transport and authentication

Every route requires `Authorization: Bearer …`, including negotiation and health. Configuration
maps that credential to an immutable actor/grant revision. JSON cannot choose an actor. The client
negotiates the exact version before use; older graph clients are refused:

```json
{"protocol":{"major":3,"minor":0}}
```

Send this to `POST /v1/version`. `GET /v1/session` returns the installation identity, authenticated
actor, available task capabilities and effective defaults. Browser requests subsequently carry
`x-milkdrift-daemon` with that identity; a mismatch is refused. Keep credentials in memory and out
of URLs, logs and browser storage. All closed requests reject unknown fields and duplicate keys.
The bounded JSON reader limits bytes, nesting and collection sizes before dispatch.

Success bodies use the common envelope:

```json
{"protocol":{"major":3,"minor":0},"request_id":"req-1","value":{}}
```

Errors use a stable code and redacted explanation. They never include credentials or task content:

```json
{"protocol":{"major":3,"minor":0},"request_id":"req-1","code":"conflict","message":"workflow generation changed","retryable":false,"details":{}}
```

`unauthenticated` means the credential failed, `unauthorized` means the current grant does not
permit the action, and `not_found` means no permitted retained object is available. `conflict`
requires inspecting current state before choosing a new request. Capacity/overload refusals are
explicit; retryability never authorizes changing or blindly repeating a semantic action.

## Ordinary task routes

| Method and route | Purpose |
| --- | --- |
| `GET /v1/workflows?search=&limit=50&cursor=` | Discover permitted workflows without a known ID. |
| `POST /v1/workflows` | Atomically create a workflow and assign IDs to local task aliases. |
| `GET /v1/workflows/{workflow}?cursor=` | Read workflow state and a bounded page of task summaries. |
| `POST /v1/workflows/{workflow}/changes` | Apply one generation-guarded atomic edit batch. |
| `GET /v1/workflows/{workflow}/tasks/{task}` | Inspect a selected task, frozen start and result preview. |
| `GET /v1/workflows/{workflow}/tasks/{task}/result` | Read authorized result metadata and a text preview. |
| `GET /v1/workflows/{workflow}/tasks/{task}/result/content` | Download complete bounded UTF-8 result text. |
| `GET /v1/workflow-requests/{request}` | Recover this actor's durable request acceptance. |
| `POST /v1/task-inputs` | Capture an actor-owned UTF-8 file. |
| `GET /v1/workflows/stream` | Receive authenticated snapshot invalidation hints. |

The following request creates a finite process task. Replace the capability with an exact choice
returned by the connected daemon. The daemon assigns workflow/task IDs and returns the `report`
alias mapping; the alias has no identity outside this request.

```json
{"request_id":"project-report-1","title":"Inspect project","description":"Report the supplied Cargo project","tasks":[{"alias":"report","definition":{"title":"Inspect Cargo manifest","description":"Describe targets and dependencies","instructions":"Inspect the supplied file and summarize its package, targets and dependencies.","capability":{"id":"cargo-report","operation":"process.execute"},"inputs":{"brief":{"kind":"supplied","name":"brief","optional":false},"file":{"kind":"supplied","name":"file","optional":false}},"outputs":{},"dependencies":[],"required":true,"corrective_of":null,"paths":[]}}],"inputs":{"brief":{"kind":"text","text":"Give a concise project report."}},"finish_when_settled":true,"limits":null}
```

This task remains queued because `file` is missing. Capture file bytes with:

```json
{"request_id":"manifest-upload-1","name":"Cargo.toml","media_type":"text/plain","text":"[package]\nname = \"sample\"\nversion = \"0.1.0\"\n"}
```

Bind the returned `input_id` under the current workflow generation:

```json
{"request_id":"supply-manifest-1","expected_generation":1,"changes":[{"kind":"bind_input","name":"file","value":{"kind":"uploaded","input_id":"input:replace-with-upload-receipt"}}]}
```

Uploads retain exact UTF-8 bytes, basename and digest. Binary files and oversized requests are
refused. The text limit is 16 KiB per value/file. Combined rendered input is also bounded; an oversized
composition is refused before accepting a task or edit. JSON escaping counts toward that bound. Upload references belong to the authenticated actor.
At start, captured content and selected producer artifacts become immutable task inputs.

Every create/change request has an exact canonical identity. JSON object ordering and whitespace do
not matter; omitted fields and explicitly supplied defaults remain different requests. A duplicate
returns its original receipt. Changed reuse conflicts, including changed upload identities. If a
reply is lost, query the receipt or replay the exact request. A local recovery file is optional;
after losing the client entirely, discovery still finds accepted work.

## Future changes and completion

Changes include `add`, `revise`, `remove`, `bind_input`, `stop`, `wait`, `continue`, `finish` and
`adopt_correction`. Revise/remove carry the observed task version as well as the batch's workflow
generation. An already frozen start refuses revision. New definitions never rewrite retained
history. Required producer output and settled producer outcome are different input bindings;
missing required output keeps consumers queued. Definitions default to required.

Stop uses the task's returned `stop_request_id` with one exact `stop` change. Its reserved capacity
remains available when ordinary admission is full. A cancellation request alone does not establish
stopped: the public task can remain running/stop-requested or uncertain until durable evidence
settles it. An entered process without terminal evidence remains uncertain after restart and is
not silently repeated. Native writer claims remain held while its outcome is uncertain.

Correction adoption names a successful corrective task and optional version-guarded future input
redirects. The original failure/result remains inspectable. No automatic correction or model-led
workflow editing is implemented in this slice. Cumulative limits never replenish on continue.

## Reads, pages and streams

Discovery searches workflow/task titles, descriptions and authorized retained text results. Each
request examines a finite window and returns `truncated` and `next_cursor`; absence from one page
is not proof of absence. Cursors bind the daemon, actor, grant and search. Workflow task pages use
`next_task_cursor` and expose `task_count`; they contain summaries, not captured inputs. These are
live pages, not a historical snapshot transaction. Both item counts and encoded page bytes are
bounded. Refetch from the beginning after edits when a consistent current view matters.

A selected task contains its definition and frozen input provenance. Frozen text and combined
result text have explicitly labelled 16 KiB previews; their digests refer to complete content.
`result_restricted` preserves permitted task-state inspection while indicating inaccessible output.
The explicit result route still refuses unauthorized output reads. Captured inputs also recheck
current artifact permissions; restricted previews and digests are withheld. Non-UTF-8 declared text
sets `text_unavailable` while retaining task state, artifacts and the complete byte digest. Complete
result text permits 2 MiB of artifact content plus bounded separators; individual immutable artifacts use `/v1/artifacts/{artifact}/content` and bounded ranges.

The workflow stream uses authenticated fetch, not native EventSource with a token in its URL.
It sends periodic `workflow_changed` invalidations; clients refresh their visible page and selected
workflow. It is not a replayable task journal. The connection has a finite lifetime. On reconnect,
`Last-Event-ID` is checked against actor/grant/daemon binding; an old daemon incarnation yields
`resync_required` and a fresh authorized snapshot is required. Revocation/expiry closes the stream
with a useful forbidden state. Queue/connection saturation refuses admission explicitly.

## Independent host and administrative routes

Health/readiness, capability catalog, current authority, peer administration and immutable artifacts
remain scoped public operations. `/v1/invocations/prepare`, `/v1/invocations`, request lookup,
inspection, observations, output and cancellation retain the independent execution-host path.
`/v1/resources` retains managed-resource generation checks, deployment verification and protected
resource holds. Task creation does not bypass those independent guards. See the
[daemon](../operations/daemon.md), [authority](../operations/authority.md),
[peer](../operations/peers.md) and [managed Linux](../operations/managed-linux.md) guides.

Old graph import/run/revision/proposal/controller/layout/learning/publication routes and their
readers are removed. There is no compatibility execution engine or conversion endpoint.

## Browser hosting

Configure exact allowed origins in `serving.browser_origins`. A mismatched origin is refused;
preflight explicitly allows bearer/identity headers and supported methods. Credentials are not
cookies. Standalone static hosting needs no CLI or filesystem access. Loopback HTTP is the local
development exception. Phone/LAN use requires browser-trusted HTTPS for the static site and daemon
proxy; forward authentication headers and disable SSE buffering. Physical phone and deployed proxy
behavior require separate qualification. See the [web setup](../../apps/web/README.md).

[Current versions](versions.md) are checked against their owning constants.
