# Documentation clarity sprint

Status: prepared; no rewrite assignment has been executed or accepted.

Make Milkdrift understandable to a contributor learning the code and an operator using the
applications. Replace compressed jargon with explanations of purpose, use, relationships, and
consequences. Add an introduction for every workspace package. Apply the
[documentation standard](../../engineering-rules.md#7-documentation) and the
[virtual-office procedure](../README.md) throughout.

## Scope and limits

This sprint covers maintained Markdown, package READMEs, Rust crate/module/API documentation, and
comments that explain non-obvious implementation or test decisions across the workspace. Existing
useful explanations should be retained. Audit scope is broad; each execution assignment is small.

Change explanations and supported documentation examples. Preserve executable behavior, public
names and visibility, dependencies, wire/schema data, fixtures, test assertions, and qualification
claims. No UI, new provider, workflow primitive, architecture cleanup, or product feature is
authorized. Do not add generic documentation tooling or weaken checks to make prose pass.
Source/doc disagreements require investigation and a recorded finding, not a convenient rewrite.
Use the [scope and findings policy](../../engineering-rules.md#findings-beyond-the-assignment)
for necessary corrections and broader discoveries.

The initial inspection found 22 workspace packages with no package READMEs. The examples supplied
by the user are in `crates/blueprint/src/context.rs`, `crates/model/src/context.rs`, and
`crates/model/src/task.rs`. Recheck the working tree and manifests before assigning work; these
are starting observations, not evidence of completed coverage.

## Phase order

| Phase | Outcome | Prerequisite |
| --- | --- | --- |
| [01: Context example](01-context-example.md) | One reviewed example of useful policy, schema, and limit documentation. | An explicit assignment. |
| [02: Foundation packages](02-foundation-packages.md) | Introductions and source explanations for six definition/contract packages. | 01 accepted. |
| [03: Execution packages](03-execution-packages.md) | Explain how five packages execute and retain work. | 01 accepted; coordinate related explanations. |
| [04: Integrations and applications](04-integrations-and-applications.md) | Explain the eleven remaining packages and supported uses. | 01 accepted; coordinate related explanations. |
| [05: Maintained prose](05-maintained-prose.md) | Clear product, architecture, contributor, operator, and reference documents. | 01 accepted; use reviewed source explanations where relevant. |
| [06: Review and close](06-review-and-close.md) | Reader review, verified examples, integrated evidence, and sprint removal. | Coverage from 01–05 accepted. |

After the initial example is reviewed, phases 02–05 are work areas rather than a mandatory serial
queue. Assign a coherent reader outcome, such as explaining one package's main operation or one
operator procedure. Include the related introductions, API docs, examples, and links needed to
complete that explanation. Record likely files for coordination and update them as needed;
there is no file-count limit. Split large areas by responsibility and real dependencies. Each
worker stops at the assigned outcome; the coordinator handles further work within the user's request.

## Coverage to assign

This is a temporary work allocation, not a replacement architecture map. Keep each row's reviewed
scope explicit, including areas inspected and retained as already useful. Link large generated
inventories under `target/` rather than pasting them here. A README alone does not complete a row.

| Phase | Package or document area | State | Reviewed scope / next portion |
| --- | --- | --- | --- |
| 01 | Context example in blueprint/model | queued | Named symbols in phase 01; not the whole packages. |
| 02 | `crates/contracts` | queued | — |
| 02 | `crates/capability` | queued | — |
| 02 | `crates/blueprint` | queued | Include remaining APIs after phase 01. |
| 02 | `crates/workspace` | queued | — |
| 02 | `crates/authority` | queued | — |
| 02 | `crates/model` | queued | Include remaining APIs after phase 01. |
| 03 | `crates/persistence` | queued | — |
| 03 | `crates/runtime` | queued | Split context, scheduling, reporting, recovery, and structured work. |
| 03 | `crates/capability-host` | queued | — |
| 03 | `crates/control` | queued | — |
| 03 | `crates/prompt-sequence` | queued | — |
| 04 | `crates/control-protocol` | queued | — |
| 04 | `crates/control-client` | queued | — |
| 04 | `crates/peer-protocol` | queued | — |
| 04 | `adapters/local-process` | queued | — |
| 04 | `adapters/model-provider` | queued | — |
| 04 | `adapters/local-secret` | queued | — |
| 04 | `adapters/peer-http` | queued | — |
| 04 | `adapters/redb-store` | queued | — |
| 04 | `apps/daemon` | queued | — |
| 04 | `apps/cli` | queued | — |
| 04 | `tools/evidence` | queued | — |
| 05 | Root entry docs and `docs/README.md` | queued | — |
| 05 | `docs/product/` and `docs/architecture.md` | queued | — |
| 05 | `docs/development/` excluding active sprint notes and whiteboard topic files | queued | Review process documentation too; preserve contributors' positions and evidence in topic files. |
| 05 | `docs/guides/` and `docs/operations/` | queued | — |
| 05 | `docs/reference/` prose | queued | Schema bytes remain unchanged. |
| 05 | `docs/decisions/` | queued | Preserve decision history; clarify explanations. |
| 05 | `examples/` prose and remaining maintained comments | queued | Include `.github/` comments; no workflow or example-data changes. |

## Current assignment and handoff

To begin, give an agent [phase 01](01-context-example.md) with the instruction to execute its
named scope. That request assigns the receiving agent as worker; the coordinator records the
assignment and arranges review. No additional scope design is needed for that first portion.
For later phases, select the next incomplete package/document row and name the limited portion
in the assignment. Preparing these prompts does not itself dispatch any rewrite work.

Before execution, the coordinator reviews the whiteboard using its
[preparation prompt](../whiteboard/prepare-sprint.md). No rewrite assignment is active. The
coordinator records assignments in prose or using these prompts, deriving routine details from
the user's request rather than waiting for a completed form:

- Assignment ID, phase, owner, and reviewer:
- Responsibility and expected reader outcome; likely files and explicit exclusions:
- Prerequisite accepted result and source/test references:
- Checks under the verification policy, integration responsibility, and stop condition:
- State, reviewed portion, and next permitted action:
- Current handoff: changed explanation; checks with tree identity/log reference; unresolved findings:

Use one record per active assignment when parallel work is explicitly assigned. The coordinator
updates coverage from accepted handoffs. Keep assignment-specific findings here with location,
problem, evidence, owner, and next action; remove resolved findings after recording accepted
coverage. Link broader issues and discussions to their whiteboard topic; assigned execution
progress belongs here, with board planning fields kept in its overview.

## Completion criteria

- Every current workspace package has a useful README and an appropriate crate entry explanation.
- Every coverage row has been reviewed, including public APIs and non-obvious private/test comments.
  Necessary terminology is explained; mechanically expanded comments do not count as improvement.
- A reader can follow construction, a normal use, and a meaningful failure for each package's
  main operation using the docs. Definitions and runtime records are clearly distinguished.
- Examples and local links pass the required checks. Source comments describe behavior supported
  by implementation and tests. No runtime, API, format, fixture, or dependency change is hidden here.
- The final reviewer has checked cross-package explanations and raised no unresolved blocking
  findings. Any explicitly accepted deferral remains identified as unfinished work with an owner.
- The checks required by the [verification policy](../../workflow.md#choose-verification-for-the-change)
  pass for the integrated changes; useful outcomes are in their maintained owners;
  temporary sprint files and links are removed under phase 06, retaining the whiteboard.
