# milkdrift-runtime

Runtime turns authorized commands and observed execution results into durable workflow progress.
It plans events, reconstructs current state, schedules eligible work, and decides what can resume
after interruption. It uses [persistence ports](../persistence/README.md) for storage and
`TaskExecutor` for external capabilities. The service is synchronous; the
[capability host](../capability-host/src/worker.rs) owns production worker threads.

## Follow a start command

Assume an immutable revision and a created run already exist. A caller submits `StartRun` in a
`RunCommandDocument`, with a stable command identity, expected run sequence, reason, and exact
authority claim. [`RuntimeService::handle_authorized_command`](src/engine.rs) first looks for a
saved result matching that command's intent. A match returns the original result; conflicting
intent under the same identity fails.

For a new command, runtime evaluates authority and checks the current run. The
[start planner](src/engine/command_planning/admission.rs) proposes `RunStarted` and eligible entry
nodes; the authority path binds the allowed start decision for later execution. The
[commit planner](src/engine/command_planning/commit.rs) applies those events to a candidate
projection and derives workspace, account, and discovery changes. `RunJournal::commit_command`
saves them with the receipt and response in one transaction. A durable rejection instead saves
its response and decision without semantic events.

This commit makes work eligible. A later `scheduler_tick` resolves capabilities and inputs within
the configured admission limits and saves the exact invocation request, attempt, and lease.
`claim_execution_effects` then records ownership and returns actions only for newly committed
claims. Workers call `execute_effect` outside the scheduler. Immediately before adapter entry,
runtime rechecks authority, lease and run state, prepares the exact generation, and commits the
entry decision with any controller reservation. A replayed entry decision cannot enter again.

```text
start command -> receipt + events -> eligible work
                                         |
                              scheduler saves attempt/lease
                                         |
                              worker claims exact effect
                                         |
                         final entry decision + reservation
                                         |
                                   external adapter
                                         |
                         reporter -> durable observations
                                         |
                             terminal/structured progress
```

`ExecutionReporter` accepts sequenced observations incrementally. Runtime validates and commits
each one before acknowledging it; adapters cannot write projections or journal events directly.
A durable terminal report takes precedence over a later worker error. An adapter returning
without terminal evidence leaves an uncertain outcome even if some progress or output was saved.

## Recover the right boundary

There are different questions after an interruption:

| Saved evidence | What the caller can do |
| --- | --- |
| Command receipt/result, reply lost | Redeliver the same identity and intent to recover the result without another append. The daemon separately preserves the complete external request/response. |
| Scheduled attempt, expired lease, no recorded start | Recovery may rotate ownership of the same immutable attempt. |
| Recorded start/entry, no terminal evidence | Recovery classifies the frozen effect/idempotency facts. Unknown or non-idempotent effects remain unresolved; eligible safe retries still obey retry limits and timers. |
| Durable terminal observation | Replay preserves the outcome, even if the worker later fails to return. |

An error while reading storage does not prove that a receipt or terminal fact is absent. A
cancellation request, acknowledgement, expired lease, or closed connection does not prove the
external action stopped. [`ResolveExternalWork`](src/command.rs) records an authorized decision
to retry when safe, retain, arrange remediation, or resolve from evidence. A query decision records
intent; any external query uses a separately authorized capability. These are new facts, never
edits to the old attempt. [ADR 0004](../../docs/decisions/0004-side-effects-retries-and-uncertain-outcomes.md)
owns the effect-safety rule.

## Run structured work and select its inputs

Blueprint [structured definitions](../blueprint/README.md) become durable execution occurrences.
Branch selects one arm; fork creates isolated child scopes; join records which children satisfy
its policy. A reducer separately selects or combines declared results. Repeat and pinned
subworkflows create attached child runs with explicit input/output imports. Waits, signals,
timers, and continuation decisions survive restart. Terminals account for owned work before
closing a scope. The [structured driver](src/engine/structured.rs),
[child lifecycle](src/engine/structured/subworkflow.rs), and [completion path](src/engine/completion.rs)
connect these steps.

Workspace inputs resolve exact immutable value versions and provenance. Task context selection
adds earlier evidence requested by [`TaskContextPolicy`](../blueprint/src/context.rs).
The [durable candidate source](src/context/source.rs) discovers bounded metadata at the dispatch
projection's frozen sequence, using recent events and exact historical anchors. The
[selector](src/context.rs) ranks candidates and saves a restricted manifest before dispatch;
the host materializes only selected content and verifies it against that manifest. A retry rebinds
the previous selection to its new attempt rather than selecting from newer history.

Current policy enforcement has known gaps: blueprint session declarations are not checked against
model request sessions; stopping selection at the first optional overflow can skip later required
checks; omission-reason precedence can retain protected reference metadata. See `CausalContextBuilder`
for the affected choices. The manifest records the selection actually made, not proof that every
declared policy intent was enforced.

## Inspect, revise, and reopen

`RunProjection` folds accepted events into current obligations and terminal summaries. Settled
attempt detail retires from active memory but remains in the journal. Use the
[historical queries](src/query.rs) when an old attempt is no longer in the current frontier.
These reads bound memory; they can still scan substantial history. A verified snapshot accelerates
replay, while a missing or invalid optional snapshot falls back to the journal.

A new revision changes future work through [reconciliation](src/reconciliation.rs). Planning,
approval where needed, and application bind exact revisions and a run sequence. Started and
completed executions retain their governing revision. [Control](../control/README.md) handles
untrusted proposals and risk policy around this runtime operation.

For composition, open with `open_closed_with_authority`, recover active runs with
`recover_startup_closed`, finish application/host startup, then call `resume_admission`. Simpler
embedders can use the constructors that complete recovery immediately. The default evaluator
denies external commands; production supplies its evaluator explicitly. Startup validates active
obligations and discovery indexes, while a full historical scrub is a separate storage-admin
operation. `begin_shutdown` closes new admission; the host continues driving cancellation,
reports, and recovery while its workers drain. [Daemon operations](../../docs/operations/daemon.md)
owns the production procedure.

Controller policy enters through `ControllerLifecycle`; persistence owns its cumulative account.
The production lifecycle remains uninstalled pending the qualification in
[status](../../docs/product/status.md). Ordinary structured repeats use their own declared bounds.

The `test-support` feature exposes deterministic fixtures. Run
`cargo test -p milkdrift-runtime --all-features` for command, scheduling, structured execution,
context, recovery, and projection contracts. Follow the
[verification policy](../../docs/development/workflow.md#choose-verification-for-the-change)
for the checks appropriate to a change.
