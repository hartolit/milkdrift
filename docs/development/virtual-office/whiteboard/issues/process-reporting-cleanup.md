# Process cleanup when durable reporting fails

Source inspection found exits from the local-process adapter that do not establish cleanup of
the child and its I/O threads after a reporter failure. Repair belongs in a focused local-process
adapter change with lifecycle regression tests.

## Current technical assessment

In [`LocalProcessAdapter::execute_inner`](../../../../../adapters/local-process/src/process.rs),
the child is spawned, registered, and given stdin/stdout/stderr threads before the initial
`local process started` progress report. If that report fails, `?` returns before the monitor,
termination, or thread joins. Dropping `ActiveRegistration` only removes the cancellation-map
entry; it does not terminate the child. Dropping the thread handles does not join them. The host
permit is released, so released capacity is not evidence that the OS resources ended.

A second path occurs when the [monitor](../../../../../adapters/local-process/src/process/monitor.rs)
returns an error from a progress report or heartbeat. `execute_inner` attempts to join the I/O
threads before matching that error and calling `terminate_child_immediately`. If the child still
holds pipes open, cleanup can wait on the child before requesting its termination. This is an
ordering problem even on platforms where ordinary cancellation tests pass.

The [shared conformance suite](../../../../../crates/capability-host/src/conformance.rs) checks
that reporter rejection propagates. Its
[process fixture](../../../../../adapters/local-process/tests/process_execution.rs),
`process_conformance_case`, runs the helper with `exit 0`; it does not establish cleanup of a
long-running child when the first report fails. Existing lifecycle tests exercise cancellation,
timeouts, and owned descendants without this combined reporting failure.

The [runtime effect owner](../../../../../crates/runtime/src/engine/effects/entry.rs) records
uncertainty when entered work returns without terminal evidence. That preserves workflow truth,
but cannot release resources that the adapter has stopped owning. A focused follow-up should
arrange child termination and I/O joining on every post-spawn exit, and use bounded tests with
a long-running child for initial-report, progress-report, and heartbeat rejection. Unix descendant
and non-Unix immediate-child evidence should remain separately qualified.

## Contributions

2026-09-08 — Alder-20260908-c (agent pseudonym), documentation-clarity phase 03: traced the
adapter, monitor, active-registration drop, host permit drop, runtime reporter, and conformance
fixture at base `52cf340` with documentation-only edits. This is a source-derived finding;
no failure-injection execution, new regression test, or production incident is claimed.
