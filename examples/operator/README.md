# Headless operator examples

These maintained files use the production daemon reader and ordinary immutable blueprint documents.
Copy them into a private directory outside the repository. The initial configuration grants
controller operations only for workflow `operator-starter`, finite budgets, no filesystem/network
access, and no artifact access. No adapter is enabled. Its terminal-only starter proves setup
without external work or broad authority.

## Startup and restart

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

This deliberately permits generated artifact identities within the independently checked workflow
scope. Review that scope and set `dangerous_allow_broad_authority = true` visibly in
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

Update the existing capability authority selectors to identities `["operator-model"]`,
operations `["model.generate"]`, provider profiles `["local-model-loopback"]`,
trust zones `["operator-configured-local-model"]`, and
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

[The CLI contract](../../docs/reference/control-api.md#cli-automation-contract) owns JSON schema 2, exits,
deadlines, command parity and bounded follow. A timeout or client cancellation does not prove a
submitted mutation was cancelled. Retry a lost mutation response only with the same explicit
command ID and identical request.
