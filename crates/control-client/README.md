# Control client

Use `ControlClient` to operate a daemon from Rust. It sends authenticated HTTP requests, decodes
the [control protocol](../control-protocol/README.md), and exposes bounded reads. The CLI adds
arguments, deadlines and output presentation; neither client owns workflow execution or persistence.

Construct a `ClientConfig` for the daemon origin and a `BearerCredential` from a private source,
then call `negotiate`. Construction itself performs no I/O. Replies reject an incompatible protocol.
The daemon maps the credential to an actor and grant; each operation checks the scope it needs.

## Ordinary tasks

Use `projects`, `project`, `project_directories` and `project_environments` to discover current
authorized project choices on the daemon. `project_command` performs attach/create, rename/settings,
association and unused-registration removal. Preserve the complete request for `project_receipt`
or exact replay. `project_operations` exposes this actor's unresolved creations; explicitly reconcile
the original intended folder or abandon its registration without deleting any files.

New workflow requests select `project_id`, and each new task deliberately chooses reviewed project
settings or a complete explicit selection. Empty explicit access/tools stay empty. Authorized task
reads return their effective binding, which a text-only edit can retain unchanged. If current
project scope does not permit its disclosure, the read instead carries `Restricted` and omits
execution paths/tools; that projection cannot be reused as an authoring definition. Use
`workflows_in_project` for one project's workflow page or the earlier unassociated workflow page.

`task_session` reports the installation identity, shared daemon name and name generation, current
actor, rename permission, effective limits and task choices. After first authentication, use
`with_daemon_identity` to bind subsequent requests to the learned identity. Retain that pin with
the endpoint across reconnects; do not silently replace it when the endpoint serves another daemon.
The name is presentation metadata and never replaces the stable identity.
`upload_task_input` captures UTF-8 file content; `create_workflow` submits instructions and named
inputs. Acceptance returns daemon-assigned identities and an alias map. A file input identifies
captured bytes, not a path that can later change under the worker.

`workflows` performs paged discovery and search. `workflow` returns the first bounded task-summary
page and `workflow_page` accepts its continuation. Read one selected task through `task`; fetch its
retained result through `task_result`. A bounded text preview exposes its truncation flag. Artifact
metadata and bounded ranges remain the complete immutable download path. A task read may expose
lifecycle metadata while marking result content restricted under the caller's current grant.

Use `workflow_authoring` and `checkpoint_workflow_authoring` to read and conditionally replace a
workflow's latest unfinished canvas. The position methods version layout separately. These writes
do not have command receipts and are not automatically retried: after an uncertain reply, compare
current content before deciding whether to submit the newer candidate. `review_workflow_authoring`
exposes repairs and required predecessors; `run_workflow_authoring` uses an exact canonical request
and the ordinary receipt recovery path to admit only the reviewed scope.

`workflow_topology` reads accepted ports, data connections and exact ordering conditions without
loading task instructions or result text. Its continuation is separate from the task-summary cursor
and binds the workflow generation as well as the daemon and current viewing authority. Continue
while `truncated` is true; if that generation changes, reload the connection view. A null input
preview means it was not included, not that the source is absent. Fetch the selected source/task
when an action needs its exact content rather than copying a bounded display preview.

`change_workflow` sends a generation-guarded atomic batch. Started task definitions remain frozen;
corrections are separate future tasks. `workflow_receipt` recovers exact durable acceptance after
a reply is lost. Mutations do not retry automatically. Replaying the same exact request returns
its acceptance; changing a request under its old ID conflicts. A fresh client can authenticate and
rediscover work without a saved recovery file.

`daemon_metadata` reads the current shared name and this actor's separate rename permission.
`rename_daemon` sends one generation-guarded `RenameDaemonRequest` through the same daemon command
as the GUI. `daemon_rename_receipt` recovers the actor-scoped accepted result after a lost reply.
Preserve the whole request for exact replay; changing its name or generation under the same identity
conflicts. Current metadata and a historical receipt can have different generations after another
client renames the daemon. A rename requires daemon configuration authority, not workflow editing.

Other languages use the same [HTTP API](../../docs/reference/control-api.md). The
[independent JSON integration test](../../apps/daemon/tests/control_plane/independent_client.rs)
drives actual daemon task creation, captured input, replay, restart and output reads. The standalone
[web client](../../apps/web/README.md) supplies browser authentication and fetch-based task SSE.

## Independent host operations

`execution_discovery` and `prepare_invocation` expose configured independent capabilities in either
daemon role. Save the prepared reply before `invoke`; retain it exactly for lost-response recovery.
`invocation_lookup`, `invocation`, `invocation_observations` and `cancel_invocation` inspect and
control that caller-bound invocation. `upload_input` publishes bounded, verified content without
accepting a client-selected host path or producer claim.

Artifact downloads read metadata, assemble bounded ranges and verify the complete size and digest.
The [CLI download owner](../../apps/cli/src/command/artifact.rs) demonstrates that contract.
Capability, authority, peer and managed-resource operations remain separately authorized.

## Bound observation and recovery

Safe JSON reads retry classified transient failures up to `safe_query_retries`; every attempt has
a timeout. Add an outer deadline to bound a sequence of operations. Negotiation, mutations, peer
actions and artifact ranges do not use that safe-read retry loop. `ClientError::retryable` does not
prove that a command was unaccepted or an external effect absent.

The generic `subscribe` API owns bounded health/capability stream decoding and reconnection. Treat
`ResyncRequired` as a request for a fresh authorized snapshot and a new subscription without the
old cursor. Replace the entire capability catalogue on `CapabilitySnapshot`, including an empty
one. Dropping a subscription stops local observation and leaves daemon work running. HTTP feed
incarnation changes invalidate their cursors; an SSE cursor does not replace a durable task receipt.

The [crate API](src/lib.rs) owns exact defaults and method behavior. The
[control reference](../../docs/reference/control-api.md) owns transport bounds and error categories.
