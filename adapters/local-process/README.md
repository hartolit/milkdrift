# Local process adapter

Use this adapter to run a configured executable as a workflow task or independent invocation: a compiler, verifier, or
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

During preparation, the adapter materializes only configured inputs, selects the working directory,
expands argument placeholders, and freezes stdin and the environment. The runtime or serving owner
then rechecks authority and durably authorizes entry. Only the consumed prepared handle can spawn,
after rechecking executable identity. Direct inputs use their explicit selection and host-invocation
artifact owner; workflow inputs retain their exact causal manifest. Each argument template produces one OS argument. A substituted
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

### Bound local cleanup without claiming descendant containment

Before spawn, the adapter creates ordinary blocking child endpoints and nonblocking daemon
endpoints. Unix uses the existing `rustix` safe `fcntl` wrapper. Windows uses `interprocess` only
to set `PIPE_NOWAIT` on synchronous pipe handles and preserve raw read errors. Its unnamed
receiver distinguishes an idle pipe from EOF, which `std`'s Windows reader conflates. It refuses a
handle unexpectedly reopened for overlapped I/O before spawning. It never uses that dependency's
flush or background limbo pool. The stdin worker owns a synchronous `File` writer rather than
calling `ChildStdin`'s asynchronous Windows write method on that handle. This keeps pipe
cancellation in the existing invocation owner, without cross-thread handle closure or detached I/O work.

Each worker checks its stop signal between nonblocking operations and bounded-channel sends.
Idle/full pipes and a full channel wait at most one 5 ms polling interval before checking again.
Cancellation, timeout, or overflow stops input and starts one monotonic cleanup deadline covering
the configured graceful and forced termination intervals. The monitor requests termination while
continuing bounded output collection. Ordinary parent exit starts a final-output window of
`forced_termination_ms`. EOF ends collection early; expiration interrupts the remaining readers.
The same deadline covers child/group observation; cleanup does not start another wait allowance.
As with other local deadlines, OS scheduling and individual system calls are not hard real-time
promises.

The owner signals I/O interruption, requests force where still needed, polls child/group state
only until that deadline, then joins every worker before releasing the cancellation registration
and host permit. Workers close their own handles. After monitoring, cleanup collects the remaining
bounded queue and each joined reader's EOF observation so slow progress reporting does not discard
already collected final output. Failure and panic paths disconnect the channel and use the same
ownership rule, with a forced-termination allowance when monitoring has not started. There is no
blocking `child.wait()` hidden in `Drop`. If reporting failed, its original error propagates
without a replacement terminal report.

A missing stdout/stderr EOF becomes `process_io_incomplete` with `Uncertain` status, even when the
parent exited successfully or cancellation was requested. The terminal observation separates
parent exit, owned termination, local joins, EOF, captured byte counts, and observed overflow.
Its explanation survives restart and is exposed as the attempt's `terminal_detail`; the ordinary
uncertain-work rules retain unresolved controller reservations and refuse unsafe duplicate retry.
Incomplete captures and declared files are not published as successful outputs. Complete output
continues to use the configured capture/truncation policy. Closing local pipes does not prove
that an escaped descendant or its external effects stopped.

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
