# Local control API 3.6

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
{"protocol":{"major":3,"minor":6}}
```

Send this to `POST /v1/version`. `GET /v1/session` returns `daemon_id`, `daemon_name`,
`daemon_name_generation` and `can_rename_daemon`, together with the authenticated actor, available
task capabilities and effective defaults. First connection needs only the endpoint and credential.
The session also projects `authoring` contracts from the configured adapter generation: readable
executor labels, exact input slots, real output keys, conditional publication, byte bounds and an
owner-defined default output when one exists. Missing metadata means the client cannot offer direct
connectors for that executor. These facts contain no argv, environment or secret configuration.
`task_output_limit_bytes` bounds the aggregate actual output bytes. Each selected output retains
its own ceiling; adding another output cannot shrink an existing promise or increase the aggregate
reservation. An omitted value from an older server
is unknown (decoded as zero), so clients cannot invent an allocation before refreshing metadata.
`project_roots` retains the scoped native-root projection for earlier explicit tool definitions.
Ordinary project setup uses the bounded project directory operations below; choosing a folder never
grants access. Native paths still pass the ordinary Rust path and authority checks.

The browser learns the stable daemon identity through this authenticated bootstrap, then sends it
in `x-milkdrift-daemon` on subsequent requests. First-use trust comes from the supplied endpoint and
transport policy; discovering an identity does not independently verify the endpoint.

The browser saves the endpoint and learned identity together and compares the identity before
loading work on reconnect. A supplied mismatched pin returns `daemon_identity_mismatch` with HTTP
409. The client clears active state and requires the user to forget the saved connection and
connect afresh; it never transfers old drafts or results to the replacement. The server header is
optional for independent native/JSON callers, and supplying it enables the same mismatch check on
task and metadata routes. Rust callers can use `ControlClient::with_daemon_identity` to bind their
requests. Keep credentials in memory and out of URLs, logs and browser storage. All closed requests
reject unknown fields and duplicate keys. The bounded JSON reader limits bytes, nesting and
collection sizes before dispatch.

Success bodies use the common envelope:

```json
{"protocol":{"major":3,"minor":6},"request_id":"req-1","value":{}}
```

Errors use a stable code and redacted explanation. They never include credentials or task content:

```json
{"protocol":{"major":3,"minor":6},"request_id":"req-1","code":"conflict","message":"workflow generation changed","retryable":false,"details":{}}
```

`unauthenticated` means the credential failed, `unauthorized` means the current grant does not
permit the action, and `not_found` means no permitted retained object is available. `conflict`
requires inspecting current state before choosing a new request. Capacity/overload refusals are
explicit; retryability never authorizes changing or blindly repeating a semantic action.

## Shared daemon name

The daemon owns one display name; clients have no competing local rename operation. Duplicate
names are permitted, so connection selectors also show the endpoint. The stable installation
identity remains separate from the name and continues to bind requests, receipts and cursors.

| Method and route | Purpose |
| --- | --- |
| `GET /v1/daemon/metadata` | Read `daemon_id`, `name`, `generation` and the current actor's `can_rename` permission. |
| `POST /v1/daemon/rename` | Submit `request_id`, `expected_generation` and the proposed `name`. |
| `GET /v1/daemon/rename-requests/{request_id}` | Recover this authenticated actor's accepted rename. |

Metadata and receipt reads use the authenticated session negotiation permission; an inspector can
read the shared name without configuration administration. A rename requires `RenameDaemon` and
the `daemon_configuration` resource fact within configuration scope. Only the Controller preset
includes that operation; Supervisor workflow editing does not imply it. Both daemon roles expose
the same naming boundary. Clients display the returned permission but the daemon checks current
authority on every mutation.

The Rust metadata owner trims surrounding whitespace, requires 1–80 Unicode scalar values after
trimming, and rejects control characters anywhere in the submitted name. Operator configuration
`name` seeds absent metadata; omission supplies `Milkdrift`. Startup never overwrites an accepted
rename with the seed. Generation starts at one, independently of any workflow generation.

Acceptance atomically stores the new name and an actor-scoped receipt containing `request_id`,
`daemon_id`, `name` and `generation`. Exact canonical request comparison occurs before stale
generation or capacity rejection. The canonical request retains the untrimmed submitted name:
changing spaces in that value is changed reuse, even when both names would normalize identically.
Replaying an earlier acceptance returns that receipt without undoing a later rename. After a lost
reply, query its receipt or replay every original field; do not substitute the current generation
or allocate another request identity. The CLI's `daemon name`, `daemon rename` and
`daemon rename-receipt` use these same operations.

Rename request identities contain 1–512 UTF-8 bytes without control characters. Spaces and Unicode
are preserved; encode the complete identity as one URL path segment for receipt lookup. The exact
identities `.` and `..` are refused before mutation because URL path normalization would prevent
unambiguous recovery.

The store retains at most 1,024 accepted rename receipts, with at most 16 KiB of combined canonical
request and saved result data per receipt. A full receipt family refuses new renames with a
nonretryable `overload`; name reads, receipt reads and exact replay remain available. Accepted keys
are never evicted. Another actor cannot retrieve this actor's receipt, even with the same request
identity. A generation conflict leaves the client's draft available for a deliberate new request.

## Projects and execution defaults

A project is a daemon registration of an ordinary user folder and defaults for future tasks.
Its name is presentation; its stable ID and pinned native folder identity govern association and
execution. These operations never rename, delete or change ownership of existing user files.

| Method and route | Purpose |
| --- | --- |
| `GET /v1/projects?limit=50&cursor=` | List only authorized project registrations. |
| `GET /v1/projects/{project}` | Read name, generation, folder, settings, availability and management permission. |
| `GET /v1/project-directories?parent=&limit=50&cursor=` | Browse permitted daemon folders; omitted parent lists permitted roots. |
| `GET /v1/project-environments` | Read compatible configured host/process/Podman choices and availability. |
| `POST /v1/project-commands` | Submit an exact actor-scoped registration command. |
| `GET /v1/project-receipts/{request}` | Recover this actor's accepted result or unresolved creation. |
| `GET /v1/project-operations` | List this actor's bounded unresolved creation intents. |

Project pages use opaque actor/grant-bound cursors and at most 200 items. Directory pages are live
observations, not snapshots: each page examines at most 1,024 entries and a folder enumeration ends
at a 65,536-entry refusal boundary. Outside-scope paths and inaccessible project names/counts are
not disclosed. Native aliases resolve before registration; a second registration of the same physical
directory is refused. Missing, moved or replaced roots remain registered but become unavailable.
Dedicated project folders cannot contain the daemon's protected store.

Every mutation has `request_id` and a tagged `command`. For example, attaching existing source
requires no filesystem write:

```json
{"request_id":"attach-source-1","command":{"kind":"attach","name":"Animation","folder":"/home/operator/projects/animation","settings":{"environment":{"kind":"host"},"allow_changes":false}}}
```

`create` instead supplies `parent` and one `folder_name`, plus the same `name` and `settings`.
`rename`, `settings` and `remove` supply `project_id` and `expected_generation`; `rename` supplies
the new name and `settings` supplies the complete replacement defaults. `associate` adds
`workflow_id` under the same project guard and requires that workflow's edit authority. An associated
workflow cannot silently move to another project. Removal refuses any retained workflow reference.

Settings separate tool location from the task's model. `environment` is `{"kind":"host"}` for bounded
native file tools, or `{"kind":"process","capability":"…","operation":"…","generation":1}` for a
configured project-aware host process; `podman` carries the same exact selection fields for a
configured worker with verified `/project` attachment. Available choices come from the existing
capability owner. Fixed-directory profiles and generic installation lifecycle operations are not
project environments. `allow_changes:false` supplies no write tools and refuses mutation-capable
trusted host processes. Current grants still constrain every effective selection.

Creation first reserves a bounded exact operation, a registration and a recovery receipt. Linux
creation uses exclusive one-child `mkdirat` relative to a pinned existing parent, then synchronizes
the child and parent before saving the inspected identity. There is no filesystem/database atomicity
claim. A lost or failed effect may leave `status:"unresolved"` with `operation_id`, `operation_folder`
and `operation_name`; exact replay returns that intent without retrying creation. After inspection,
submit `{"kind":"reconcile","operation_id":"original-request-id"}` under a new exact request to attach
that original child, or `{"kind":"abandon","operation_id":"original-request-id"}` to abandon registration.
Neither operation deletes the folder. Unsupported creation platforms retain an explicit unresolved
operation rather than pretending a directory was created.

The store bounds registrations at 1,024 and exact operations at 8,192; one command is at most 32 KiB.
Unresolved creations reserve their reconciliation capacity, so ordinary capacity exhaustion cannot
consume their recovery slot. Receipts preserve the originally accepted project result after renaming,
settings changes or registration removal. Changing a request under an already used key conflicts.

New ordinary workflows supply `project_id`; earlier adopted workflows remain discoverable with
`?earlier=true`. `?project_id=…` filters one project. A disclosed `project_id:null` can also mean the
actor may inspect the workflow but lacks current project scope; the earlier filter tests the real
association without exposing inaccessible project identities. Organizational association
changes defaults for later submissions only: old definitions, frozen paths/tools, inputs and receipts
are not rewritten. Old request decoding permits exact receipt lookup before enforcing the new
creation requirement.

Each newly submitted task uses `execution:{"kind":"project","expected_generation":1}` to resolve the
reviewed defaults. Alternatively `kind:"explicit"` supplies `expected_generation`, complete `settings`,
`access` (`none`, `read` or `write`) and `tools`; explicit empty tools/access stay empty. New typed
selections cannot also supply the separate legacy `tools` field. Inherited project settings use the
whole project root. An explicit file-only selection may narrow `paths` to permitted directories
inside that root; process execution requires the whole selected project directory. Accepted reads
expose `kind:"bound"` and the effective project identity, generation, folder, access and tools. Text-only queued edits may
retain that exact binding unchanged; new tasks cannot manufacture bound provenance. Started tasks
retain their frozen selection. The shared control owner applies these rules to model-authored work
as well, and storage checks the reviewed project generation atomically with new task acceptance.

When workflow inspection is permitted but the project folder is outside the actor's current scope,
task reads remain available with `execution:{"kind":"restricted"}` and no path or tool details.
Other task content and artifacts keep their independent disclosure checks. The restricted marker is
a read projection, not an empty permission choice: submitting it for authoring is refused.

## Ordinary task routes

| Method and route | Purpose |
| --- | --- |
| `GET /v1/workflows?search=&limit=50&cursor=` | Discover permitted workflows without a known ID. |
| `POST /v1/workflows` | Atomically create an empty workflow or assign IDs to up to 32 supplied task aliases. |
| `GET /v1/workflows/{workflow}?cursor=` | Read workflow state and a bounded page of task summaries. |
| `POST /v1/workflows/{workflow}/changes` | Apply one generation-guarded atomic edit batch. |
| `GET /v1/workflows/{workflow}/tasks/{task}` | Inspect a selected task, frozen start and result preview. |
| `GET /v1/workflows/{workflow}/tasks/{task}/result` | Read authorized result metadata and a text preview. |
| `GET /v1/workflows/{workflow}/tasks/{task}/result/content` | Download complete bounded UTF-8 result text. |
| `GET /v1/workflow-requests/{request}` | Recover this actor's durable request acceptance. |
| `POST /v1/task-inputs` | Capture an actor-owned UTF-8 file. |
| `GET /v1/workflows/stream` | Receive authenticated snapshot invalidation and daemon-name generation hints. |

The following request creates a finite project-aware process task. Replace the project identity,
reviewed settings generation and capability with exact choices returned by the connected daemon.
Select the compatible process as that project's environment and explicitly permit project changes.
The daemon assigns workflow/task IDs and returns the `report`
alias mapping; the alias has no identity outside this request.

```json
{"request_id":"project-report-1","project_id":"project:replace-with-project-id","title":"Inspect project","description":"Report the supplied Cargo project","tasks":[{"alias":"report","definition":{"execution":{"kind":"project","expected_generation":1},"title":"Inspect Cargo manifest","description":"Describe targets and dependencies","instructions":"Inspect the supplied file and summarize its package, targets and dependencies.","capability":{"id":"cargo-report","operation":"process.execute"},"inputs":{"brief":{"kind":"supplied","name":"brief","optional":false},"file":{"kind":"supplied","name":"file","optional":false}},"outputs":{},"dependencies":{},"required":true,"corrective_of":null,"paths":[]}}],"inputs":{"brief":{"kind":"text","text":"Give a concise project report."}},"finish_when_settled":true,"limits":null}
```

This task remains queued because `file` is missing. Capture file bytes with:

```json
{"request_id":"manifest-upload-1","name":"Cargo.toml","media_type":"text/plain","text":"[package]\nname = \"sample\"\nversion = \"0.1.0\"\n"}
```

A supplied `{"kind":"absent"}` value also keeps a required binding queued until an authorized
input edit supplies a value. Optional consumers may start with that same explicit absence;
each consumer's binding decides readiness. Empty supplied text remains a value, not absence.

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

Changes include `add`, `revise`, `remove`, `bind_input`, `rename`, `stop`, `wait`, `continue`, `finish`,
`adopt_correction`, `set_limits` and `dismiss_admission`. Revise/remove carry the observed task version as well as the batch's workflow
generation. An already frozen start refuses revision. New definitions never rewrite retained
history. Required producer output and settled producer outcome are different input bindings;
missing required output keeps consumers queued. Definitions default to required.

`wait` pauses new external entry until an authorized `continue` command. Supplying inputs does not
answer that decision. A singleton `wait`, `continue` or `stop` accepts an observed generation at or
after the last edit, new start or wait decision, up to the current generation. Progress and retries
therefore do not obstruct those controls. Human edits use the same decision watermark, so ordinary
progress does not conflict with an unrelated edit. New starts, authoring decisions and changed task
versions still refuse stale edits. Model-authored batches retain their causal generation checks.
A request predating a new decision refuses; replay of an earlier accepted Continue returns its exact
receipt without clearing a later question.
The browser submits Pause, Continue and Stop independently of unfinished authoring changes.

A queued task can retain its frozen start when paused or waiting for project access. Its definition,
inputs and capability selection remain fixed. `started_at` records that acceptance, not physical
adapter entry. Task summaries expose `entered`, matching selected-task `frozen.entered`; it denotes
durable entry intent and is not terminal effect evidence. Known native-path overlap reports
**Project is in use** without naming another actor's work. The daemon retries within its bounded
scheduler, preserving the same start and rechecking authority, generation and folder identity.

Stop uses the task's returned `stop_request_id` with one exact `stop` change. Its reserved capacity
remains available when ordinary admission is full. A cancellation request alone does not establish
stopped: the public task can remain running/stop-requested or uncertain until durable evidence
settles it. An entered process without terminal evidence remains uncertain after restart and is
not silently repeated. Native writer claims remain held while its outcome is uncertain.

Correction adoption names a successful corrective task and optional version-guarded future input
redirects. The original failure/result remains inspectable. Each referenced output must be declared
by its producer, including optional outputs. A required-success binding cannot consume a failed
producer's partial output merely because a sibling binding accepts its settled outcome.

Set a model task's `actions` to `{"automatic_repair":false}` to enable structured future-work
proposals. The daemon supplies a bounded authoring snapshot and requests a canonical `TaskBatch`;
plain result text never becomes commands. The immutable response and pending admission are retained
before control processes it. Accepted changes, source linkage and alias receipt commit together.
A restart or lost reply recovers that admission without issuing new children.

The canonical response is named `model_response`, with media type
`application/vnd.milkdrift.model-response.v1+json`. A structured-only response can have no
`final_text`; an action task that requires a named result should declare `model_response` instead.
Ordinary text responses may separately publish `final_text` with media type `text/plain`.

Human edits and task starts invalidate stale model snapshots. Progress reports do not invalidate the
source task's own snapshot. Invalid/conflicted output retains a bounded decision reason. With
`automatic_repair:true`, control may admit an ordinary feedback task using the same capability,
tools and account; denied authority waits for a person. Workflow reads expose decisions for source
tasks on their page, and selected task reads expose their admitting origin independently of paging.
Use a reviewed future batch with `dismiss_admission` naming the rejected source task to close its
decision. Dismissal does not execute the rejected output or broaden capability authority.

`set_limits` supplies a complete limits object prospectively. Task, model-batch and elapsed-time
ceilings cannot fall below their retained consumption; increasing them never changes the underlying
grant. Continue does not replenish tasks, model batches, the rejected-response count, failed-lineage
spending or unresolved time. An accepted model batch that adds work resets the rejection streak;
automatic feedback itself does not. At a model-batch ceiling, already admitted work can run, but
another proposed batch is refused. Explicit waits and undecided admissions prevent finish, even
when every external task has settled. Feedback and dismissal may clear their own rejection wait;
they do not clear a separate pause accepted by a person.

For a complete explicit selection, `execution.tools` may contain `{"kind":"read_file"}`,
`{"kind":"write_file"}` or
`{"kind":"process","capability":{"id":"configured-tool","operation":"process.execute"}}`.
Only model tasks may use this catalog. Inherited project settings resolve it from the reviewed defaults.
Native file tools use the accepted project access and optional narrower `paths`; process tools must
match the selected configured environment. An explicit direct process task selects an empty tool
list because the process is its executor. Earlier accepted definitions retain their separate `tools`
and `paths` encoding. The host rechecks authority at every model/tool call,
retains bounded feedback and uses one cumulative task reservation. Generic binary model inputs remain
unsupported; captured text and supported textual artifacts carry exact bounded content.

`active_execution_ms` is a shared cumulative allowance. Before external entry, the daemon reserves
the adapter's enforced attempt duration alongside all live reservations and previously observed
elapsed time. A task whose bound cannot fit is refused before entry, even if it might finish early.
Preparation without an enforceable duration is also refused. Concurrent clocks consume the same
allowance; restart and uncertain outcomes retain the open account. Reaching the limit requests a
stop, but only terminal evidence can establish that execution stopped. A client HTTP deadline is
not proof that remote physical work has terminated.

## Reads, pages and streams

Discovery searches workflow/task titles, descriptions and authorized retained text results. Each
request examines a finite window and returns `truncated` and `next_cursor`; absence from one page
is not proof of absence. Cursors bind the daemon, actor, grant and search. Workflow task pages use
`next_task_cursor` and expose `task_count`; they contain summaries, not captured inputs. These are
live pages, not a historical snapshot transaction. Both item counts and encoded page bytes are
bounded. Refetch from the beginning after edits when a consistent current view matters.

A selected task contains its definition and frozen input provenance. Frozen text and combined
result text have explicitly labelled 16 KiB previews; their digests refer to complete content.
When a model publishes named `final_text`, the result preview and complete text download put that
answer first, followed by the remaining textual artifacts in retained order. Canonical response and
tool evidence remain available; clients do not parse provider JSON to extract a different answer.
`result_restricted` preserves permitted task-state inspection while indicating inaccessible output.
The explicit result route still refuses unauthorized output reads. Captured inputs also recheck
current artifact permissions; restricted previews and digests are withheld. Non-UTF-8 declared text
sets `text_unavailable` while retaining task state, artifacts and the complete byte digest. Complete
result text permits 2 MiB of artifact content plus bounded separators; individual immutable artifacts use `/v1/artifacts/{artifact}/content` and bounded ranges.

The workflow stream uses authenticated fetch, not native EventSource with a token in its URL.
It sends bounded `workflow_changed` invalidations; clients coalesce the affected workflow reads.
A separate `metadata_changed` hint carries only the changed name generation, even when no workflows
exist or discovery fails. Periodic session/catalog refresh handles other connection changes without
reloading those catalogs for every workflow hint. A checked name generation cannot replace newer metadata,
and late responses after disconnect or replacement are discarded. It is not a replayable task
journal. The connection has a finite lifetime. On reconnect,
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

Saved browser profiles retain endpoints, identity pins and local selection IDs. Older profiles keep
their label only as a disconnected cached name until authentication replaces it with the daemon's
name. Disconnected names are labelled as cached. A failed cache write is reported separately while
the authenticated session continues to show the current daemon name; browser labels never become
rename commands. The browser retains no credential in that profile.

[Current versions](versions.md) are checked against their owning constants.

## Canvas source reads and authoring guards

An empty workflow is discoverable immediately and cannot become completed before any task has
been admitted. Save and arrange operations do not admit tasks. The session advertises
`authoring_bounds`: 2 MiB of document data, 200 pending cards, 400 sources, 1,024 connections,
600 positions and at most 32 actions per deliberate Run/Apply. These limits describe one latest
snapshot; clients do not append receipts for keystrokes or pointer movement.

| Method and route | Purpose |
| --- | --- |
| `GET /v1/workflows/{workflow}/authoring` | Read the latest authorized document, generation and content digest. |
| `PUT /v1/workflows/{workflow}/authoring` | Replace that document using `expected_generation`; no task admission. |
| `GET /v1/workflows/{workflow}/positions` | Read separately versioned card positions. |
| `PUT /v1/workflows/{workflow}/positions` | Conditionally replace positions without changing semantic generations. |
| `POST /v1/workflows/{workflow}/authoring/review` | Resolve selected cards/change indices, expose required new predecessors and bounded local repairs. |
| `POST /v1/workflows/{workflow}/authoring/run` | Accept exactly the reviewed scope with an exact request identity and atomically update its draft aliases. |
| `GET /v1/workflows/{workflow}/topology?cursor=` | Read up to 200 tasks' connections, order and output metadata without instructions or result bodies. |

The authoring document holds incomplete task fields, source cards, wire intentions, other explicit
changes, a usual-model preference and daemon-owned accepted aliases. Accepted definitions live only
in workflow history. Wires retained after admission preserve presentation references; accepted
topology remains authoritative. A candidate edit projects its complete incoming connections before
Run. Required new predecessors are included in review; an existing predecessor with candidate edits
requires an explicit `use_current` choice or selection of that candidate. Inspecting a card never
changes this scope. Incomplete included work blocks the whole submission.
A source with a `shared_name` and null `value` deliberately waits for that workflow input to be
supplied later. With both fields null, the source is unfinished and included work cannot run.

Checkpoint conflicts return the current generation and preserve the last saved document. After a
lost reply, read current content and compare it to the attempted checkpoint; do not repeat an old
conditional write as though it were an exact command. Run uses ordinary canonical receipt recovery,
even after another client saves later changes. Run checks both authoring and semantic workflow
observations. `details.authoring_issues` is a JSON array of at most 64 typed repairs identifying
affected cards, connections, sources or changes. Transport error code and retryability remain
independent of those repair hints.

Latest authoring and position bytes count against workflow capacity, with bounded replacement
headroom so shortening/removing saved content remains possible. Retained source identities are
protected and reauthorized before disclosure or admission. One latest document protects at most
400 distinct retained artifacts, including files and named results; extra references refuse with
an explicit request to remove unused sources or result connections. Drafts do not expire by age. Topology
pages share a 64 KiB source-preview budget and a 2 MiB response limit; cursors bind workflow
generation, actor, authority and daemon identity. A changed generation refuses mixed-page assembly.
An explicit `missing` topology preview means the supplied value is absent. A null preview means
no preview was included, for example because the page exhausted its byte budget; clients must not
infer absence or permission from it. Restricted and expired previews retain those distinct states.
Another authorized editor can retain an exact source already owned by this workflow. Introducing
a new uploaded input still requires its uploader's scope. A client with a known checkpoint can
remove revoked sources even when current authority prevents reading or retaining the old document.

New `dependencies` maps encode independent `success` or `settled` order. Data bindings express
their own readiness: a successful named output, optional settled absence, or an explicit settled
outcome. Removing a data wire does not invent independent order. Historical array dependencies
retain their original predicates; their topology projection marks `dependencies_legacy` and exposes
the effective conditions. Fresh definitions must use maps, including an empty `{}`. Known media
mismatches are refused by shared admission, and adapters validate actual bytes at entry.
Saved-authoring admission declares the reviewed adapter's named outputs before any consumer is
connected. A later consumer can therefore wait on an accepted queued task's result without silently
editing that producer. An explicitly selected producer edit can introduce a new promise in the same
atomic batch as its consumer; existing stricter declarations remain intact.

Workflow reads include bounded `supplied_inputs` previews from the daemon-owned shared values.
Selected task reads include the currently supplied values referenced by that task, independently
of its `frozen.inputs`. Each map value distinguishes available, restricted and expired sources;
an absent key means a value has not been supplied. Text previews share a 64 KiB budget and each
value is bounded by the existing 16 KiB task-input allowance. A truncated preview is explicitly
marked and cannot be treated as the complete source for replacement.
`TaskRead.inline_inputs` projects inline values and captured-upload provenance from the owning task
definition under the same bounded, authorized read rules. A fresh client uses it to reconstruct
inline source cards; `frozen.inputs` remains the separate evidence of what a started task consumed.

Task results retain the legacy artifact list and add `outputs`, keyed by actual named publications.
Each accessible entry includes the exact artifact, retained media type and byte count. A result
preview is presentation only and is never an input artifact. An output need not have been declared
in the historical producer definition; future binding admission still validates the retained
publication, outcome rule and current authority through the shared task path.
Selecting a settled outcome requires current access to every artifact it contains, including
terminal-result references. Review and admission recheck those permissions when publication follows
an earlier saved connection; an unpublished future outcome remains a valid prerequisite.

A new or revised task may include `authoring_generation` to bind its composition choices to the
observed registered generation. A stale or unavailable contract refuses the entire request before
acceptance. The guard is stored with the accepted definition and checked before start; a changed or
unavailable capability leaves the task queued for an explicit review rather than selecting a replacement. Exact accepted request
replay is resolved before rechecking authoring metadata, so upgrading an adapter does not invalidate
old receipts. Omitting the guard preserves the existing programmatic request shape.
