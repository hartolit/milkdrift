# Assignment 07 handoff

Owner: current Codex task, explicitly assigned by the user. Base: `5c4d612`. Implementation:
`30b5d60` with platform corrections through `515337f`, on `codex/trusted-process-cleanup-bounds`.
The implementation and verification are ready for coordinator acceptance; assignment 08 is unstarted.

The existing process owner now creates nonblocking daemon pipe endpoints before entry, interrupts
its three I/O workers and joins them before releasing registration and the host permit. One
monotonic deadline covers child/group observation and output draining. Failure, partial startup
and unwinding keep the same ownership. Blocking pipe operations, blocking channel sends and the
blocking child wait in cleanup are removed. Windows uses the maintained `interprocess` safe
handle API without its flush/limbo pool and a synchronous `File` stdin writer; Unix uses the existing
`rustix` dependency. A synchronous pipe must not call Windows `ChildStdin`'s asynchronous write
method: the hosted runner reproduced a stuck completion callback on an ordinary 18-byte input. The
[adapter README](../../../../adapters/local-process/README.md#bound-local-cleanup-without-claiming-descendant-containment)
owns the mechanism, ordering, selected dependency and limits.

The bounded reproduction failed on the former implementation because an idle descendant outside
the owned process group retained stdout/stderr. Explicit PID/release-file fixture cleanup handles
both success and test failure. The regression now completes invocation cleanup while that external
holder remains alive, including parent exit, timeout, repeated cancellation and adapter shutdown.
Missing EOF produces uncertain work with separate parent/group, local join and capture facts.
Incomplete outputs are not published. Unknown usage stays unknown and unsafe retry remains refused.
Attempt inspection now exposes the existing durable uncertainty reason through `terminal_detail`,
including after restart. No protocol, profile, storage or public Rust API shape changes.

Linux/Rust 1.95 verification passes:

- The full [workflow gate](../../workflow.md#full-local-gate): formatting, all-target/all-feature
  checking, **809 workspace tests**, **24 doctests**, warning-denying Clippy/rustdoc, dependency
  audits, test discovery and all **24 repository contracts**. Five manual longevity cases remain
  ignored and were not rerun. Two exact transitive Windows dependencies have scoped `0BSD` license
  exceptions; existing duplicate-version warnings remain non-failing.
- `cargo test -p milkdrift-local-process --all-features`: six private lifecycle tests and 33
  integration tests. They cover complete final bytes, saturated output/reporting queues, unread
  stdin, a stopped child requiring force, inherited pipes, initial/progress/heartbeat rejection,
  reporter panic and partial worker startup. Completion gates verify joins precede unregistering.
- `cargo test -p milkdrift-capability-host --all-features`, plus daemon `control_plane` and
  `two_daemon_peer` suites. The daemon has 16 passing control-plane cases; the peer suite has two
  passing cases and one manual longevity case ignored. The new daemon case exercises drain,
  cancel and retain, reopens the same store and proves uncertainty without duplicate entry.
- Actual-binary operator, deterministic model and controller qualification lanes using the rebuilt
  daemon, CLI and process helpers. The model lane uses `--output target/process-cleanup/final-model-evidence`
  to preserve earlier evidence. No real provider was contacted.
- Default/all-feature public API inventories for local-process, daemon and control-protocol are
  reviewed under `target/public-api/`; each pair is identical. The new cleanup types remain private.

Raw local logs and reports are under ignored `target/process-cleanup/`; `verified-gate.json`
records the passing full gate and `binary-lanes.json` records the actual-binary runs. The initial
reproduction failure and Windows diagnostic are retained there. The platform workflow now includes
shared host lifecycle tests and the exact daemon inherited-pipe shutdown/restart case. The hosted
macOS run also exposed an overlong socket address in a newly included host fixture; the fixture
now binds a short address before moving the same special file into the materialized workspace.
Its refusal assertions are unchanged. The corrected candidate passes
[35023792743](https://github.com/hartolit/milkdrift/actions/runs/35023792743) on Ubuntu 24.04,
Windows 2025 and macOS 15. Each actual runner checks the complete workspace, executes the selected
domain/protocol/client/shared-host/process suites and passes the daemon shutdown/restart case.
The Windows normal stdin, unread stdin and inherited-pipe cases all pass. Retained platform logs
are under `target/process-cleanup/platform-515337f/`. These selected runtime suites do not establish
a complete Windows or macOS workspace gate.

Local pipe closure proves ownership cleanup, not termination of escaped descendants or their
effects. Non-Unix ownership remains the immediate child; Unix process groups are not a sandbox.
Daemon-account privileges, no CPU/memory/network isolation, the executable check-to-spawn race,
and OS scheduling limits remain explicit. Production controller activation and broader external
qualification are unchanged. The next assignment should consume the exact platform results and
these maintained limits. The previously pending macOS lifecycle qualification is now covered by
this actual runner evidence.
