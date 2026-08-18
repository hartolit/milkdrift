# ADR 0005: Prospective immutable revision reconciliation

- Status: accepted
- Date: 2026-08-18

## Context

Milkdrift permits workflow editing while a run exists. A new immutable revision can add, remove, or change nodes and dependencies while old executions are pending, active, completed, cancelled, or uncertain. Applying a graph diff without runtime history would silently reinterpret completed work or repeat side effects.

## Decision

Revision adoption is a command-driven, three-stage process: plan, decide where required, then apply. The pure planner compares exact immutable old and requested revisions against the projection at one exact run sequence. It uses stable node identities plus domain-separated configuration and dependency fingerprints and considers execution state, descendants, structured scopes, and side-effect history.

The persisted typed plan classifies unchanged completed and active work, changed active and pending work, added work, never-started removed work, completed or uncertain side-effect work, dependency changes affecting started descendants, incompatible interfaces or pinned subworkflows, and work requiring authority. Each item records a prospective policy: finish current and use the new definition next time, cancel and restart when safe, retain and remediate/compensate, remove only never-started independent work, require a decision, or reject retrospective rewriting.

Plan creation, decisions, and application are idempotent. A plan names the exact old revision, new revision, and sequence used for planning. Application verifies that the run and revision lineage have not moved except for the plan's own decision facts. Immediate prospective actions are themselves durable facts: removing an eligible execution, requesting cancellation of an active attempt, or creating remediation work is recorded before the plan is marked applied. Adoption then appends a new pin from a recorded point forward. It never deletes, edits, or reinterprets an earlier event.

## Rejected alternatives

- Mutating the revision pinned by old events, because that rewrites provenance.
- Applying textual or positional graph diffs, because node identity and semantic configuration are the durable comparison keys.
- Automatically deleting removed work regardless of state, because completed dependants and effects remain true.
- Replanning silently at apply time, because approval would no longer describe the applied change.

## Consequences

Compatible future work can move to a new revision while historical work remains attached to its original definition. Stale plans fail explicitly, and ambiguous edits can require human/controller authority. Planning stores more metadata, but decisions remain reproducible and inspectable.

## Reconsideration triggers

Additional classifications or policies may be added with versioned documents as new structured semantics appear. The prospective-only and stale-safe rules are permanent unless another design proves it cannot rewrite completed history.
