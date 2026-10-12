# Milkdrift

Milkdrift lets people and agents work through a durable daemon-owned task collection. Submit a brief
and captured files, run a configured model or trusted process, inspect its result, and reconnect
later from another client. Future work can change without changing the facts recorded at start.
Models and tools run outside the core.

The current slice includes a standalone Svelte client, CLI and public task API. Independent execution
hosts and managed-resource protections remain available. See [status](docs/product/status.md) for
what is implemented and [roadmap](docs/product/roadmap.md) for the finite remaining assignments.

## Fresh-directory quick start

Build the actual executables and the Rust connected-task harness:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon -p milkdrift-cli --bin milkdrift -p milkdrift-evidence --bin connected-tasks
cargo connected-tasks --help
```

The [Svelte README](apps/web/README.md) gives the exact setup, static-client and browser-test commands.
The harness uses a new explicitly selected empty directory, finite ports and a trusted Rust process
that reads a supplied Cargo manifest and reports its real targets/dependencies. It also provides a
controlled model endpoint for transport tests. Those fixtures do not create tasks or fake task
completion. Submit through the browser or the same public CLI:

```sh
milkdrift --endpoint http://127.0.0.1:9734/ --token-file /private/operator.token --json workflow capabilities
milkdrift --endpoint http://127.0.0.1:9734/ --token-file /private/operator.token --json workflow list
milkdrift --endpoint http://127.0.0.1:9734/ --token-file /private/operator.token --json workflow create task-request.json
milkdrift --endpoint http://127.0.0.1:9734/ --token-file /private/operator.token --json workflow receipt REQUEST_ID
milkdrift --endpoint http://127.0.0.1:9734/ --token-file /private/operator.token --json workflow show WORKFLOW_ID
milkdrift --endpoint http://127.0.0.1:9734/ --token-file /private/operator.token --json workflow task WORKFLOW_ID TASK_ID
milkdrift --endpoint http://127.0.0.1:9734/ --token-file /private/operator.token --json workflow result WORKFLOW_ID TASK_ID
```

Use the endpoint and private credential file returned by your setup. Never paste a token into an
argument, URL or repository file. Workflow/task IDs come from the acceptance receipt; if its reply
is lost, look up the original request ID or discover the workflow. Reusing that request with changed
content conflicts. The [operator examples](examples/operator/README.md) describe normal configured
operation independently of the test harness.

This release requires a new store root. Earlier development stores are refused before mutation;
keep them with their preserved readers. Do not reuse an existing service's root or stop its process
to try the task client. A client disconnect or CLI timeout does not stop accepted work.

## Read the repository

- [Vision](docs/product/vision.md): intended experience and non-negotiable semantics.
- [Architecture](docs/architecture.md): task ownership, external entry and durability.
- [Documentation index](docs/README.md): operator, protocol and implementation references.
- [Development workflow](docs/development/workflow.md): assignment scope and verification lanes.

Contributors start with [AGENTS.md](AGENTS.md). Milkdrift is licensed under either
[MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
