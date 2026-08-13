# Documentation

Milkdrift documentation is organized around one current-project spine. Reusable
engineering doctrine, current project behavior, durable decisions, and execution
memory have different owners so readers do not have to choose between competing
versions of the same fact.

## Authority spine

Follow this route for a complete view of the current project:

```text
README.md
  -> vision.md
  -> project/architecture.md
  -> project/operation.md
  -> project/implementation-status.md
  -> project/validation.md
  -> project/performance.md
  -> relevant component guide
  -> relevant ADR
```

The route is an authority map, not a requirement to read every page before every
edit. Validation and performance are needed when producing evidence; a component
guide and ADR are selected for the subsystem being changed.

## Ownership

| Question | Owner |
|---|---|
| Why does Milkdrift exist and where might it lead? | [Vision](vision.md) |
| Which reusable engineering principles apply? | [Architecture blueprint](architecture.md), [engineering rules](rules.md), and [documentation conventions](conventions.md) |
| How are the current crates and layers arranged? | [Project architecture](project/architecture.md) |
| How does one local execution run end to end? | [Operation guide](project/operation.md) |
| What is currently implemented, supported, and evidenced? | [Implementation status](project/implementation-status.md) |
| How is evidence reproduced? | [Validation](project/validation.md) |
| What measurements exist and what do they mean? | [Performance evidence](project/performance.md) |
| Why was an architectural choice made? | [ADRs](agent/decisions/README.md) |
| What is being worked on now? | [Current execution context](agent/execution/current.md) and [execution plan](agent/execution/execution-plan.md) |
| What was completed on an older tree? | [Execution history](agent/execution/history.md) |

`project/` contains current Milkdrift-specific reference. `agent/decisions/`
contains binding rationale until an ADR is superseded. `agent/execution/` contains
only immediate handoff, ordered program state, and a concise milestone ledger;
it is not a second project reference.

## Reading for implementation

Read [the repository overview](../README.md), then the project architecture,
operation guide, current status, one relevant component guide, and relevant ADR.
Read the vision when product intent matters. Read current execution context only
when an active maintenance or product package exists.

Current component and policy guides are indexed in the
[project documentation map](project/README.md). Do not load completed prompt text
or reconstruct current behavior from historical runs.

## Maintenance rule

Every changing fact has one owner. Other pages state only the local consequence
and link to that owner. Current behavior belongs in project reference; support and
accepted run state belong in implementation status; procedures belong in
validation; measured results belong in performance; rationale belongs in ADRs;
chronology belongs in history.
