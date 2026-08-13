# Agent context

This directory contains reusable agent guidance, durable project decisions, and
compact execution handoff. It is an organizational boundary, not a parallel source
of project truth.

```text
agent/
├── persona.md
├── knowledge/
├── decisions/
└── execution/
    ├── README.md
    ├── current.md
    ├── execution-plan.md
    ├── history.md
    └── archive/README.md
```

For implementation, start at the repository [documentation map](../README.md),
then read the [project architecture](../project/architecture.md),
[operation guide](../project/operation.md), [implementation status](../project/implementation-status.md),
one relevant component guide, and the relevant [ADRs](decisions/README.md).
Consult [current execution context](execution/current.md) only for immediate
handoff and [the execution plan](execution/execution-plan.md) for ordering.

Execution pages may link to current authority but must not copy component guides,
support matrices, accepted-run tables, or ADR rationale.
