# Operator CLI

`milkdrift` submits commands to a running daemon and presents its authorized results. Start with
the [operator recipe](../../examples/operator/README.md): it builds the applications, configures
authentication, runs a small workflow, and shows process/model setup. The CLI uses the
[control client](../../crates/control-client/README.md); it never opens the daemon's database.

## Choose what to inspect or change

| Task | Command family |
| --- | --- |
| Check connectivity and your grant | `daemon readiness`, `health`, `authority` |
| Create and compare immutable definitions | `blueprint validate`, `import`, `show`, `export`, `list`, `diff` |
| Turn implementation prompts into a workflow | `sequence validate`, `compile`, `import`, `show`, `status`, `stage`, `remediate` |
| Start or control work | `run start`, `pause`, `resume`, `cancel`, `signal` |
| Observe progress and outcomes | `run list`, `show`, `wait`, `timeline`; `node`; `attempt inspect` |
| Address an unknown external outcome | `attempt resolve`, with an explicit decision and evidence |
| Review a prospective change | `proposal submit`, `list`, `show`, `approve`, `reject`, `apply` |
| Inspect execution options | `capability` and `provider` |
| Manage a configured peer | `peer list`, `show`, `connect`, `reload`, `disconnect`, `drain`, `revoke` |
| Retrieve output or presentation state | `artifact metadata`, `get`; `layout get`, `put` |
| Inspect or continue a controller checkpoint | `controller status`, `continue`; production activation remains gated |

Use `milkdrift --help` and family help for exact arguments. The
[control API reference](../../docs/reference/control-api.md#cli-automation-contract) owns JSON
output, exits, bounds, and the complete command contract. The
[sequence guide](../../docs/guides/headless-dogfood.md) follows verification and approved repair.

## Use it from scripts

Choose `--json`, explicit command IDs for mutations, and an overall `--timeout-secs`. `run wait`
and noninteractive `--follow` require a deadline. `--yes` supplies local confirmation for commands
that require it; the daemon still checks authority, evidence, and optimistic guards.

A successful start reply means acceptance. `run wait` checks for a terminal result, while attempt
inspection and artifact download expose the supporting evidence. Failed or cancelled terminals
remain failure exits even when selected by the wait filter. Ctrl-C or an expired CLI deadline
ends the local request/observation and does not itself cancel the workflow.

After a lost command reply, retain the same ID and complete request: changed reason, guards,
evidence, or document bytes can conflict. Commands that construct a proposal from fresh reads,
such as `sequence remediate`, need the [recovery guidance](../../docs/guides/headless-dogfood.md#failure-and-remediation)
before being invoked again. Pages are explicit; a timeline page cursor and a run-stream cursor
belong to different feeds. A following timeline can therefore repeat facts from its initial page.

Artifact downloads and blueprint/sequence exports require a new destination file. The artifact
command assembles bounded ranges and verifies size and digest before committing its output.
An interrupted uncommitted output is removed on normal cleanup; existing files are never replaced.

## Contribute

`main` owns arguments and the overall deadline. `session` loads one credential, negotiates, and
constructs common command envelopes; `command` routes each family. `input`, `output`, `error`,
and `command/stream` own bounded documents, file cleanup, machine records, exits, and observation
presentation. Workflow decisions stay in the daemon and its domain owners.

The documentation parser tests maintained commands with the production Clap definitions.
`tests/automation.rs` runs actual CLI children against a scripted endpoint; the
[headless evidence lane](../../docs/development/verification-evidence.md#actual-binary-scenarios)
also uses the actual daemon. These establish different parts of the operator experience.
