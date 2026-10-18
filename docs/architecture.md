# Architecture

Milkdrift owns durable tasks and the authority to advance them. A browser, CLI, person or agent
submits the same scoped commands to a daemon. The daemon decides when inputs and dependencies
permit a task to start, records that start, and delegates external work to a configured capability.
Closing a client changes neither the accepted work nor its recorded outcome.

[Vision](product/vision.md) owns product intent. [ADR 0051](decisions/0051-daemon-task-workflows.md)
owns the task-workflow decision; [status](product/status.md) distinguishes the implemented slice
from the later methods and retention assignments. The ordinary graph, revision,
proposal, controller and published-workflow engines have been retired. Existing stores are not
converted into task workflows. Their readers remain in the preservation checkpoints.

## Terminology

A **workflow** is the daemon-owned collection of task obligations, supplied inputs and cumulative
limits. A **task definition** describes future work: instructions, a configured capability,
named input bindings, dependencies, outputs and optional filesystem claims. Queued definitions
can be revised using exact version guards; their prior accepted definitions remain in history.

A **start** freezes the accepted definition, selected input bytes, capability generation and
authority. An **entry intent** records that an external invocation is about to cross its final
entry boundary. A **result** is the retained terminal evidence and its immutable output artifacts.
An acknowledged stop request is a separate fact from evidence that the invocation stopped.

The public API allocates workflow and task identities. Client aliases are temporary names within
one atomic request. Mechanical `RunId`, `RevisionId`, `NodeId` and attempt coordinates still carry
frozen task provenance through independent capability and peer contracts; they do not identify
an alternate graph scheduler.

## Owners and dependency direction

| Owner | Responsibility |
| --- | --- |
| `blueprint` | Task definitions, input/dependency/output contracts and finite workflow limits. |
| `authority` | Scoped grants, resource facts and current authorization decisions. |
| `control` | Shared authenticated admission of create and prospective edit commands. |
| `runtime` | Task readiness, frozen starts, entry, reports, cancellation, completion and recovery. |
| `persistence` | Atomic workflow and daemon-name receipts, artifacts, native claims, managed and peer storage ports. |
| `redb-store` | Durable transactions, bounded accounting, integrity and fresh-format refusal. |
| `capability` and `workspace` | External operation contracts, immutable artifacts and causal identities. |
| `capability-host` | Selected generation permits, preparation, execution lifetime, materialization and resource claims. |
| Process/model/peer adapters | Bounded external transport and truthful effect observations. |
| `control-protocol` | Public DTOs, closed readers, bounded encoding and cursor contracts. |
| `control-client`, CLI and Svelte | Transport, presentation, request recovery and explicit client selection. |
| Daemon | Configuration, authenticated owner queue, serving, health and shutdown composition. |
| `tools/evidence` | Rust fixtures and actual-binary observations; no production dependency imports it. |

Dependencies point toward semantic contracts. Adapters do not decide workflow readiness, clients
do not schedule continuation, and views do not own durable facts. Rust owns backend executables,
fixtures and evidence orchestration. Svelte uses TypeScript generated from the public Rust DTOs.

## Definitions and prospective control

Missing required supplied inputs keep tasks queued. Task-output bindings require a declared
producer output; settled-outcome bindings explicitly permit a failed producer's retained outcome.
Explicit dependencies and input dependencies are validated for cycles before acceptance. Display
order, task names and browser selection do not establish execution order.

Changes use the workflow generation and affected task versions. An accepted start makes that
task's definition immutable. Binding a new supplied value affects future starts only. Removing
unstarted work retains its accepted history. Correction adoption names a successful corrective task
and explicit, version-guarded future bindings; it never rewrites the original failed result.

Task admission, model-batch admission and execution allowances are cumulative. Removing a task does
not refund task admission; accepted external entry consumes its reserved allowance. Rejection
streaks, corrective lineages, concurrency and tool calls have their own bounded policies.
Runtime advances one cumulative elapsed-time account across simultaneous open attempts and
reserves each adapter's enforced duration before entry. Unknown outcomes retain their reservation
and clock; a limit stop is not terminal proof. Task document schema 3 retains those watermarks and
model admission linkage; redb internal format 22 refuses earlier documents rather than inventing
missing evidence. Saved method expansion remains later work in the [roadmap](product/roadmap.md).

An opted-in model task's accepted response output and pending admission are one durable transition.
Control reads the immutable structured batch after successful settlement and uses the same checked
future-edit transaction as a person. That transaction also records the source link, alias map and
receipt. Recovery processes pending records before new scheduler polling. Edits and starts change
the authoring generation; the source's progress reports do not. Conflicts retain the response and
require a refreshed proposal. Explicitly permitted feedback is ordinary work under the same account;
denied authority waits for a person. Required-success and settled-outcome inputs remain separate,
including when both refer to the same failed producer.

The capability host owns a bounded model/tool conversation inside an entered task. It uses the
configured provider mapping, exact process generations and declared native path claims, rechecking
authority on each step. It retains response/tool evidence before continuing and never schedules
workflow tasks. Prospective limit edits and resumes preserve spent allowance and uncertain effects.

## One command and external-effect path

```text
browser / CLI / independent JSON client
                  |
      authentication and owner queue
                  |
        control -> authority -> runtime
                  |
       atomic task state and receipt
                  |
 frozen inputs + selected capability generation
                  |
  host preparation -> final authority check
                  |
       durable entry -> external adapter
                  |
        durable reports and artifacts
```

Receipt identity binds the canonical public request before defaults, alias allocation or uploaded
content resolution. Exact replay returns the durable acceptance. Changed reuse conflicts. A client
whose POST reply is lost can inspect the request receipt and discover the workflow without a local
recovery file. A new request identity can create new work and is not an automatic recovery action.

Host preparation and final entry retain the existing generation, admission and lifetime checks.
A task's native filesystem claims are durable read/write claims over resolved paths. Overlapping
writers conflict; uncertain writers retain claims across restart. Captured text is copied into the
frozen start rather than read from a later version of a local file.

### Direct and remotely served operations

Independent direct execution has its own host invocation owner and exact request recovery. An
execution-only daemon does not gain local workflow ownership by receiving an originating task's
provenance. Peer catalogs, authenticated delegation, exact selected generations, bounded transfers
and cancellation acknowledgements remain independent of the task scheduler.

Managed installations keep their own generation guards, editing claims, execution holds, platform
verifiers and uncertainty. Retiring workflow publication does not remove an installation's
protection. Obsolete graph-ancestry transfers are refused; the new task owner does not invent
ancestry to bypass a managed guard.

## Scoped authority and disclosure

Create, edit, inspect, artifact access and external entry each use the authority owner. Persisting
a request does not preserve permission forever: final entry rechecks current authority, and reads
recheck their scope. Discovery examines all permitted workflow scopes without requiring a known
workflow identity. Cursor signatures bind the daemon/feed, actor, grant and query; a cursor cannot
widen a later request's authority.

Browser origins are exact configuration entries. Bearer credentials remain in memory; neither
cookies nor URL tokens provide browser authentication. The standalone static client checks daemon
identity and separates credentials, selection and cached results for each connection.
Non-loopback access requires a browser-trusted HTTPS endpoint; loopback HTTP is the development
exception. TLS termination and origin configuration are deployment responsibilities, not workflow
semantics.

## Daemon connections and names

A first browser connection needs an endpoint and credential. Authenticated session bootstrap returns
the daemon's stable identity and shared name; the client saves the identity with the endpoint and
binds subsequent requests to it. Reconnect checks the saved identity before loading work. A different
installation at the address ends ordinary use and requires deliberately forgetting that connection
before connecting afresh. Discovery is first-use endpoint trust under the existing transport policy,
not a separate identity-verification service. Native/JSON callers may supply the same identity header;
the server continues to accept unpinned callers. A name neither proves identity nor grants authority.

The daemon's private metadata module runs under the existing Owner queue and authorization path.
`DaemonMetadataStore` is its narrow persistence port; redb owns atomic name, generation and exact
rename-receipt storage. `DaemonDisplayName` owns normalization and validation. A configured name,
or `Milkdrift` when omitted, seeds absent metadata once. Later configuration changes cannot undo an
accepted rename. A rename requires Controller's separate `RenameDaemon` operation and configuration
scope, independently of workflow editing, in both daemon roles. Metadata never lives in a workflow.

CLI and Svelte use the same generation-guarded rename command and actor-scoped recovery receipt.
Receipt lookup precedes stale-generation and capacity rejection; changed canonical requests conflict.
The bounded receipt family refuses new names at capacity while preserving current reads and exact
replay. The [control API](reference/control-api.md#shared-daemon-name) owns the public fields and bounds.

The existing two-second invalidation stream coalesces session metadata refresh with workflow reads,
including clients with no workflows. Checked identity and name generations reject obsolete responses;
disconnect clears active state. Names stored in browser profiles are explicitly cached observations.
Legacy profiles preserve their endpoint, identity pin and selection ID, replacing local labels with
the returned name after authentication. Cache failures do not replace current in-memory metadata.

## History, capacity and recovery

Atomic workflow commits retain the runtime's append-only facts and exact command receipt together.
The storage owner reserves space for settlement and mandatory stop actions before ordinary
admission can consume it. Artifact publication proves the entered task invocation in the same
transaction that exposes its metadata. Pending and retained artifact bytes participate in capacity
accounting. A full store refuses new ordinary work rather than silently discarding accepted facts.

Restart restores the durable task records. A queued task can become ready using its accepted
inputs; an entered invocation without terminal evidence becomes uncertain, not successful or safely
retryable. Release of native or managed resource holds requires the owning terminal/physical-stop
proof. A client timeout and a cancellation acknowledgement supply no such proof.

Discovery returns compact workflow summaries. Workflow task lists are explicitly paged; a selected
task is read separately. Large captured text and result text have labelled previews with retained
digests, while complete result content has a separate bounded download. SSE supplies invalidation
hints, not a second event journal: clients refresh authoritative snapshots and resynchronize after
a daemon incarnation changes.

## Daemon lifecycle and compatibility

The daemon owns one bounded request queue and external worker pools. Admission closes during
shutdown before workers drain, cancel or retain uncertainty under the configured policy. The clock
uses one durable high-water owner for request and worker paths, avoiding clock rollback and owner
queue lock inversion.

Unsupported configuration, protocol and store formats are refused explicitly. The current reader
can adopt physical schema 19 only with internal document format 22 and its exact valid table set.
One transaction adds the daemon metadata tables and changes the physical marker to 20 without
rewriting workflow, receipt or installation-identity rows. Earlier graph/task formats still require
a separate fresh root; this narrow addition does not restore their readers or change frozen grants.
Daemon configuration stays at version 14 because the optional name seed preserves existing reads;
control protocol 3.3 requires coordinated client upgrades. Do not stop existing services, reset a
store or repoint an existing installation merely to try the new task client. See
[operation](operations/daemon.md), [authority](operations/authority.md) and the
[public API](reference/control-api.md) for configuration and transport details.
