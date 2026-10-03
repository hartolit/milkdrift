# 02 — Run with inputs and reconnect safely

After 01 is ready, follow [the shared rules](README.md),
[implementation practice](../../practices/implementation.md), and
[documentation practice](../../practices/documentation.md). Finish input submission and request
recovery through the daemon, public client, and CLI together.

Start with the public `StartRun` command, `apps/daemon/src/host/commands/runs.rs`, runtime `CreateRun`,
`crates/control-client`, and `apps/cli/src/command/run.rs`. Reuse existing upload, workspace input,
published-run input, and saved-request handling where they already own the required behavior.

## Build this

Let the user run one saved workflow with a supplied change brief, then run the same version with
a different brief. Validate required names, types, duplicate/unexpected inputs, size limits, and
read permissions before work starts. Files go through the existing upload route or supported text
inputs, not arbitrary paths on the server. Keep each run's data and outputs separate.

Freeze the accepted inputs. Editing a local file later cannot change accepted work. Reuse the
existing input validation across ordinary and published runs where its meaning is the same;
keep their different permissions intact. Update affected readers, versions, fixtures, and callers
together if a public request changes. Preserve the supported replay behavior of older saved requests.

Before submission can cause work, retain enough exact request information to recover after the
client exits or loses the first reply. Use the existing mechanism or the smallest missing addition.
The saved record identifies the host, caller, request, workflow version, inputs, and required guards.
Do not save credentials. Handle sensitive input records with private access and clear retention;
do not create another execution-history database or an unbounded client archive.

Recover by looking up or resending the same request, not by generating new IDs or reading a changed
file. The same key with different contents must conflict. Do not silently replay against another
host or caller. A new execution is an explicit choice. Keep the existing point at which model
selection is frozen; do not make reconnect choose a different model or invent earlier pinning.

Make waiting optional and bounded. Show the recoverable identity before waiting. Closing the client
or reaching its timeout stops observation, not the daemon's work. Cancellation remains a separate
request; accepting cancellation does not prove an external process has stopped. Reconnect must not
reset usage limits, release uncertain resources, or create duplicate child runs.

## Check just this work

Compile changed packages and affected callers. Test input validation, private or corrupt inputs,
two sets of inputs for one version, exact replay, and changed-request conflicts. Exercise the
create/start interruption points: before create, after create but before start, and after start
before the reply. Count actual fixture requests when asserting that work was not repeated.

Add one small real daemon/CLI test for lost reply or client exit and recovery. A controlled model
endpoint is sufficient. Keep existing direct and published request regressions relevant to this
change. Do not run the full system scenario or live-model qualification here.

## Commit points

Commit input support with all required API/runtime/client consumers and tests. Commit exact request
saving and replay as another working change. Commit client wait/reconnect behavior and its regression
when ready. Include guide changes where the user-visible behavior is introduced.

Stop when supplied-input execution and recovery work without manual ID reconstruction.
Record commits, commands, checks, and recovery behavior in `handoffs/02.md`. No full workspace gate.
