# Milkdrift

Milkdrift is a local-first, durable workflow runtime for work performed by AI endpoints, coding
agents, tools, humans, and peer machines. It lets an operator inspect execution, pause it, and
revise future work while retaining the history and evidence needed to understand each decision.
Models and tools run outside the core.

The product is a pre-1.0 headless daemon and CLI. There is no UI or storage migration. Local
processes run with the daemon account's privileges; they are not sandboxed. Controller final-entry
resource accounting exists in the libraries, but continuous controller activation remains refused
pending qualification. Real external interoperability and hosted portability evidence remain
incomplete; [status](docs/product/status.md) owns the exact versions, limitations, and evidence.

## Fresh-directory quick start

From this checkout, use the pinned Rust toolchain to build both applications. This PowerShell
scenario copies [production examples](examples/operator/README.md) into a new operator directory,
generates a credential in the current process environment, and runs a terminal-only workflow.
The starter has finite authority for one workflow and enables no external adapters.

```powershell
cargo build -p milkdrift-daemon --bin milkdrift-daemon -p milkdrift-cli --bin milkdrift
if ($LASTEXITCODE -ne 0) { throw 'Application build failed' }
$daemon = (Resolve-Path target/debug/milkdrift-daemon.exe).Path
$cli = (Resolve-Path target/debug/milkdrift.exe).Path
$examples = (Resolve-Path examples/operator).Path
$operatorDirectory = Join-Path $env:USERPROFILE ('Milkdrift/operator-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $operatorDirectory | Out-Null
Copy-Item "$examples/daemon.toml", "$examples/starter.json" $operatorDirectory
Set-Location -LiteralPath $operatorDirectory
$env:MILKDRIFT_TOKEN = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32))
& $daemon --config daemon.toml --check-config
if ($LASTEXITCODE -ne 0) { throw 'Invalid daemon configuration' }
$daemonProcess = Start-Process $daemon -ArgumentList '--config', 'daemon.toml' -WorkingDirectory $operatorDirectory -PassThru -WindowStyle Hidden
$readyBy = [DateTime]::UtcNow.AddSeconds(10)
do {
    & $cli --json --timeout-secs 1 daemon readiness
    if ($LASTEXITCODE -eq 0) { break }
    Start-Sleep -Milliseconds 100
} while ([DateTime]::UtcNow -lt $readyBy -and !$daemonProcess.HasExited)
if ($LASTEXITCODE -ne 0) { throw 'Daemon did not become ready' }
$import = & $cli --json --command-id starter-import blueprint import starter.json | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Blueprint import failed' }
$revision = $import.value.value.revision_id
& $cli --json --command-id starter-start run start run-starter operator-starter $revision
if ($LASTEXITCODE -ne 0) { throw 'Run start failed' }
& $cli --json --timeout-secs 10 run wait run-starter --terminal succeeded
if ($LASTEXITCODE -ne 0) { throw 'Starter did not succeed' }
& $cli --json run timeline run-starter --limit 100
```

Keep this shell for subsequent authenticated CLI calls. Secrets belong in process environment or
private referenced files, never argv or source. Paths in configuration resolve against its
directory. The listener uses loopback; if the default port is occupied, change `bind` and set
`MILKDRIFT_ENDPOINT` to the same address. For a Unix shell and foreground shutdown/restart, use the
[operator guide](examples/operator/README.md#startup-and-restart).

Next run [one byte-pinned local process](examples/operator/README.md#one-byte-pinned-local-process)
or [one separately managed model](examples/operator/README.md#one-separately-managed-loopback-model).
[Prompt sequences](docs/guides/headless-dogfood.md) compose coding, verification, review, and
prospective remediation. [Daemon operations](docs/operations/daemon.md) covers retention and backup.

## Read the repository

- [Vision](docs/product/vision.md): intended experience and success criteria.
- [Architecture](docs/architecture.md): terminology, invariants, dependency direction, and package owners.
- [Learning the implementation](docs/README.md#learning-the-implementation): six source traces with
  black-box commands and independent tests.
- [Development workflow](docs/development/workflow.md): the full gate and focused suites.
- [Documentation index](docs/README.md): operator, wire, schema, evidence, and decision references.

Contributors start with [AGENTS.md](AGENTS.md). Milkdrift is licensed under either
[MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
