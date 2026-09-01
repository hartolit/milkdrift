# ADR 0026: Controllers use one durable bounded lifecycle

- Status: accepted for the library contract; production-daemon installation superseded by ADR 0027
- Date: 2026-08-31

## Context

The control crate already described bounded controllers and exposed a pure `ControllerLimits`
evaluator, but production scheduling did not call it. Limits embedded only as descriptive metadata
could reset on restart, omit resource dimensions, or disagree with repeat's native budgets. Adding
a controller scheduler or privileged AI node would duplicate runtime ownership and bypass the
ordinary authority, proposal, and reconciliation path.

## Decision

Controller policy is strict schema-1 semantic revision data under
`org.milkdrift/controller-policy`. Its domain-separated `cp1_` digest binds the exact controller
identity, wrapper node/workflow, pinned body, cumulative limits, checkpoint and stop behavior,
missing-usage policy, currency, required control operations, and bounded provenance. The one
supported stop behavior is deterministic controller failure. Unknown versions, legacy
metadata-only patterns, digest mismatches, and wrapper/repeat/body contradictions fail closed.

`milkdrift-control::ControllerLifecycleOwner` is the sole parser, progress accountant, and caller
of `ControllerLimits::assess`. An embedding installs it into the existing deterministic runtime only
while admission is closed. Runtime supplies host-owned run/revision/node/execution/time/projection facts
at activation, cycle entry, and checkpoint continuation. The owner never accepts model counters or
authority claims. Runtime records `ControllerAssessmentRecorded` and creates an allowed iteration
in one atomic transition.

Progress comes from authoritative projections: terminal repeat children count cycles/failures;
exact resolved descriptor categories count process/model invocations; attempt usage supplies units
and currency cost; child artifact metadata supplies logical bytes; attributed revision-adoption
requests and rejections maintain monotone run-actor counters through compaction; the first
assessment/execution boundary supplies elapsed time; and immutable reachable body revisions supply
repeat/child depth plus conservative potential model/process entry. Missing metering on an admitted
model/process attempt fails closed. New capability-resolution snapshots bind exact category; legacy
schema-1 snapshots without it retain their original digest and count conservatively as both process
and model rather than becoming a bypass. Duplicate commands and observations cannot add another
accepted fact.

The policy owns cumulative ceilings. Repeat retains a one-iteration-higher structural maximum and
does not duplicate controller time/cost budgets. Proposal mutation/node dimensions are evaluated on
the validated proposal before revision storage. Approval/application of controller-authored
revisions reassess cumulative policy. Exact-interval checkpoints reuse durable repeat-continuation
requests and ordinary authorized decisions; approval rechecks revocation and policy before another
cycle. A controller cannot change its own policy. A stronger actor may propose a new immutable
policy revision, but it affects only work prospectively reconciled to that revision.

## Rejected alternatives

- Keep limits as prose or an opaque unvalidated metadata blob, because neither is executable or
  safe across restart.
- Add a daemon polling loop or autonomous node kind, because that would be a second scheduler and a
  privileged AI path.
- Trust counters or a “done” claim from model output, because those are untrusted proposal data.
- Map every ceiling into `RepeatBudget`, because repeat cannot own proposal, category, artifact,
  failure, rejection, or structure accounting and duplicate ownership can disagree.
- Treat missing cost/unit observations as zero, because a hard resource ceiling would become
  bypassable.
- Mutate limits on an active run, because that would rewrite the meaning of accepted history.

## Consequences

Controller assessment is rebuildable from journal and immutable revision facts; compact snapshots
retain only monotone summaries and the current assessment/frontier. Stable framed hashes identify
cycles, assessments, and checkpoints. Restart cannot reset progress, duplicate a body, approve a
checkpoint, or resume a reached bound. Human and AI callers continue through the same grants,
commands, proposal documents, approvals, revisions, and reconciliation plans.

One controller policy document exists per immutable revision. Multiple logical occurrences are
independent by execution identity. Controller-authored proposals are rejected when multiple active
occurrences make proposer attribution ambiguous. Paused-at-bound behavior and unsafe reinterpretation
of legacy controller metadata are deliberately unsupported. ADR 0027 records why the production
daemon leaves the lifecycle uninstalled until cumulative resources are reserved at final entry.

## Reconsideration triggers

Add another stop behavior only with a typed policy version and an existing durable pause/wait
owner. Support multiple simultaneously proposing controller occurrences only after proposal
provenance names one exact controller execution. Change accounting sources only with an additive
durable fact and replay/compaction/restart proof; UI, provider choice, or model quality alone is not
a reason to split lifecycle ownership.
