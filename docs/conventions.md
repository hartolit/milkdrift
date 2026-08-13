# Documentation conventions

These conventions define a reusable documentation style for engineering repositories. They optimize for clarity, modularity, searchability, and durable context rather than minimum line count.

## Principles

### One fact, one owner

Every fact that changes over time should have one canonical owner. Other documents link to that owner and add only context specific to their own domain.

Examples:

- current product support belongs in the status document;
- a component's lifecycle belongs in that component guide;
- a project-wide architectural decision belongs in an ADR or project architecture;
- a validation procedure belongs in a runbook or validation document;
- a recorded historical result belongs in execution history.

Duplication is justified only when the repeated text is itself a local invariant required to understand the document. Copying status tables, roadmap language, validation commands, or architecture summaries into several files is not justified.

### Preserve rationale, not repetition

Lean documentation is not sparse documentation. Keep information that explains:

- why a boundary exists;
- ownership and lifecycle semantics;
- failure and rollback behavior;
- compatibility constraints;
- rejected alternatives and tradeoffs;
- measurement scope and environment;
- exact commands required to reproduce evidence.

Remove or relocate material when it merely restates another document, narrates old edits, or mixes future plans into an evergreen reference page.

### Separate current truth from history

Evergreen reference documents use present tense and describe the current design. Historical evidence belongs in ADRs or execution history. Current status may mention the active phase, but component guides should not require readers to reconstruct behavior from phase numbers.

### Organize by purpose and domain

Prefer stable domains over chronology. A backend guide remains the backend guide across phases; a new phase should update it rather than create another backend summary.

## Document classes

| Class | Location | Purpose |
|---|---|---|
| Reusable definition | `docs/*.md` | Architecture blueprint, engineering rules, and documentation conventions |
| Agent definition | `docs/agent/persona.md` | Reusable collaboration and reasoning guidance for engineering agents |
| Knowledge | `docs/agent/knowledge/` | Reusable technical principles, evidence, and tradeoffs |
| Decision | `docs/agent/decisions/` | Project-specific architectural decisions and rationale |
| Execution | `docs/agent/execution/` | Compact current context, active/inactive plan, and milestone history |
| Project reference | `docs/project/` | Current project architecture, status, components, policies, and runbooks |

Do not put project-specific backend names, crate lists, phase state, or product support in reusable definition or knowledge documents.

`docs/agent/` is an operational organization boundary, not an authority tier. Reusable persona/knowledge and project-specific decisions/execution live together there because they are common agent inputs, but their canonical roles remain distinct.

## File organization

Use lowercase kebab-case filenames. ADRs use `NNNN-short-decision-name.md`. Avoid all-caps phase-report filenames, date-stamped copies, `final-v2` variants, and parallel pages with indistinguishable authority.

A directory should have an index when readers need to choose among several document roles. The index should explain ownership, not summarize every child document in detail.

Split a document when the new file has a stable independent owner or search domain. Do not split merely because a file is long.

## Recommended structures

These are defaults, not mandatory templates.

### Project reference

```text
# Domain name

Short purpose and scope.

## Responsibility / boundary
## Current behavior
## Important invariants or failure semantics
## Interfaces or data flow, when useful
## Evidence or limitations, when domain-specific
## Related documents
```

Omit sections that add no value. Do not add an empty "Overview" section only to satisfy a template.

### ADR

```text
# ADR-NNNN: Decision

- Status
- Date

## Context
## Decision
## Rejected alternatives
## Consequences
## Review trigger
```

A superseded ADR remains available as history and points to its replacement.

### Runbook or validation procedure

```text
# Procedure name

## Purpose
## Preconditions / pinned inputs
## Procedure
## Expected result
## Failure classification
## Evidence to record
```

Commands should be copy/pasteable. State whether they are canonical gates, focused diagnostics, or optional observations.

### Current execution context

`docs/agent/execution/current.md` is a compact immediate handoff. Its purpose is to
tell a new execution agent what is active—or that execution is parked—without
reconstructing the program from the entire repository.

It should contain:

- the reviewed baseline and gate state;
- the active phase objective, or an explicit statement that no phase is active;
- unresolved acceptance steps;
- environment-specific facts needed for the next agent;
- links to the canonical documents that own repeated facts.

This is **derived context**. It may deliberately repeat a small amount of current information for operational clarity, but it must not become the only owner of support state, architecture, component behavior, or validation procedure.

Update it while a phase is active. When the phase closes, move durable evidence to execution history, update canonical project references, and either advance `current.md` to a reviewed successor or park it explicitly with no active phase.

### Execution history

Use one chronological history document. Each milestone entry contains only:

- date and baseline;
- durable outcome;
- accepted local, hosted, or hardware run identity where useful;
- important evidence gaps at closure; and
- links to current behavior and preserved measurements.

Do not copy command transcripts, full support matrices, or measurements already
owned by current reference into history.

### Knowledge note

Explain the problem, principles, tradeoffs, and evidence. Keep project-specific conclusions out of reusable knowledge unless they are clearly labelled examples.

## Markdown style

- Use exactly one H1 title.
- Use sentence-case headings.
- Prefer short paragraphs for reasoning and bullets for independent facts.
- Use tables for matrices and comparisons, not for prose that reads better as paragraphs.
- Use fenced code blocks for commands, schemas, and diagrams.
- Name the language for code fences when it materially improves rendering; `text` is appropriate for conceptual diagrams.
- Use relative repository links for internal documentation.
- Link the first useful reference instead of repeating the same link after every paragraph.
- Avoid decorative emphasis, repeated callouts, and headings that only restate the filename.
- Avoid arbitrary line-count or word-count targets.

## Status, plans, and phase language

The canonical status document owns support matrices, active execution position, known limitations, and validation provenance. Component guides describe current behavior and link to status for product-level support.

Execution plans own future work. Evergreen project reference should not say "Phase N will..." unless the phase relationship is itself necessary context; prefer a link to the active plan.

Historical phase names are appropriate in execution history because chronology is the subject of that document.

The active phase name—or an explicit no-active-phase statement—is appropriate in `execution/current.md` because immediate execution state is that document’s subject. Keep detailed roadmap sequencing in the plan and durable current behavior in project reference.

## Validation and measurements

Separate procedures from evidence:

- a validation/runbook document explains how to run a check;
- the status document records whether the current source baseline has passed the required check;
- execution history preserves old run results and measurements;
- performance documents own benchmark methodology and domain-specific baselines.

Never present a measurement without enough scope to interpret it. Record the relevant source revision, toolchain/environment when material, and whether evidence is local, CI-backed, deterministic, or observational.

## Updating documentation with code

When a change alters behavior:

1. update the domain guide that owns the behavior;
2. update project architecture or an ADR if a boundary or decision changed;
3. update status if support, limitations, or validation state changed;
4. update validation/runbooks if commands or expected results changed;
5. update `agent/execution/current.md` when the active phase's working context changes materially;
6. append execution history only when closing a meaningful execution milestone.

Do not update unrelated documents merely to repeat the same change summary.

## Exceptions

Definition documents such as architecture, rules, and persona may use structures optimized for definitions rather than the project-reference template. Knowledge notes may use whatever structure best explains the topic.

Completed prompt and analysis bodies belong in Git history rather than a tracked
archive. Retain a small provenance index only when it helps locate durable outcomes.

## Review checklist

Before merging documentation changes, verify that:

- each changing fact has one clear owner;
- reusable documents contain no accidental project-specific state;
- current reference and historical evidence are separated;
- rationale, invariants, failure semantics, and reproducible evidence were not removed merely to shorten a file;
- links point to canonical owners instead of duplicated summaries;
- filenames and folders communicate document role;
- internal Markdown links resolve;
- commands and support claims match the source baseline they describe.
