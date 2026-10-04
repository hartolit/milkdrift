# Operator CLI

`milkdrift` submits commands to a running daemon and presents its authorized results. Start with
the [operator recipe](../../examples/operator/README.md): it builds the applications, configures
authentication, runs a small workflow, and shows process/model setup. The CLI uses the
[control client](../../crates/control-client/README.md); it never opens the daemon's database.

## Choose what to inspect or change

| Task | Command family |
| --- | --- |
| Check connectivity and your grant | `daemon readiness`, `health`, `authority` |
| Author model steps, prompts and connections | `workflow new`, `models`, `add`, `prompt`, `connect`, `output`, `save`, `open` |
| Create and compare immutable definitions | `blueprint validate`, `import`, `show`, `export`, `list`, `diff` |
| Turn implementation prompts into a workflow | `sequence validate`, `compile`, `import`, `show`, `status`, `stage`, `remediate` |
| Start or control work | `run start --request-file FILE`, `reconnect FILE`, `pause`, `resume`, `cancel`, `signal` |
| Observe progress and outcomes | `run list`, `show`, `wait`, `timeline`; `node`; `attempt inspect` |
| Address an unknown external outcome | `attempt resolve`, with an explicit decision and evidence |
| Review a prospective change | `proposal submit`, `list`, `show`, `approve`, `reject`, `apply` |
| Inspect execution options | `capability` and `provider` |
| Manage a configured peer | `peer list`, `show`, `connect`, `reload`, `disconnect`, `drain`, `revoke` |
| Retrieve output or presentation state | `artifact metadata`, `get`; `layout get`, `put` |
| Inspect or continue a controller checkpoint | `controller status`, `continue`; the daemon must explicitly enable its lifecycle |

Use `milkdrift --help` and family help for exact arguments. The
[control API reference](../../docs/reference/control-api.md#cli-automation-contract) owns JSON
output, exits, bounds, and the complete command contract. The
[sequence guide](../../docs/guides/headless-dogfood.md) follows verification and approved repair.

## Use it from scripts

Choose `--json`, explicit command IDs for mutations, and an overall `--timeout-secs`. `run wait`
and start/reconnect `--wait` or noninteractive `--follow` require a deadline. `--yes` supplies local confirmation for commands
that require it; the daemon still checks authority, evidence, and optimistic guards.

A successful start reply means acceptance. `run wait` checks for a terminal result, while attempt
inspection and artifact download expose the supporting evidence. Failed or cancelled terminals
remain failure exits even when selected by the wait filter. Ctrl-C or an expired CLI deadline
ends the local request/observation and does not itself cancel the workflow.

For run starts, choose an explicit new `--request-file` and recover with `run reconnect FILE`.
The client saves the exact start before sending it and checks the original host/caller/grant before
replay. `--input NAME=FILE` uploads frozen text; `--prepare-only` stops after saving. Keep the private
record while the result is uncertain, then remove it deliberately; there is no automatic archive.
For other lost command replies, retain the same ID and complete request: changed reason, guards,
evidence, or document bytes can conflict. Commands that construct a proposal from fresh reads,
such as `sequence remediate`, need the [recovery guidance](../../docs/guides/headless-dogfood.md#failure-and-remediation)
before being invoked again. Pages are explicit; a timeline page cursor and a run-stream cursor
belong to different feeds. A following timeline can therefore repeat facts from its initial page.

Artifact downloads and blueprint/sequence exports require a new destination file. The artifact
command assembles bounded ranges and verifies size and digest before committing its output.
Normal failure removes an uncommitted output; removal failure returns exit 9 and reports that the
destination may remain, while retaining the original error internally. Cancellation and unwinding
attempt removal and report any failure on stderr. Forced termination cannot guarantee cleanup.
Existing files are never replaced.
Output and diagnostic write failures return exit 9, including a closed stdout pipe. If the error
channel itself fails, the exit remains nonzero even though no failure envelope can be delivered.

The [model workflow recipe](../../examples/operator/README.md#author-a-model-workflow) uses a local
draft of pending mutations. `workflow` commands send editing gestures to the daemon; they do not
calculate graph identities or call models. Draft edits lock the file and atomically replace it
only after the reply and an unchanged-byte check. `--expected-edit TOKEN` additionally guards the
version inspected by a script or another session. `save` stores a new immutable revision;
`open` reopens an exact revision into a new file. Unsupported rich definitions refuse editing.

## Contribute

`main` owns arguments and the overall deadline. `session` loads one credential, negotiates, and
constructs common command envelopes; `command` routes each family. `input`, `output`, `error`,
and `command/stream` own bounded documents, file cleanup, machine records, exits, and observation
presentation. Workflow decisions stay in the daemon and its domain owners.

The documentation parser tests maintained commands with the production Clap definitions.
`tests/automation.rs` runs actual CLI children against a scripted endpoint; the
[headless evidence lane](../../docs/development/verification-evidence.md#actual-binary-scenarios)
also uses the actual daemon. These establish different parts of the operator experience.
