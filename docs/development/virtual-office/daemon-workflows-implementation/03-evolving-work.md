# 03 — Evolving tasks, parallel research and correction

**Completed locally October 17; see the [handoff](handoff.md) for tested checkpoints and evidence.**
The user explicitly authorized proceeding with 03 while the actual early
user review in 02 remains pending as an evidence gap. Extend the same
production task engine, public protocol, CLI and Svelte. Deliver the planning → parallel research →
composition → corrective work portion of [the journey](journey.md), including failed research.

Read [AGENTS](../../../../AGENTS.md), canonical docs, [implementation](../../practices/implementation.md),
[documentation](../../practices/documentation.md), [workflow](../../workflow.md),
[README](README.md), [ADR 0051](../../../decisions/0051-daemon-task-workflows.md),
[code-change map](code-change-map.md) and the actual 02 handoff. Inspect relevant source/callers and
checks. Do not redesign the chosen task representation or add a second controller/scheduler.

## Complete the automatic path

A planning task consumes the user's brief and returns a bounded structured batch containing three
research tasks and a composition task with local aliases, declared result locations and explicit
input/dependency rules. The control owner admits that batch with the same typed operations used by
human future edits. The daemon allocates stable IDs and schedules ready tasks; the fixture, CLI and
Svelte do not create children on the model's behalf.

Implement response persistence and recoverable admission exactly as the ADR specifies: immutable
model response, pending admission, generation checks, atomic accepted delta/receipt/alias map and
restart-safe processing. No result-to-next-work gap may lose or duplicate additions. Reject invalid
schema, dangling/forward-invalid references, cycles, unauthorized paths/tools/models, overlarge
batches or exhausted shared allowance without partial changes. An explicitly granted automatic
feedback task handles invalid/conflicted output using bounded diagnostics and a refreshed snapshot;
otherwise expose a waiting decision. Human approval grants no hidden capability expansion.

Finish required-success, optional and settled-outcome inputs through backend and UI. Research tasks
write into separate scoped project folders and publish immutable output refs. The compositor receives
selected successful outputs and one explicit failed outcome. It can create corrected research and
new work automatically. Required-success consumers remain blocked until an authorized future edit
rebinds them to a successful correction. Instructions, timestamps and screen order never schedule.

Provide complete ordinary model/tool execution: selected configured model, allowed tool catalog,
bounded model requests and tool-call rounds, real file read/write/process tools, exact outputs and
usage. Keep this loop in capability-host under existing authority/final-entry/reporting; only runtime
schedules tasks. Do not pretend generic file content is supported by provider mappings; captured
text content has exact bytes and bounded tool feedback. Existing external coding-agent process
profiles remain valid ordinary task capabilities. No provider family or inference implementation.

Implement cumulative workflow limits, explicit finish/wait, failed-lineage/no-progress stops and
prospective human resume/limit change. Budget and wall-clock policy survives restart and all descendants.
Resume does not clear spending or uncertainty. A finished workflow requires the explicit completion
condition, not an empty runnable queue. Review is an ordinary task whose admitted batch links new
correction to the earlier task; historical results remain unchanged.

Support concurrent human/model editing in Svelte and CLI: readable pending task inputs/dependencies,
editing and previewing a complete future batch, version conflict refresh, explicit approval when
needed, missing-input supply, correction/rebinding, stop/replacement, and limits/status inspection.
Show real parallel lanes, frozen started input inspectors, pending waits and the cause of follow-up
work. Complete operator and protocol documentation in the same changes.

## Evidence and checkpoints

Exercise required versus optional tasks and typed adopt-correction, including output compatibility,
adoption cycle/competing-adoption refusal and started consumers keeping their frozen inputs. The
failed research task remains failed while its successful adopted correction satisfies completion.
Test native path claims with disjoint folders, same path, parent/child overlap, capture racing a writer
and an uncertain writer after restart. Current managed claims alone do not cover these native roots.
Fill ordinary capacity while work is active, then settle/request stop using its reserved records.

Use existing runtime/control/persistence/account tests as the starting owners, adding new task-focused
cases. Build affected callers, then run focused packages and integration targets:

```sh
cargo test -p milkdrift-blueprint -p milkdrift-control -p milkdrift-authority --all-features
cargo test -p milkdrift-runtime --all-features
cargo test -p milkdrift-persistence -p milkdrift-redb-store --all-features
cargo test -p milkdrift-capability-host -p milkdrift-model-provider --all-features
cargo test -p milkdrift-control-protocol -p milkdrift-control-client -p milkdrift-cli --all-features
cargo test -p milkdrift-daemon --test control_plane --all-features
cargo test -p milkdrift-evidence --test application --all-features
```

Run frontend check/lint/unit/build and focused browser tests; verify discovery of new test cases.
Add fault injection before/after response save, admission acceptance and child entry. Assert exactly
one accepted child batch after restart/lost reply. Race human edits with model additions and with
start. Reject cycles, out-of-grant requests and malicious result text without effects. Exercise failure
input policy, authority revocation during a tool loop, cumulative accounting across corrections,
no-progress/lineage limits, exhausted budgets, stop-requested uncertainty and explicit finish/wait.

Run the animation trace through production browser and binaries with controlled model/tool responses.
The harness may fail one research call and delay another to prove overlap, but may not create tasks,
issue continuation batches or order daemon work. Observe actual independent start/end evidence.
If a configured authorized real model is available, retain its quality/failure separately; neither
fabricate model success nor use human-written children as proof of automatic construction.

Commit coherent tested boundaries: shared batch admission and fault recovery; parallel inputs/tool
execution and budgets; human/model repair and complete UI/evidence. Fix each discovered in-scope defect
now. Update affected owners' READMEs, protocol/schema versions, architecture/status and actual evidence;
review changed public APIs and run documentation/diff checks. Handoff the tested commit and evidence
scope ready for 04. Prompt 07 alone owns the integrated full gate; do not start it or 04 automatically.
