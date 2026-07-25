# Documentation

This directory separates reusable engineering guidance from project-specific truth and execution history. The goal is modular documentation: one authoritative home for each fact, with links instead of parallel copies.

## Structure

```text
docs/
├── README.md          documentation model and authority
├── conventions.md     reusable documentation conventions
├── architecture.md    reusable architecture principles
├── rules.md           reusable engineering rules
├── persona.md         reusable agent/collaboration definition
├── knowledge/         reusable technical knowledge
├── decisions/         project architecture decision records
├── project/           current project-specific reference material
└── execution/         active execution inputs and chronological history
```

Top-level definition documents and `knowledge/` are intentionally project-agnostic so they can be reused. Project names, concrete crate graphs, product support, validation state, backend details, and runbooks belong under `project/`. ADRs and execution material are project-specific by nature.

See [Documentation conventions](conventions.md) for naming, structure, ownership, and maintenance rules.

## Authority and evidence

Documentation describes intent and context; code, tests, generated metadata, and reproducible commands are evidence of what is actually implemented.

Use these roles when documents overlap:

1. **Reusable policy:** [architecture](architecture.md) and [engineering rules](rules.md) define general invariants and defaults.
2. **Project decisions:** accepted ADRs record why the project chose a specific design and when it should be revisited.
3. **Project architecture:** `project/architecture.md` applies the reusable policy to the current project.
4. **Current status:** the project status page records what the current source tree supports and what evidence exists.
5. **Domain reference:** component, backend, lifecycle, policy, and runbook documents own detail for one domain.
6. **Execution:** plans and analyses guide future work; the history file records completed-phase evidence. They are not substitutes for current project reference material.
7. **Knowledge:** reusable notes explain engineering principles and tradeoffs; they do not override project policy.

A conflict between an accepted ADR, normative policy, current project architecture, and executable behavior is a documentation or implementation defect. Reconcile it explicitly rather than relying on an informal precedence loophole.

## Entry points

- [Documentation conventions](conventions.md)
- [Architecture principles](architecture.md)
- [Engineering rules](rules.md)
- [Agent persona](persona.md)
- [Project documentation](project/README.md)
- [Execution documentation](execution/README.md)
- [Rust systems knowledge](knowledge/rust_knowledge.md)

## Maintenance rule

Do not create a second canonical page because an existing page is long. Split by ownership or domain, then link the new module from its local index. Historical execution results are appended to the execution history rather than accumulated as standalone phase-report files.
