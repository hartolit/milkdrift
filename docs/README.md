# Documentation

This directory is organized as a modular knowledge system. Reusable engineering doctrine, agent-operational context, and current project truth are separated so each kind of information has a stable owner without forcing readers to search one giant manual.

The objective is not minimum text. It is **high-signal context with one authoritative home for changing facts**.

## Structure

```text
docs/
├── README.md              documentation map and authority
├── conventions.md         reusable documentation conventions
├── architecture.md        reusable/selectable architecture blueprint
├── rules.md               reusable engineering rules
├── vision.md              non-normative project motivation and long-term research direction
├── agent/                  agent-facing context and project execution memory
│   ├── README.md           how the agent context is organized
│   ├── persona.md          reusable collaboration/system definition
│   ├── knowledge/          reusable technical knowledge
│   ├── decisions/          project architecture decision records
│   └── execution/          current context, analysis, plan, and history
└── project/                current project-specific reference material
```

The folder boundary and the authority boundary are related but not identical. `agent/` means "material intentionally organized for agent consumption and handoff"; it does **not** mean everything inside is merely advisory. Accepted ADRs remain project decisions. Conversely, `agent/execution/current.md` is deliberately a derived working set and is not the canonical owner of the facts it gathers.

See [documentation conventions](conventions.md) for naming, ownership, maintenance, and the intentional role of dense current execution context.

## Information layers

### Reusable doctrine

- [Architecture blueprint](architecture.md) teaches how to choose and reason about modular workspace topology.
- [Engineering rules](rules.md) defines reusable implementation and review defaults.
- [Documentation conventions](conventions.md) defines how repository knowledge is structured and maintained.
- [Agent persona](agent/persona.md) and [knowledge notes](agent/knowledge/rust_knowledge.md) are reusable agent inputs.

These files should not accumulate project support state, concrete crate inventories, temporary phase constraints, or validation claims.

### Project vision

[Project vision](vision.md) records why llm-app exists and the longer-term ideas the
current product is intended to explore: clean context, composable workflows,
navigable memory, local/peer/hosted execution, multiple frontends, trust, and deeper
system integration. It is deliberately exploratory rather than normative.

Vision may motivate experiments and future tracks, but it does not override accepted
ADRs, applied project architecture, current status, or an active phase specification.

### Project decisions and execution memory

- [Architecture decisions](agent/decisions/README.md) record why important project choices were made and when they should be revisited.
- [Current execution context](agent/execution/current.md) is the dense handoff for the phase being worked now.
- [Execution plan](agent/execution/execution-plan.md) owns the ordered roadmap and phase gates.
- [Architecture analysis](agent/execution/analyzer.md) preserves the analysis that motivated that program.
- [Execution history](agent/execution/history.md) preserves completed-phase evidence and measurements.

`current.md` is intentionally mutable and compact enough to read before implementation. It may repeat selected current facts for operational clarity, but every repeated fact should point back to the document that owns it.

### Current project reference

[Project documentation](project/README.md) owns the applied architecture, exact workspace structure, runtime/backend behavior, current support, validation procedures, portability claims, and other llm-app-specific reference material.

Reference documents describe how the system works **now**. Execution documents describe what is being done **next** or what was proven **then**.

## Authority and evidence

Documentation carries different kinds of authority:

1. **Reusable policy** — architecture and engineering rules define general invariants/defaults.
2. **Accepted project decisions** — ADRs record binding project choices until amended or superseded.
3. **Applied project architecture** — maps reusable principles and ADRs onto the current workspace.
4. **Current status** — records what the present source tree claims to support and the validation provenance for that claim.
5. **Domain reference** — component, backend, lifecycle, policy, and runbook documents own detail for one area.
6. **Execution context** — `current.md`, the plan, analysis, and history guide work but do not replace current project reference.
7. **Knowledge notes** — explain reusable engineering mechanisms, tradeoffs, and evidence.

Code, tests, generated metadata, and reproducible commands remain the evidence for implemented behavior. If an accepted decision, applied architecture, current reference, and executable behavior disagree, treat the disagreement as a defect and reconcile it explicitly rather than choosing whichever source is convenient.

## Reading routes

### Starting an implementation task

Read in this order unless the task clearly needs less context:

1. [agent persona](agent/persona.md);
2. this documentation map;
3. [architecture blueprint](architecture.md) and [engineering rules](rules.md);
4. [project architecture](project/architecture.md) and [implementation status](project/implementation-status.md);
5. [current execution context](agent/execution/current.md);
6. the relevant project domain guide and ADRs;
7. the exact phase section of the execution plan when execution sequencing matters.

Use the full analyzer when architectural rationale or previously identified risks are relevant; it should not be necessary for every local edit.

### Looking up domain knowledge

Start at [project/README.md](project/README.md) for current system behavior, [agent/decisions/README.md](agent/decisions/README.md) for decision rationale, and `agent/knowledge/` for reusable technical explanation.
Use [project vision](vision.md) when evaluating long-term direction or whether a proposed capability serves the larger system; do not use it as evidence that a capability is implemented or architecturally committed.

## Maintenance rule

Do not create a second canonical page merely because an existing page is long. Split when the new document has a stable independent owner or search domain, then index it locally.

Preserve rationale, failure semantics, measurements, and constraints that teach the system. Remove duplication by assigning ownership—not by deleting context until every file is a summary.

Closed phases append to execution history. The active phase updates `agent/execution/current.md`. Current project behavior updates the relevant project guide and status page.
