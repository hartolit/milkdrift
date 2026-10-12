# Verification evidence tools

This unpublished Rust package exercises product APIs and actual binaries. Use a lane whose
observations support the claim being made. Product crates never depend on the evidence harness.

| Tool | Purpose |
| --- | --- |
| `strict-checks` | Compiler, Clippy, feature, dependency, repository and redacted secret checks with retained command status. It runs no product scenarios. |
| `connected-tasks` | Two isolated production daemons, a trusted input-dependent Cargo report process and controlled model endpoint. `serve` supports the browser journey; `check` runs a finite CLI/public-client/restart scenario and verifies cleanup. |
| `milkdrift-external-evidence` | Real coding-process/model interoperability through ordinary daemon tasks, corrective work and strict redacted schema-2 reports. Explicit fixture mode never qualifies. |
| `operational-evidence` | Bounded storage, queue, observation and shutdown measurements through maintained owners. Platform-specific observations identify their platform. |
| `core_paths` | Timing selected maintained storage, capability and daemon paths; measurements are not correctness budgets. |
| `mutation-evidence` | Whether focused tests detect particular implementation mutations. A baseline failure, timeout or unrelated fixture failure does not qualify a mutation as caught. |
| `evidence-process-helper` | Deterministic native process behavior used by tests; it is never a real coding-agent qualification. |

Build the production binaries before actual-application scenarios. Choose a fresh ignored output
root or private external directory and preserve existing stores. The connected browser setup is:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon -p milkdrift-evidence --bin connected-tasks
cargo connected-tasks serve --root target/connected-browser --daemon target/debug/milkdrift-daemon
```

The owner writes a private `browser.json` only after both daemons are ready. Supply its absolute path
as `MILKDRIFT_BROWSER_CONFIG` to the [web browser suite](../../apps/web/README.md). The fixture's
`control --root DIRECTORY` subcommand requests bounded restart/crash/stop actions from its live Rust
owner. Tokens and private request bodies must not be printed or retained in browser traces.

The finite application lane uses those same daemons and capabilities:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon -p milkdrift-cli --bin milkdrift -p milkdrift-evidence --bin connected-tasks --bin milkdrift-external-evidence
target/debug/connected-tasks check --root target/task-client-evidence --daemon target/debug/milkdrift-daemon --cli target/debug/milkdrift
target/debug/milkdrift-external-evidence --daemon target/debug/milkdrift-daemon --fixture --allow-fixture --output target/task-external-evidence
```

`check` uploads a Cargo manifest through the real CLI, edits the local source after capture,
submits and replays a task, restarts its daemon, and uses a fresh public client to discover the
same result and receipt. It checks changed-request refusal, daemon isolation and the controlled
model's selected input bytes. A redacted `check.json` appears only after both daemon children and
the model fixture have shut down successfully. The private root also contains credentials and
model request captures: retain only the explicitly redacted report in shared CI artifacts.

The old headless graph, local-model graph and installed-controller qualification executables
were retired under [ADR 0051](../../docs/decisions/0051-daemon-task-workflows.md). Current quality
CI builds and executes the task lanes above. The manual stress lane retains peer turnover and
tests current task capacity, replay and recovery; it does not claim the former graph compaction
or controller longevity behavior. Mutation shards now target `authority`, `retention`, `runtime`,
`uncertainty`, `tasks`, `inputs` and `peer`. Old exact-mutant exceptions were removed with their
source owners; new survivors require their own reviewed classification.

The [external guide](../../docs/guides/external-evidence.md) owns real profile prerequisites,
qualification and report handling. Passing a local fixture says that the harness worked, not that
a real provider or installed process qualified.

## Extend an observation

`application` owns child launch, bounded capture, readiness, deadlines and lifecycle cleanup.
`OwnedChild::terminate` kills and reaps for abrupt restart. `shutdown` separately checks public signal
shutdown where supported; these observations are not interchangeable. Failure retains cleanup
uncertainty, and Drop does not silently retry a failed explicit cleanup. An observation's overall
deadline includes nested daemon reads and child commands.

`http_fixture` supplies bounded loopback framing. A handler failure, panic or failed join prevents
successful fixture completion. The fixture never manufactures workflow outcomes; it is an external
capability driven by the production daemon. Backend executables and helper operations stay in Rust.

The external runner separates profile validation, generated task requests, scenario execution and
report qualification. `repository_contracts`, operational contracts and external integration tests
check different boundaries. Preserve useful failure output under the selected evidence root and
follow the [verification policy](../../docs/development/workflow.md#choose-verification-for-the-change).
The current [verification evidence](../../docs/development/verification-evidence.md) identifies
executed qualification and its limits; configured tools are not themselves passing evidence.
