# Milkdrift agent constitution

This is the binding entry point for work in this repository. Detailed product and architecture
rules remain with their canonical owners; do not copy them into task notes or new overview files.

## Reading order

1. `AGENTS.md`
2. `docs/product/vision.md`
3. `docs/architecture.md`
4. `docs/product/status.md`
5. `docs/product/roadmap.md`
6. Relevant ADRs, references, source, and tests

Before planning a change, use the [practice guide](docs/development/practices/README.md) to select
and read the practices relevant to the actual work, even when the assignment omits their links.

## Source-of-truth map

| Question | Owner |
| --- | --- |
| Enduring product intent and non-negotiable semantics | `docs/product/vision.md` |
| Architecture, ownership, and dependency direction | `docs/architecture.md` |
| What works now and what remains limited | `docs/product/status.md` |
| Ordered unfinished product work | `docs/product/roadmap.md` |
| Durable design decisions and tradeoffs | `docs/decisions/` |
| Schema and protocol contracts | Owning code constants/readers, fixtures, then `docs/reference/` |
| Verified behavior and evidence | Tests, CI workflows, `docs/development/verification-evidence.md` |
| Development methods and review guidance | The relevant files in `docs/development/practices/` |
| Assignment scope, completion, and verification procedure | `docs/development/workflow.md` |
| Historical chronology | Git history, commits, CI runs, release notes, and external audits |

When prose and executable behavior disagree, establish whether code or documentation drifted; do
not silently choose the convenient side. Schema constants, readers, and golden fixtures are the
implementation evidence for current versions.

## Non-negotiable invariants

- Definitions are immutable, execution history is append-only, and control authority is explicit.
- Reconciliation is prospective. Never rewrite history to make a new decision look retroactive.
- Humans, AIs, services, and peers use the same scoped authority and command path.
- Models, tools, processes, and peers are external capabilities. The core owns no local inference.
- Uncertain effects remain uncertain until durable evidence or authorized reconciliation resolves
  them; never manufacture exactly-once claims.
- Context is causal, bounded, provenance-bearing, and branch-isolated unless an explicit semantic
  boundary exposes it.
- Queues, pages, streams, retries, artifacts, receipts, audits, and retained operational state are
  bounded with truthful overflow behavior.
- Idempotency binds exact canonical requests to durable results. Archival must preserve exact replay
  and conflict behavior.
- Every durable or executable fact has one owner. Views and adapters may project it, not duplicate
  authority over it.
- UI state never becomes workflow, authority, runtime, or persistence semantics.

## Required development behavior

- Inspect current source, consumers, tests, manifests, and Git context before planning a change.
- Complete an owned boundary end to end: implementation, refusal paths, tests, docs, and evidence.
- Prefer deletion, private modules, and narrow visibility. Do not add generic `common`, framework,
  registry, or abstraction layers without a proven multi-owner contract.
- Keep current facts in canonical docs. Temporary sprint plans, assignments, and progress notes
  belong in the [virtual office](docs/development/virtual-office/README.md); broader issues and
  discussions may persist across sprints on its whiteboard. Follow the
  [scope and findings policy](docs/development/workflow.md#findings-beyond-the-assignment).
  Elsewhere, do not add pass diaries, prompt histories, duplicated status pages, or generated inventories.
- Write documentation to explain purpose, use, and consequences in plain language. Follow the
  [documentation practice](docs/development/practices/documentation.md) for prose,
  code comments, and package READMEs; naming technical properties is not an explanation.
- Preserve the current scope freeze: no UI, new provider family, or new workflow primitive until an
  independently reviewed task explicitly authorizes it.
- Make no support, safety, portability, or interoperability claim that tests or evidence do not
  establish.
- Before completion, follow the [verification policy](docs/development/workflow.md#choose-verification-for-the-change).
  Executable changes require the full gate. Prose, planning, and documentation-only changes use
  the checks specified there; report the checks actually run and any limits on the evidence.

Use `docs/development/workflow.md` for assignment procedure, completion, focused suites, evidence
lanes, fixture rules, and public-API review. Individual practices own the detailed development rules.
