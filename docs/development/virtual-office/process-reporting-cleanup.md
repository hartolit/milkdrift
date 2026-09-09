# Repair process cleanup after reporting failure

Implement the [process-reporting cleanup issue](whiteboard/issues/process-reporting-cleanup.md).
Follow [AGENTS.md](../../../AGENTS.md), its reading order, and the standard development workflow.

The local-process adapter can leave a child and its I/O threads running when the initial report
after spawn is rejected. A later progress or heartbeat failure can also lead it to join I/O before
requesting child termination. Recheck those paths in the current tree and fix the lifecycle as a
whole, including related post-spawn exits that can lose ownership of a child or thread.

Start with `LocalProcessAdapter::execute_inner` in
[process.rs](../../../adapters/local-process/src/process.rs), the
[monitor](../../../adapters/local-process/src/process/monitor.rs), stream workers, platform
termination, and active registration. Trace the host permit and runtime reporter where their
ordering affects cleanup. Choose the smallest design that keeps the child and every started I/O
thread owned through cleanup, requests termination before waiting on pipes that the child may
keep open, and releases registration and capacity only after their corresponding work is settled.

Demonstrate the failure with a long-running test child, then verify rejection of initial progress,
streamed progress, and heartbeat reports. Tests must fail within a bounded deadline if cleanup hangs,
and must clean up their own test processes on failure. Observe child termination and completion
of the started I/O workers; returning an adapter error or releasing a permit alone is insufficient.
Preserve normal completion, cancellation, timeout, output-limit, and shutdown behavior. Distinguish
Unix owned-descendant evidence from the immediate-child guarantees available on other platforms.

Propagate reporting failures without inventing a successful terminal observation or treating the
external operation as if it never entered. Preserve the runtime's handling of uncertain outcomes.
Do not expand sandboxing, process-tree portability, provider support, or unrelated workflow behavior.

Complete the fix under the repository's implementation and verification rules, including the
required full gate and relevant lifecycle regression evidence. Reconcile the whiteboard issue with
what the checks establish. Stop with the completed change, checks actually run, and any remaining
platform or lifecycle limits. Do not begin the other whiteboard issues.
