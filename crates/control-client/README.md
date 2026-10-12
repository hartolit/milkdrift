# Control client

Use `ControlClient` to operate a daemon from Rust. It sends authenticated HTTP requests, decodes
the [control protocol](../control-protocol/README.md), and exposes bounded reads. The CLI adds
arguments, deadlines and output presentation; neither client owns workflow execution or persistence.

Construct a `ClientConfig` for the daemon origin and a `BearerCredential` from a private source,
then call `negotiate`. Construction itself performs no I/O. Replies reject an incompatible protocol.
The daemon maps the credential to an actor and grant; each operation checks the scope it needs.

## Ordinary tasks

`task_session` reports the installation identity, current actor, effective limits and task choices.
`upload_task_input` captures UTF-8 file content; `create_workflow` submits instructions and named
inputs. Acceptance returns daemon-assigned identities and an alias map. A file input identifies
captured bytes, not a path that can later change under the worker.

`workflows` performs paged discovery and search. `workflow` returns the first bounded task-summary
page and `workflow_page` accepts its continuation. Read one selected task through `task`; fetch its
retained result through `task_result`. A bounded text preview exposes its truncation flag. Artifact
metadata and bounded ranges remain the complete immutable download path. A task read may expose
lifecycle metadata while marking result content restricted under the caller's current grant.

`change_workflow` sends a generation-guarded atomic batch. Started task definitions remain frozen;
corrections are separate future tasks. `workflow_receipt` recovers exact durable acceptance after
a reply is lost. Mutations do not retry automatically. Replaying the same exact request returns
its acceptance; changing a request under its old ID conflicts. A fresh client can authenticate and
rediscover work without a saved recovery file.

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
