# Headless operator examples

These maintained files use the production daemon reader and ordinary immutable blueprint documents.
Copy them into a private directory outside the repository. The initial configuration grants
controller operations only for workflow `operator-starter`, finite budgets, no filesystem/network
access, and no artifact access. No external adapter is enabled. Its terminal-only starter proves setup
without external work or broad authority.

## Independent execution

Copy [execution-only.toml](execution-only.toml) to a private directory as `daemon.toml` for a
host that executes capabilities without workflow services. Retain its `host_id` across restarts.
The example starts with no adapters and grants the named process/model operations and artifact
access with finite limits. It includes restricted artifacts because uploads default to that
classification and process/model outputs use it. The grant has no workflow restriction because
direct invocations have no workflow lineage. Configure the reviewed process profile as described
below, or the [local model profile](../../docs/guides/local-model-endpoint.md), and add the corresponding exact
filesystem/network/secret scopes to the actor grant. Validate with `--check-config` before starting.

This template explicitly opts into `dangerous_allow_broad_authority` because the model advertises
unknown effects and newly uploaded artifact identities cannot be enumerated in advance. Its named
capability/operation lists and sensitivity/resource bounds still apply. Review that grant for the
installation before adding adapters.

A direct model call requires a profile with known enforceable token and billing bounds. The local
model profile deliberately starts with unknown bounds; configure byte-BPE input/output counting
and either a declared tariff or explicitly unbilled operation for the actual endpoint. Discovery
is not a promise that an unknown allowance can enter. The host never loads model weights itself.

Use the same explicit endpoint and credential arguments for each command. For example in
PowerShell, after starting the daemon:

```powershell
milkdrift --endpoint http://127.0.0.1:9734/ --token-env MILKDRIFT_TOKEN --json invocation catalog
$uploaded = milkdrift --endpoint http://127.0.0.1:9734/ --token-env MILKDRIFT_TOKEN --json artifact upload ./input.txt --host host:execution --upload-id input-1 --media-type text/plain | ConvertFrom-Json
$m = $uploaded.value
@(@{name='source'; value=@{type='artifact'; reference=@{identity=$m.artifact_id; digest=$m.digest; media_type=$m.content_type; size_bytes=$m.size}}}) |
  ConvertTo-Json -Depth 8 -AsArray | Set-Content -Encoding utf8 ./inputs.json
milkdrift --endpoint http://127.0.0.1:9734/ --token-env MILKDRIFT_TOKEN --json invocation prepare operator-process process.execute --host host:execution --request-id process-1 --inputs ./inputs.json --output ./request.json
$accepted = milkdrift --endpoint http://127.0.0.1:9734/ --token-env MILKDRIFT_TOKEN --json invocation submit ./request.json | ConvertFrom-Json
milkdrift --endpoint http://127.0.0.1:9734/ --token-env MILKDRIFT_TOKEN --json --timeout-secs 60 invocation wait $accepted.value.execution
milkdrift --endpoint http://127.0.0.1:9734/ --token-env MILKDRIFT_TOKEN --json invocation show $accepted.value.execution
milkdrift --endpoint http://127.0.0.1:9734/ --token-env MILKDRIFT_TOKEN --json invocation observations $accepted.value.execution --after 0 --limit 128
```

The process profile must declare `source` as an input and consume it through an input argument or
stdin. [direct-process-inputs.json](direct-process-inputs.json) demonstrates a small explicit inline
value; materialization encodes that value as JSON. Use upload when the process needs exact file bytes.

For a fresh model call, upload [direct-model-task.json](direct-model-task.json) with media type
`application/json`, name its reference `milkdrift.model_task` in the inputs array, and prepare
`operator-model model.generate`. Download an output artifact from its observation with
`milkdrift --endpoint http://127.0.0.1:9734/ --token-env MILKDRIFT_TOKEN --json artifact get ARTIFACT --output ./result.txt`. The CLI verifies size and digest
and requires a new destination file. Direct requests cannot reuse workflow continuation.

`prepare` writes a new request file without admitting execution. It binds the selected host,
catalog, operation, inputs, advertised ceilings and absolute deadline. Review it and submit promptly;
stale discovery is refused. Preserve the file unchanged for replay after a lost reply or restart.
Use `invocation lookup process-1` to recover acceptance; a new request identity could create new work.
Cancellation uses `invocation cancel EXECUTION --request-id stop-1`; acknowledgement does not prove
that the operation stopped. `wait` streams bounded pages and exits unsuccessfully for refusal,
failure, retained uncertainty or timeout. Timeout does not release accepted work.

Uploads accept at most 512 KiB each and publish only verified complete content. Repeating the same
upload ID/content returns the same artifact; changing content conflicts. Cumulative per-actor input
quotas survive restart. Use HTTPS for non-loopback endpoints with a distinct credential configured
for the selected owner. Workflow use of the same host follows the [peer guide](../../docs/operations/peers.md).

## Startup and restart

[peer-placement.json](peer-placement.json) is a two-task blueprint: repository A work requires
`peer-a`, then repository B work requires `peer-b`, both using `process.execute` and returning
`host_result`. It requires configured peer relationships, host-owned process profiles, output
download authority and an origin grant covering those tasks. The default starter configuration
does not supply them. Follow the [peer guide](../../docs/operations/peers.md#pin-tasks-to-approved-hosts)
before importing it. The loopback daemon scenario executes this exact maintained document.

Run the [fresh-directory quick start](../../README.md#fresh-directory-quick-start). Configuration
paths resolve against its directory; keep the generated environment credential for later CLI calls.

For Unix, build the same binaries and add this checkout's absolute `target/debug` directory to
`PATH`. Copy `daemon.toml` and `starter.json` into a private directory,
generate `MILKDRIFT_TOKEN` with `export MILKDRIFT_TOKEN="$(openssl rand -hex 32)"`, and launch the
daemon in the foreground with `milkdrift-daemon --config daemon.toml`. Use the README's CLI command
arguments in another authenticated shell; extract the imported revision with
`jq -r '.value.value.revision_id'` instead of PowerShell's `ConvertFrom-Json`.

Stop a foreground daemon with Ctrl-C and wait for its exit before restarting the same configuration.
On Windows the README's hidden child has no interactive Ctrl-C console. After the terminal-only
starter finishes, `Stop-Process -Id $daemonProcess.Id` followed by `$daemonProcess.WaitForExit()`
is an abrupt stop. Then run `& $daemon --config daemon.toml` in the foreground, supplying the same
environment credential, and use another shell for the client. Subsequent Ctrl-C shutdown follows
[the daemon's bounded shutdown policy](../../docs/operations/daemon.md#shutdown).
An abrupt stop with entered external work may leave uncertainty; it is not clean shutdown.

After restart, inspect `run show run-starter` and `run timeline run-starter --limit 100` and repeat
`blueprint import starter.json` with command ID `starter-import`. Exact replay returns the original
result. [The control API](../../docs/reference/control-api.md#commands) owns replay/conflict rules.

## One byte-pinned local process

Copy `process.json` and `process-profile.example.json`. Review and edit the profile:

- Use the exact absolute path to a trusted executable. The Unix example uses `printf` and one
  literal argument. On Windows, use `C:/Windows/System32/whoami.exe` with arguments
  `["/user", "/fo", "csv", "/nh"]`. The reader requires a nonempty argument vector. The executable
  runs with the daemon account's privileges.
- Match `platform` to the daemon build. On Windows set all three fields to `false`:
  `owned_process_group`, `terminal_group_observation`, and `descendant_escape_prevention`.
  The shipped Unix values are refused by a Windows build.
- Compute its BLAKE3 with `b3sum EXECUTABLE`, put `b3_` followed by the 64 hex digits in
  `implementation.content_digest`, and put its exact file length in `size_bytes`.
  A changed executable requires a newly reviewed profile revision. BLAKE3 CLI tooling is external
  build tooling; Milkdrift verifies the pin at registration and again before entry.
- Set `filesystem_roots` to the executable parent with `execute` access and the configuration
  directory with `read_write` access. The explicit `isolated_root` working directory is managed
  by the daemon. Inputs and stdin are explicitly empty/disabled. The declared output is the
  bounded `stdout` artifact; stderr is bounded and discarded.
- Keep the 10-second wall timeout, bounded output/capture limits, best-effort cancellation, and
  `retain_uncertain` restart policy.

Keep profile ceilings within the [process authority requirements](../../docs/guides/local-process.md#profile-schema-2).
The maintained template fits the starter's finite artifact grant.
Retain its `Any` locality and peer selectors: [revision admission](../../docs/operations/authority.md)
cannot narrow dimensions absent from task requirements. The workflow still names one exact
capability and trust zone, and this configuration registers only the reviewed local adapter.

Save as `process-profile.json`. In `daemon.toml` set
`adapters.process_profiles = ["process-profile.json"]`. Add exact corresponding authority
filesystem roots with `access = ["read", "write"]` or `["execute"]`. Authority roots use
canonical host paths in `C:/...` form on Windows and `/...` on Unix, with no traversal.
Redirected directories or aliases must resolve to the same exact path as the adapter;
[the authority path rules](../../docs/operations/authority.md) define that comparison.
Permit the initial workspace scope:

```toml
[actors.authority.resources.workspace]
scopes = ["root"]
allow_any_in_run = false
```

For output/context inspection, replace the artifact deny-all table with:

```toml
[actors.authority.resources.artifacts]
type = "allow"
sensitivities = ["public", "internal", "restricted"]

[actors.authority.resources.artifacts.identities]
type = "any"
```

This deliberately permits every artifact identity at the listed sensitivities. Artifact access is
independent of workflow/run scope; use an `only` selector for known artifact IDs when narrower
access is required. Review this scope and set `dangerous_allow_broad_authority = true` visibly in
`[actors.authority]`; this acknowledgement does not grant any additional resource by itself.
Validate with `--check-config`, restart, inspect `capability show operator-process`, import
`process.json`, and start `run-process operator-starter REVISION`. Use fresh explicit command
IDs. Follow the [grant revision rule](../../docs/operations/authority.md#changing-authority) whenever
editing authority. Wait with `--timeout-secs 20 run wait run-process`.

## One separately managed loopback model

Copy [the endpoint profile](../local-model/openai-compatible-loopback.example.json) to
`model-profile.json`, keep identity `local-model-loopback`, and replace the model alias with
the exact alias served by your already-running endpoint. The example uses loopback
`http://127.0.0.1:8080`, no authentication, disabled ambient proxies, no redirects, and explicit
request/response/SSE/time bounds. Advertise streaming and system-role support only when the server
supports them. Milkdrift does not install, download, start or stop the model server.

Copy `model.json`. Register its exact capability:

```toml
[[adapters.model_profiles]]
capability_id = "operator-model"
profile = "model-profile.json"
```

Update the existing capability authority selectors to identities
`["operator-model", "milkdrift-workflow-control"]`, operations
`["model.generate", "workflow.accept_result"]`, provider profiles `type = "any"`,
trust zones `["operator-configured-local-model", "milkdrift-control"]`, and
maximum side effect `"unknown"`. The example's finite 4,000,000-unit grant covers the contract
ceiling when the profile has unknown token bounds. With byte-BPE counting, generation resolution
checks the profile's configured output maximum, even when this task requests only 64 output units.
Retain the explicit dangerous acknowledgement: external model effects and cancellation cannot be
proven absent. Use the workspace and artifact scopes above.
Add exactly this network scope (adjust both fields when the endpoint/profile changes):

The provider selector also admits the built-in control capability, which has no provider profile.
The model task itself remains pinned to `local-model-loopback`, and acceptance can only use the
named control operation. Both tasks use the same frozen run authority.

```toml
[actors.authority.resources.network]
profiles = ["local-model-loopback"]
destinations = ["127.0.0.1:8080"]
```

Validate and restart the daemon, then run:

```sh
milkdrift --json provider show local-model-loopback
milkdrift --json --command-id model-validate blueprint validate model.json
milkdrift --json --command-id model-import blueprint import model.json
milkdrift --json --command-id model-start run start run-model operator-starter REVISION --request-file model-run.request.json
milkdrift --json --timeout-secs 180 run wait run-model --terminal succeeded
milkdrift --json run show run-model
milkdrift --json attempt inspect run-model ATTEMPT_ID
milkdrift --json run timeline run-model --limit 100
milkdrift --json artifact metadata ARTIFACT_ID
milkdrift --json artifact get ARTIFACT_ID --output response.json
```

Take the revision, attempt and artifact IDs from the preceding JSON. Attempt inspection retains
the model's immutable invocation outcome. This example separately requires a complete, non-whitespace
final response before reaching success; inspect its acceptance task for `accepted` and `reason`.
An exhausted response remains a completed model invocation but takes the workflow's rejection
terminal. [Result acceptance](../../docs/guides/result-acceptance.md) covers other output purposes.
Attempt inspection also retains
exact capability/profile generation, context-manifest provenance, usage when supplied, bounded
progress counts and output metadata. Inspect output files explicitly; ordinary CLI JSON does not
print generated prose. A post-entry timeout, truncated response, lost server or cancellation may
retain uncertainty. Inspect the exact attempt and use `attempt resolve --action retain` or another
explicitly authorized resolution; do not assume retry is safe. [Model evidence](../../docs/guides/local-model-endpoint.md)
documents optional structural qualification and hermetic failure tests.

## Author a model workflow

Use a running workflow-enabled daemon and a model profile configured as above. The caller needs
blueprint validation/import and inspection, catalogue/profile reads, and permission to select the
model and the control acceptance capability. Use a credential scoped to `release-notes`, then
one scoped to `meeting-summary` for the second workflow, or an existing explicitly broader grant.
The starter configuration's `operator-starter` scope does not cover these identities. Configure
the scopes through the existing [authority configuration](../../docs/operations/authority.md).
Running the saved definition also needs `model.generate`, `workflow.accept_result`, artifact
and workspace access. These commands use the endpoint and credential environment configured
earlier. Choose the exact capability shown by `workflow models`; the example assumes
`operator-model`. Profile credentials remain with the daemon.

From the repository root, create a draft/review workflow using the maintained prompt files:

```sh
milkdrift workflow models
milkdrift workflow new release-notes --name "Release notes" --file release-notes.draft.json
milkdrift workflow input release-notes.draft.json brief
milkdrift workflow add release-notes.draft.json draft --model operator-model \
  --prompt examples/operator/release-notes/draft.txt --maximum-output-units 512
milkdrift workflow add release-notes.draft.json review --model operator-model \
  --prompt examples/operator/release-notes/review.txt --maximum-output-units 512
milkdrift workflow connect release-notes.draft.json draft brief --run-input brief
milkdrift workflow connect release-notes.draft.json review brief --run-input brief
milkdrift workflow connect release-notes.draft.json review draft --from-step draft
milkdrift workflow output release-notes.draft.json review --name notes
milkdrift workflow inspect release-notes.draft.json
milkdrift --command-id release-notes-save-1 workflow save release-notes.draft.json
```

The `brief` declaration names a value supplied separately on each run. It is not embedded in the
definition. The review's bindings select that brief and only the draft step's final text; there is no
implicit conversation history. Every step requires a canonical model response with natural `stop`
and non-whitespace final text. Empty responses and `length` finishes enter a visible review hold,
whose unchanged continuation fails. This is a completeness check, not a judgment that the notes
are accurate or useful. The [result acceptance guide](../../docs/guides/result-acceptance.md)
explains the evidence and control branch.

These authoring commands make no model request or run. Use the saved revision in both starts below.
`--input` reads a bounded UTF-8 file, uploads it as a restricted `text/plain` artifact, and retains
the exact start in the new `--request-file` before submission. It also accepts `NAME=-` for stdin.

```sh
milkdrift --command-id release-harbor-1 run start harbor-notes release-notes REVISION --input brief=examples/operator/release-notes/harbor-brief.txt --request-file harbor.request.json
milkdrift --command-id release-lantern-1 run start lantern-notes release-notes REVISION --input brief=examples/operator/release-notes/lantern-brief.txt --request-file lantern.request.json
milkdrift --timeout-secs 90 run reconnect harbor.request.json --wait
```

Reconnect resends the saved request to recover its receipt. It never rereads the brief or creates a
new run identity. Editing the local brief after preparation cannot change accepted work. Add
`--prepare-only` to a start to save it without running; reconnect can submit that exact record later.
Use a new run, command, and request-file destination for deliberately new execution. Existing
request files are never overwritten. For already uploaded inputs or other supported media, supply
`--inputs FILE` containing an array such as `[{"name":"brief","artifact_id":"input:…"}]`.

Starts and reconnects return acceptance by default. Add `--wait` with an explicit `--timeout-secs`
to observe completion. Before waiting, the CLI prints the recovery file, command and run identity;
JSON mode emits a `run.prepared` record with `final: false`, followed by one final outcome.
Closing the client or reaching its deadline ends observation while daemon work continues.
Cancellation is a separate `run cancel` request, and its acceptance does not prove external work
has stopped. Reconnect leaves usage accounting, uncertain effects and accepted child runs intact.

Recovery files identify the host, actor, exact grant, command, workflow revision, input artifacts,
reason, evidence and guards. They contain no credential or source-file bytes. Keep them private:
Unix creation uses mode 0600 and reconnect refuses group/other access and symlinks. Keep a record
while work is active or its outcome is uncertain; remove it deliberately when recovery is no longer
needed. The CLI keeps no automatic request archive. Uploaded content follows the daemon's existing
bounded artifact retention. A different host or caller/grant is refused before resubmission.
The output allowance of 512 is an example choice, not a promise that a model will finish within it.

Read accepted notes directly with `milkdrift run result harbor-notes --details`. Once the workflow
succeeds, `milkdrift run result harbor-notes --field notes --output harbor-notes.txt` retrieves and
verifies the declared final artifact. A returned model response, its required completeness check,
and the workflow outcome remain separate in the display; absent usage is unknown. If the final
review fails its check, follow the [held model repair](../../docs/guides/headless-dogfood.md#repair-a-held-model-result)
procedure before signalling its review hold. Editing the saved draft below affects future starts;
repairing a paused run requires a separately approved prospective proposal.

The save reply returns the daemon's exact `revision_id`. Use it as `REVISION` below. Saving with
no changes returns the same version; changing a prompt and saving produces a child of that exact
base. Old versions and any work already using them remain unchanged.

```sh
milkdrift workflow open REVISION --file reopened.draft.json
milkdrift workflow prompt reopened.draft.json review --prompt examples/operator/release-notes/review.txt
milkdrift --command-id release-notes-save-2 workflow save reopened.draft.json
milkdrift workflow inspect reopened.draft.json
```

`--prompt -` reads UTF-8 text from standard input. `workflow model FILE STEP CAPABILITY` explicitly
changes a model; no unavailable choice is silently substituted. `workflow disconnect FILE STEP
INPUT` removes a connection. `workflow move FILE STEP --before OTHER` changes order, and omitting
`--before` moves the step to the end. A reorder that puts a source after its consumer refuses.
`workflow remove FILE STEP` refuses while another step or the selected output still uses it.

Create a second independent workflow beside the first:

Find and reuse saved definitions through bounded version pages. `show` returns the declared
inputs and outputs without execution values. A run always names one exact revision; there is no
implicit mutable preferred-version pointer. Following an empty filtered page's `next_cursor` may
be necessary because each page scans only a bounded number of stored definitions.

```sh
milkdrift workflow list --workflow release-notes --limit 32
milkdrift workflow show REVISION
milkdrift --command-id copy-notes-1 workflow copy REVISION independent-notes \
  --name "Independent release notes" --file independent.draft.json
```

Supply a different workflow identity for an independent copy. The saved copy records the exact
source revision in its immutable provenance reason and starts its own version lineage. It copies
definition nodes, edges, interfaces and metadata, with only the requested name and identity changed.
It never copies old run inputs, private outputs, reservations or writable run scopes. Later edits
leave both the source and already accepted runs unchanged. Copying requires source inspection and
destination import permission; executing it still requires the ordinary run/capability grants.
Governing agreements include the original workflow identity, so copying them refuses instead of
dropping their checks. Reuse their exact saved definition or explicitly author another governed
method. Rich definitions retain their graph on copy but still require explicit blueprint mutations
when the model editor cannot preserve them. Saving or copying does not publish a callable service.

For a separate workflow authored from scratch:

```sh
milkdrift workflow new meeting-summary --name "Meeting summary" --file meeting.draft.json
milkdrift workflow input meeting.draft.json brief
milkdrift workflow add meeting.draft.json summarize --model operator-model \
  --prompt examples/operator/release-notes/meeting.txt --maximum-output-units 256
milkdrift workflow connect meeting.draft.json summarize brief --run-input brief
milkdrift workflow output meeting.draft.json summarize --name summary
milkdrift --command-id meeting-save-1 workflow save meeting.draft.json
```

Draft files contain the workflow identity, exact optional base, and pending ordinary blueprint
mutations. They may lack steps or a final output while editing; `save` requires both and nonempty
prompts. They contain neither credentials nor per-run values. New/open destinations must not exist.
Edits hold a companion `.lock` file's OS lock and check the original bytes before atomically
publishing a complete replacement. The empty lock file may remain; the OS releases the lock on
process exit. An interrupted write leaves the previous draft intact, although forced termination
can leave an unpromoted temporary file in its directory. This does not establish power-loss
durability. Do not edit a draft outside the CLI while a command is in progress.

`inspect --json` returns `edit_token`. Supply it with `workflow --expected-edit TOKEN ...` to refuse
changes since that inspection. Independently copied drafts may intentionally branch from the same
immutable base; there is no mutable latest-version pointer. Rich imported definitions that this
editor cannot reproduce exactly refuse before a draft is written. Advanced clients may submit
existing mutations through the public `construct_blueprint` operation. Existing offline
`blueprint create` and `govern` serve the separate governed-method bootstrap.

## Author, inspect and change future work

`blueprint validate` calls the daemon's strict reader without storing a revision.
`blueprint export REVISION --output FILE` writes exact canonical bytes to a new file.
`sequence compile FILE --author human:operator --output blueprint.json` needs no daemon or
credential and emits an ordinary blueprint; validation/import still use the canonical daemon path.
[Prompt sequences](../../docs/guides/headless-dogfood.md) remain the linear process-stage authoring
format. They do not add model-backed sequence stages.

Pause, build/submit a prospective remediation proposal with `sequence remediate` or
`proposal submit`, inspect `blueprint diff BASE PROPOSED`, then approve/apply with exact
proposal digest, revision, sequence and decision IDs. These operations remain on ordinary authority
and reconciliation paths. Continuous controller activation remains gated.

[The CLI contract](../../docs/reference/control-api.md#cli-automation-contract) owns JSON schema 2, exits,
deadlines, command parity and bounded follow. A timeout or client cancellation does not prove a
submitted mutation was cancelled. Retry a lost mutation response only with the same explicit
command ID and identical request.
