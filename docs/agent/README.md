# Agent context

This directory packages the material that an engineering agent needs to reason consistently across sessions. It is an **operational organization boundary**, not a separate source of truth parallel to the rest of the documentation.

## Structure

```text
agent/
├── README.md
├── persona.md
├── knowledge/
│   └── rust_knowledge.md
├── decisions/
│   ├── README.md
│   └── NNNN-*.md
└── execution/
    ├── README.md
    ├── current.md
    ├── analyzer.md
    ├── execution-plan.md
    └── history.md
```

The contents have different lifecycles:

- `persona.md` is reusable collaboration guidance;
- `knowledge/` is reusable technical explanation and evidence-oriented guidance;
- `decisions/` is project-specific architectural rationale;
- `execution/current.md` is the mutable handoff for an active phase or an explicitly parked state;
- `execution/analyzer.md` and `execution-plan.md` are preserved source artifacts for the execution program and inactive future tracks;
- `execution/history.md` is chronological closure evidence.

## Recommended agent read path

For implementation work:

1. read [persona](persona.md);
2. read the repository [documentation map](../README.md);
3. apply the reusable [architecture blueprint](../architecture.md) and [engineering rules](../rules.md);
4. read the current [project architecture](../project/architecture.md) and [implementation status](../project/implementation-status.md);
5. load [current execution context](execution/current.md);
6. read the project guide and ADRs relevant to the task;
7. consult the exact execution-plan section or analyzer findings when needed.

This order gives the agent the reusable model first, then the applied project truth, then the dense working context.

## Duplication rule

Agent context may **reference** canonical project material freely. `execution/current.md` may intentionally restate a small amount of current information so an agent can begin work without reconstructing the phase from many documents, but it must name the canonical owner and be updated when that owner changes.

Do not copy full component guides, status matrices, or ADR rationale into execution files. Dense context should connect knowledge, not fork it.
