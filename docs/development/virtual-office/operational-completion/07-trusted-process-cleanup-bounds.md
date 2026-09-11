# Assignment 07 — Bound owned process I/O cleanup honestly

## Outcome

Remove the observed possibility that inherited pipes keep the daemon's owned I/O threads joining
indefinitely. Bound Milkdrift-owned shutdown while preserving truthful reporting of external work.
Retain `TrustedHostProcess`: this assignment does not create a sandbox, network/CPU/memory isolation,
or a universal ability to terminate escaped or malicious descendants.

Read the sprint README, `AGENTS.md`, canonical docs, workflow and implementation/documentation
practices, local-process README, daemon shutdown and current platform qualification. Preserve the
existing byte-pinned identity and child/registration/worker ownership; do not replace working lifecycle
code with timeout flags that leak threads.

## Source trail and concrete reproduction

Inspect `adapters/local-process/src/process/lifecycle.rs`, `streams.rs`, `monitor.rs`, `spawn.rs`,
platform helpers, lifecycle tests, shared process helper, capability-host effect workers, daemon
shutdown/join path, and `.github/workflows/platform.yml`.

The reviewed lifecycle retains child and I/O handles through reporting/setup failure and unwinding.
Cleanup disconnects the channel and terminates owned processes, then joins stdin/stdout/stderr
workers. A descendant outside the owned process group can retain a pipe without writing or closing
it. Disconnecting the Rust channel does not by itself interrupt a blocking pipe read/write.

First create a bounded deterministic fixture for this condition, with explicit fixture cleanup so
tests never leave an actual orphan. Use synchronization and observable handle/worker completion,
not a long sleep followed by an assertion that the suite did not hang.

## Implement the smallest owned cancellation mechanism

Make local pipe reads/writes interruptible through a sound platform mechanism or an existing
maintained process/I/O abstraction. No custom unsafe code contrary to repository rules, unsound
cross-thread handle close, forced thread termination, detached timed-out readers, unbounded reaper
queue, or a second process supervisor product. Explain the selected mechanism and cancellation
ownership before adopting a new dependency.

The cancellation/shutdown budget must apply to I/O cleanup, not only waiting for the direct child.
Use a monotonic local deadline for elapsed cleanup; retain the established durable boundary clock
for persisted facts. Define ordering for stopping new input, closing/disconnecting owned channels,
requesting child/group termination, interrupting local reads/writes, collecting available bounded
output, joining owned workers, and releasing registrations/permits. Partial startup and panic paths
must follow the same ownership rule. Do not hide an unbounded join in `Drop`.

Separate what is known: parent exit, owned group termination, local I/O closure, complete/truncated
capture, and unresolved external descendants/effects. Interrupting local pipes proves local worker
cleanup, not remote/descendant termination. Preserve that distinction in typed failure/cleanup
observations, artifacts, inspection and account/uncertainty handling. Never emit fabricated success
or cancellation, release unknown incurred usage as zero, or discard a terminal reporting failure.

Retain bounded stdout/stderr and backpressure. A child ignoring termination, blocked stdin,
inherited pipe, full reporting channel, failed reporter, and shutdown during partial worker creation
must all leave ownership explicit. If a platform cannot support a claimed bound safely, narrow its
advertised guarantee/configuration and refuse impossible requests before entry; do not silently
pretend platform parity. An unresolved platform case must be reported, not marked passed by
cross-compilation.

## Acceptance tests

Exercise normal parent/child cleanup, inherited idle stdout/stderr beyond the group, blocked stdin,
output saturation, reporter failure, partial stream-worker spawn failure, panic unwinding, timeout,
and repeated cancellation. Drive both local process invocation and daemon drain/cancel/retain modes.
Prove owned thread/handle/permit counts return to their defined settled state and bounded deadlines
actually cover joins. Verify truthful unresolved-effect evidence and no automatic duplicate retry.

Use the existing byte-pinned process helper and platform suites. Test supported OS behavior on actual
runners and report unexecuted environments. Do not globally skip lifecycle tests to make CI green.
Include a normal output-producing case to catch lost final bytes caused by over-eager interruption.

## Verification, explanation and stop

Run local-process lifecycle, shared host lifecycle, daemon shutdown and relevant peer-host process
cases, then the full gate and changed platform suites. Document which resources are owned and
bounded, what external effects remain uncertain, and what trusted-host privileges still allow.
Use current adapter/operator docs; no new security marketing or blanket containment claim.

Stop when the reproduced inherited-pipe failure no longer blocks owned daemon cleanup on supported
platforms, no abandoned thread/reaper growth replaces it, normal output behavior survives, and
external termination claims remain evidence-based. Do not proceed to OS sandboxing or resource-quota
implementation inside this assignment.
