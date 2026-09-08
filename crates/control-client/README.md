# Control client

Use `ControlClient` to operate a running daemon from Rust. It sends authenticated HTTP requests,
decodes the [control protocol](../control-protocol/README.md), and exposes bounded reads and
resumable observations. The CLI uses this same client; it adds argument handling, deadlines,
confirmation, and output presentation.

Create a `ClientConfig` for the daemon URL and a `BearerCredential` from a private source, construct
the client, and call `negotiate` before other operations. Construction does not contact the daemon.
The server maps that credential to an actor and grant; possessing it does not authorize every
operation. The [daemon guide](../../docs/operations/daemon.md) explains the matching configuration.

## Choose the operation

| Need | Client operation and consequence |
| --- | --- |
| Change workflow or run state | `submit` sends one `CommandRequest` and does not retry it automatically. Retain the exact request to recover a lost reply. |
| Inspect current or past work | `run`, `node`, `attempt`, and `timeline` expose authorized views. A compact run does not contain every historical attempt. |
| Browse definitions or proposals | `revisions`, `runs`, and `proposals` return one page per call. Reuse its cursor with the same filters. |
| Inspect available execution hosts | Capability, peer, and authority reads expose the caller's permitted view. `peer_action` explicitly changes a configured relationship. |
| Download output | Read artifact metadata, then fetch ranges. Verify the assembled file's size and digest as the CLI's [download owner](../../apps/cli/src/command/artifact.rs) does. |
| Observe changes | `subscribe` yields observations and recoverable transport errors. The consumer owns its overall deadline and reconnection limit. |
| Read presentation state | `layout` reads an exact workflow/revision association; writes use `submit`. |

## Recover a lost connection

Safe JSON reads retry retryable failures up to `safe_query_retries` after the initial call. Each
attempt has its own timeout; add an outer deadline when the whole operation must finish by a
particular time. Negotiation, command submission, peer actions, and artifact ranges do not use
that automatic read retry loop.

A command timeout can occur after durable acceptance. Repeat the same command ID, body, reason,
evidence, and guards under the same actor/grant to recover its result. A fresh ID expresses a new
command and can cause new work. `ClientError::retryable` is an error classification, not evidence
that the first command was unaccepted or an external effect did not happen.

`subscribe` reconnects with the last cursor it decoded and yields errors to the consumer along
the way. That cursor is not an acknowledgement that the consumer persisted the observation. A
consumer resuming after its own restart must save a cursor only after handling its item. Treat
`ResyncRequired` as a request for a fresh view, and choose how to respond to `StreamClosing`.
Malformed/truncated frames and nonretryable API errors end the subscription. Dropping the stream
stops local observation; it does not cancel a run.

The [crate API](src/lib.rs) owns defaults and individual method behavior. The
[control reference](../../docs/reference/control-api.md) owns exact feeds and error categories;
the [CLI guide](../../apps/cli/README.md) shows the operator-facing use of the same calls.
