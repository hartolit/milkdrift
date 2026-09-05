# Headless operator examples

These maintained files use the production daemon reader and ordinary immutable blueprint documents.
Copy them into a private directory outside the repository. The initial configuration grants
controller operations only for workflow `operator-starter`, finite budgets, no filesystem/network
access, and no artifact access. No adapter is enabled. Its terminal-only starter proves setup
without external work or broad authority.

## Fresh directory (PowerShell)

Build the product binaries from the checkout, then use their absolute paths:

```powershell
cargo build -p milkdrift-daemon --bin milkdrift-daemon -p milkdrift-cli --bin milkdrift
$daemon = (Resolve-Path target/debug/milkdrift-daemon.exe).Path
$cli = (Resolve-Path target/debug/milkdrift.exe).Path
$examples = (Resolve-Path examples/operator).Path
New-Item -ItemType Directory -Path C:/Milkdrift
Copy-Item "$examples/daemon.toml", "$examples/starter.json" C:/Milkdrift/
Set-Location C:/Milkdrift
$env:MILKDRIFT_TOKEN = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32))
& $daemon --config daemon.toml --check-config
$daemonProcess = Start-Process $daemon -ArgumentList '--config', 'C:/Milkdrift/daemon.toml' -PassThru -WindowStyle Hidden
& $cli --timeout-secs 10 daemon readiness
$import = & $cli --json --command-id starter-import blueprint import starter.json | ConvertFrom-Json
$revision = $import.value.value.revision_id
& $cli --json --command-id starter-start run start run-starter operator-starter $revision
& $cli --json --timeout-secs 10 run wait run-starter --terminal succeeded
& $cli --json run timeline run-starter --limit 100
```

The token is generated into the environment, inherited by the daemon, and never placed in argv
or a source file. Keep that session for later CLI calls. File references are also supported; use
an OS-private regular file and the daemon's `secret_sources` and CLI `--token-file` settings.
Relative storage, profile and secret paths resolve against the configuration directory. Artifact
content belongs under `data/artifacts`; the CLI never opens that directory.

On Unix, copy the same files, generate the token with
`export MILKDRIFT_TOKEN="$(openssl rand -hex 32)"`, and launch
`milkdrift-daemon --config daemon.toml &`. Use the same CLI commands; extract the revision from
JSON with `jq -r '.value.value.revision_id'`. Stop a foreground daemon with Ctrl-C before restart.
An abrupt kill deliberately exercises uncertainty and is not clean shutdown.

## One byte-pinned local process

Copy `process.json` and `process-profile.example.json`. Review and edit the profile:

- Use the exact absolute path to a trusted executable. The Unix example uses `printf` and one
  literal argument. On Windows, `C:/Windows/System32/hostname.exe` with an empty argument array
  is a small alternative. The executable runs with the daemon account's privileges.
- Compute its BLAKE3 with `b3sum EXECUTABLE`, put `b3_` followed by the 64 hex digits in
  `implementation.content_digest`, and put its exact file length in `size_bytes`.
  A changed executable requires a newly reviewed profile revision. BLAKE3 CLI tooling is external
  build tooling; Milkdrift verifies the pin at registration and again before entry.
- Set `filesystem_roots` to the executable parent with `execute` access and the configuration
  directory with `read_write` access. The explicit `isolated_root` working directory is managed
  by the daemon. Inputs and stdin are explicitly empty/disabled. The declared output is the
  bounded `stdout` artifact; stderr is bounded and discarded.
- Keep the 10-second wall timeout, bounded output/capture limits, best-effort cancellation, and
  `retain_uncertain` restart policy. On Windows set all three `platform` fields to false:
  Unix process-group ownership and terminal group observation are not available.

Save as `process-profile.json`. In `daemon.toml` set
`adapters.process_profiles = ["process-profile.json"]`. Add exact corresponding authority
filesystem roots with `access = ["read", "write"]` or `["execute"]`. Authority roots use
`C:/...` on Windows and `/...` on Unix, with no traversal. Permit the initial workspace scope:

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

This deliberately permits generated artifact identities within the independently checked workflow
scope. Review that scope and set `dangerous_allow_broad_authority = true` visibly in
`[actors.authority]`; this acknowledgement does not grant any additional resource by itself.
Validate with `--check-config`, restart, inspect `capability show operator-process`, import
`process.json`, and start `run-process operator-starter REVISION`. Use fresh explicit command
IDs. Wait with `--timeout-secs 20 run wait run-process`.

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

Update the existing capability authority selectors to identities `["operator-model"]`,
operations `["model.generate"]`, provider profiles `["local-model-loopback"]`,
trust zones `["operator-configured-local-model"]`, locality `["local"]`, and
maximum side effect `"unknown"`. Keep the finite 4,000,000-unit grant ceiling: current generation-level resolution checks the model contract maximum even though this task requests only 64 output units. Retain the explicit dangerous acknowledgement: external model
effects and cancellation cannot be proven absent. Use the workspace and artifact scopes above.
Add exactly this network scope (adjust both fields when the endpoint/profile changes):

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
milkdrift --json --command-id model-start run start run-model operator-starter REVISION
milkdrift --json --timeout-secs 180 run wait run-model --terminal succeeded
milkdrift --json run show run-model
milkdrift --json attempt inspect run-model ATTEMPT_ID
milkdrift --json run timeline run-model --limit 100
milkdrift --json artifact metadata ARTIFACT_ID
milkdrift --json artifact get ARTIFACT_ID --output response.json
```

Take the revision, attempt and artifact IDs from the preceding JSON. Attempt inspection retains
exact capability/profile generation, context-manifest provenance, usage when supplied, bounded
progress counts and output metadata. Inspect output files explicitly; ordinary CLI JSON does not
print generated prose. A post-entry timeout, truncated response, lost server or cancellation may
retain uncertainty. Inspect the exact attempt and use `attempt resolve --action retain` or another
explicitly authorized resolution; do not assume retry is safe. [Model evidence](../../docs/guides/local-model-endpoint.md)
documents optional structural qualification and hermetic failure tests.

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

[The CLI contract](../../docs/reference/control-api.md#cli-behavior) owns JSON schema 2, exits,
deadlines, command parity and bounded follow. A timeout or client cancellation does not prove a
submitted mutation was cancelled. Retry a lost mutation response only with the same explicit
command ID and identical request.
