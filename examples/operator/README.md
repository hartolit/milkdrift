# Submit a daemon-owned task

Use this directory to configure a new daemon root and submit ordinary tasks through the same API
as the standalone Svelte client. Existing stores and services are not adoption targets. Copy the
configuration into an explicitly selected empty directory; paths then resolve from that directory.

## Configure the host and account

Build `milkdrift-daemon` and `milkdrift` from the repository root. The maintained `daemon.toml` uses
configuration schema 14, host identity `host:local` and loopback port 9734. Change the port if occupied.
Set a private `MILKDRIFT_TOKEN` in the daemon's environment or replace the environment secret source
with a permission-restricted file. Do not commit the value or put it in a URL. The client reads a
credential from its configured token file; it does not infer the daemon's environment.

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon -p milkdrift-cli --bin milkdrift
milkdrift-daemon --config /absolute/operator/daemon.toml --check-config
milkdrift-daemon --config /absolute/operator/daemon.toml
```

The template grants an operator exact process capability access and finite budgets. Dynamic workflow
IDs and uploaded artifact identities require explicitly acknowledged broad selectors in this initial
operator configuration. `owner_actor` is provenance, not an implicit privacy filter. Narrow later
reader grants to known workflow IDs and review artifact sensitivity separately. See
[authority configuration](../../docs/operations/authority.md).

No process/model adapter is configured by default. Copy `process-profile.example.json`, replace
its executable path and exact digest/size with the trusted tool you reviewed, and choose its input,
output, filesystem and lifetime contracts. The supplied printf profile is only a basic installation
probe. For useful work configure a trusted Rust tool that consumes supplied content and produces a
report; the sprint's `connected-tasks` harness demonstrates Cargo-project inspection. Add the
profile path to `adapters.process_profiles`, and grant its exact executable/work roots. A profile
allowlist alone grants no filesystem access. The [process guide](../../docs/guides/local-process.md)
explains materialization and protected lifetime behavior.

For a model, use the maintained endpoint profile examples, configure its network/secrets and
finite billing/token bounds, and grant `operator-model` / `model.generate` with its actual effect
and trust facts. Model calls are external; configuring a model does not authorize paid execution.
The [model guide](../../docs/guides/local-model-endpoint.md) explains supported request contracts.

## Submit, discover and recover

Configure `MILKDRIFT_URL` and `MILKDRIFT_TOKEN_FILE`, or use the CLI's `--url` and `--token-file` flags.
Inspect available choices and substitute the exact advertised capability in the request file.
`process.json`, `model.json` and `starter.json` are `CreateWorkflowRequest` documents, not graphs.
`peer-placement.json` requires replacing its placeholder with the advertised exact peer capability.

```sh
milkdrift --json workflow capabilities
milkdrift --json workflow create process.json
milkdrift --json workflow list --search Operator
milkdrift --json workflow show WORKFLOW_ID
milkdrift --json workflow task WORKFLOW_ID TASK_ID
milkdrift --json workflow result WORKFLOW_ID TASK_ID
milkdrift --json workflow receipt operator-process-1
milkdrift --json workflow upload Cargo.toml --request cargo-manifest-1
```

Create returns daemon-owned IDs and an alias map. Request IDs bind exact canonical bodies. Repeating
an unchanged file replays its receipt; changed reuse conflicts. After a lost response inspect the
receipt or discover accepted work. A fresh client needs only its endpoint and credential, not a
saved workflow ID. A generated upload receipt names immutable captured text; bind it with a
`bind_input` change under the observed workflow generation. The
[API reference](../../docs/reference/control-api.md) contains complete create/upload/edit examples.

```sh
milkdrift --json workflow edit WORKFLOW_ID change.json
milkdrift --json workflow stop WORKFLOW_ID TASK_ID
```

Edits affect future work and reject stale versions or frozen starts. Stop reports a request until
evidence proves stopped. An entered task without a durable outcome remains uncertain. Complete
output is available through the result-content or immutable artifact routes; task JSON includes
labelled bounded previews. Results requiring missing artifact permission remain restricted.

## Use the browser

Start the standalone client using [its README](../../apps/web/README.md), or serve its built static
files independently. The template allows exactly `http://127.0.0.1:5173` and
`http://127.0.0.1:4173`. In **Connect a daemon**, enter its endpoint, expected `host:local` identity
and scoped credential. The credential remains in memory. Closing the entire browser does not stop
daemon work; reconnect and use discovery to find the result. Two connections retain separate
credentials, settings and selections.

Phone/LAN deployment requires browser-trusted HTTPS for the static site and daemon proxy. Loopback
HTTP is the development exception. These configuration instructions do not qualify a physical
phone or proxy deployment; test the actual arrangement before relying on it.

## Independent execution hosts

`execution-only.toml` configures the same daemon without task scheduling. Its direct invocation,
artifact upload/download, managed-resource and peer serving paths remain available under explicit
authority. `direct-process-inputs.json` and `direct-model-task.json` demonstrate independent input
contracts. See [daemon operation](../../docs/operations/daemon.md) and the
[CLI guide](../../apps/cli/README.md) for direct invocation and lifecycle commands. Workflow-backed
publication and evaluated-learning methods are retired; independent host protections remain.
