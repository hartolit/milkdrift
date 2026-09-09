# Local process adapter

Use this adapter to run a configured executable as a workflow task: a compiler, verifier, or
coding agent, for example. A `ProcessProfile` fixes the executable's bytes, arguments, working
directory, inputs, outputs, and limits. `LocalProcessAdapter` binds that profile to the machine and
advertises the resulting generation to the [capability host](../../crates/capability-host/README.md).

## Configure and follow an invocation

Begin with the [ordinary process example](../../examples/operator/README.md#one-byte-pinned-local-process)
and the [profile guide](../../docs/guides/local-process.md). `ProcessProfileDocument::from_json`
reads the strict profile; `LocalProcessAdapter::new` verifies the executable and canonical host
paths before producing `descriptor()`. The daemon uses this path in
[capability registration](../../apps/daemon/src/host/capabilities.rs). The adapter needs an injected
`InvocationDataAccess` and `SecretResolver`, so it can use durable inputs without knowing store layout.

After runtime authorizes entry, the adapter materializes only configured inputs, selects the working
directory, expands argument placeholders, prepares stdin and the environment, then rechecks executable
identity immediately before spawning. Each argument template produces one OS argument. A substituted
value containing spaces or shell metacharacters stays in that argument; the adapter does not insert
a shell. The configured executable still determines how it interprets those arguments.

`WorkingDirectoryMode` is a consequential choice. Isolated modes use the leased execution directory.
`AuthorizedHostPath` enters an explicitly allowed persistent repository while input materialization
and declared output paths remain under the separate leased root. Sequential invocations can therefore
share repository progress; parallel branches need explicitly separate workspaces.

The child environment starts empty and receives only allowlisted non-secret variables and resolved
secret references. A secret-bearing profile must disable text progress streaming. Captured stdout
and stderr have exact secret-byte matches replaced before publication; declared output files remain
the executable's responsibility.

## Results and interruptions

Reader threads drain stdout and stderr through a bounded channel. Each stream has its own capture
ceiling and progress-event allowance. `ContinueTruncated` drains beyond the capture limit without keeping
the extra bytes; `Terminate` asks the monitor to stop the process. After a successful observed exit,
the adapter publishes configured captures and declared regular files through the host data port.
Missing required outputs or publication failure prevent success; undeclared files are not imported.

Cancellation sets an invocation-specific flag. The monitor owns graceful/forced termination and
the evidence used to distinguish `Cancelled`, failure, and uncertainty. On Unix it observes the
owned process group and tears down remaining descendants when the immediate child exits. Non-Unix
ownership covers the immediate child only. A PID is not recoverable process identity after daemon
restart; `RestartPolicy` describes whether the external program can safely accept the same stable key.

If reporting fails after spawn, the invocation owner disconnects the stream channel, requests
forced termination, reaps the child, and joins every started I/O worker before releasing cancellation
registration. This also covers partial worker startup and unwinding into the host's panic boundary.
Disconnecting the channel releases readers blocked on a full queue; terminating first releases pipes
the child holds open. The original reporting error propagates without a synthetic terminal report,
so runtime still retains uncertainty about the external operation.
Cleanup depends on OS termination and pipe closure. Descendants outside the owned process group
(or any descendant on non-Unix platforms) can retain inherited pipes and delay I/O joins.

This is a trusted host process with the daemon account's privileges. Staging checks do not provide
a sandbox, CPU/memory quotas, network isolation, or containment of malicious descendants. Byte
verification also leaves a check-to-spawn race; see the platform qualifications in the
[operator guide](../../docs/guides/local-process.md#execution-and-trust-boundaries).

## Change the implementation

[`process.rs`](src/process.rs) connects preparation, spawn, monitoring, and publication. Its private
children keep byte identity, [child and worker lifetime](src/process/lifecycle.rs), platform ownership,
streams, and result reporting close to the mechanism they enforce. Health rechecks byte identity
and latches failure: restoring old bytes does not revive an invalidated generation. Deploy a changed
executable with a new profile/descriptor revision.

The [process execution suite](tests/process_execution.rs) uses a byte-pinned Rust helper to exercise
literal arguments, materialization, output limits, identity replacement, lifecycle, and shared adapter
conformance. Reporting regressions hold a child and its pipes open, inject initial/progress/heartbeat
failure, and check child exit under a deadline with fallback fixture cleanup. Private lifecycle tests
hold I/O workers at completion to verify registration persists until all joins finish. Unix descendant
tests run only on Unix; passing the Windows suite does not qualify them.
