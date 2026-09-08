# Documentation clarity sprint

Status: phase 01 executed and ready for review; no rewrite result has been accepted.

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
| 01 | Context example in blueprint/model | ready for review | Policy construction/accessors, task attachment, manifest version, output limit, and both package introductions. See the handoff below; remaining APIs are still queued for phase 02. |
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
[preparation prompt](../whiteboard/prepare-sprint.md). The
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

### Phase 01 assignment

Assigned on 2026-09-08 by the user's request to execute [phase 01](01-context-example.md).
Codex is the worker and coordinator for this assignment. Reviewer: to be assigned after handoff;
this worker will not mark its own result independently accepted. At assignment, the whiteboard
overview was empty.

The reader should be able to construct a task context policy, attach it to a task, follow its
application into an attempt manifest, and explain a model output-limit refusal. Likely edits are
`crates/blueprint/src/context.rs`, `crates/blueprint/src/model/node.rs`,
`crates/model/src/context.rs`, `crates/model/src/task.rs`, and both packages' `src/lib.rs` and
`README.md`. Related explanation corrections on this path are included; executable behavior,
API names/visibility, dependencies, fixture bytes, and qualification claims are excluded.

Trace constructors/readers, runtime discovery and selection, host materialization, provider
mappings, ADRs 0011/0012, and the kernel/contracts/causal-context tests before writing the
explanations. Codex owns the Cargo jobs and integrated diff review. Required checks are those
three owner suites, both package doctests, formatting, warning-denying rustdoc, documentation
contracts, rendered API pages, and README links. Stop at a reviewable handoff with source/test
evidence and unresolved findings; do not begin phase 02 or accept broader coverage.

### Phase 01 handoff

Codex completed the assigned documentation edits on 2026-09-08 against base commit
`875b564d6bc56172c5dcc8627ec9ca4d0d6c6fc3`. The result is ready for reader review, not independently
accepted. No foundation-package sweep has begun. The working tree contains documentation-only
changes; final file hashes, the tracked diff, and check logs are under ignored
`target/documentation-clarity/phase01/`.

Changed explanations:

- [Policy and related options](../../../../crates/blueprint/src/context.rs): default direct-input
  selection, additional selectors and exclusions, exact-source string construction and later
  resolution, budgets, branch/access checks, sessions, and required-candidate failure. The new
  doctest attaches a policy through `TaskConfig`, requests two levels of ancestors, limits
  selection to eight items, and explicitly chooses `OmitOversized` with required checks enabled.
- [Task attachment](../../../../crates/blueprint/src/model/node.rs): `new`, `direct_inputs`,
  output roles, and the distinction between definition policy and the attempt's manifest.
- [Manifest version and public manifest introduction](../../../../crates/model/src/context.rs):
  selected sources, byte verification, known producer identities, unsupported-version/digest
  refusal, and the separate version 1 envelope and version 2 body.
- [Output allowance](../../../../crates/model/src/task.rs): constructor/accessor meaning,
  `1..=4_000_000`, invalid-input errors, `max_tokens` mapping, separate byte limits, and endpoint
  support. A doctest demonstrates a valid allowance and both out-of-range refusals.
- [Blueprint introduction](../../../../crates/blueprint/src/lib.rs),
  [model introduction](../../../../crates/model/src/lib.rs), and new
  [blueprint](../../../../crates/blueprint/README.md) / [model](../../../../crates/model/README.md)
  READMEs: purpose, entry points, source traces, limits, and verification. The existing blueprint
  genesis example was retained; the model introduction adds a request/document round-trip example.

Representative before/after: “Immutable, declarative context policy owned by a task definition”
gave no construction or operational consequence. The new explanation starts with which inputs
and earlier results the runtime may consider, shows attachment, and explains why a requested
source can be omitted or prevent dispatch. “Provider input-unit cap for generated output” now
identifies the requested generated-output allowance sent as `max_tokens`, with a link to the
shared bound and endpoint limits. The numeric ceiling's historical rationale was not established
or invented, and no provider-support qualification was added.

The supporting trace runs through [policy validation](../../../../crates/blueprint/src/context.rs),
[graph validation](../../../../crates/blueprint/src/validation.rs),
[runtime dispatch](../../../../crates/runtime/src/engine/dispatch.rs),
[discovery](../../../../crates/runtime/src/context/source/discovery.rs),
[selection](../../../../crates/runtime/src/context/selection.rs),
[manifest decoding](../../../../crates/model/src/document.rs),
[selected materialization](../../../../crates/runtime/src/context/source/materialize.rs),
[host materialization](../../../../crates/capability-host/src/materialization.rs), and
[provider negotiation](../../../../adapters/model-provider/src/adapter.rs). Both
[OpenAI-compatible](../../../../adapters/model-provider/src/openai_compatible.rs) and
[Anthropic](../../../../adapters/model-provider/src/anthropic.rs) request mappings use the same
output accessor. ADRs [0011](../../../decisions/0011-causal-context-manifests.md) and
[0012](../../../decisions/0012-provider-neutral-model-endpoints.md) supplied the intended boundaries.

Executed checks (all passed):

| Check | Result / log in the phase 01 target directory |
| --- | --- |
| `cargo fmt --all -- --check` and `git diff --check` | Passed; `format-and-diff.log`. |
| Blueprint `kernel`, all features | 23 tests; `blueprint-kernel.log`. |
| Model `contracts`, all features | 4 tests, including exact manifest fixture/digest checks; `model-contracts.log`. |
| Runtime `causal_context`, all features | 9 tests covering selection, exclusions, authority, budgets, isolation, persistence, and failed publication; `runtime-causal-context.log`. |
| Both affected package doctests, all features | 3 blueprint and 2 model tests; `doctests.log`. |
| Both affected packages' rustdoc, all features, no dependencies, `RUSTDOCFLAGS=-D warnings` | Passed; `rustdoc.log`. Previous environment value restored. |
| Repository `documentation::` contracts, all features | 8 tests, including local links and source-derived versions; `documentation-contracts.log`. |
| Runtime `structured_runtime` exact production context test | 1 test, `causal_context_production::reviewer_receives_frozen_causal_evidence_without_private_sibling_transcript`; `production-context.log`. Covers real dispatch, frozen retry selection, join isolation, and restart. |
| Model-provider `mock_endpoints`, all features | 12 tests; `provider-mappings.log`. These use mock endpoints, not real-provider qualification. |
| Source and fixture preservation | All six changed Rust files differ only in rustdoc; all four blueprint/model fixture hashes match HEAD; `source-and-fixtures.log`. |

Rendered rustdoc was inspected in a browser for both crate introductions, `TaskContextPolicy`
and its example, `TaskConfig`, `ContextManifest`, `MAX_MODEL_OUTPUT_UNITS`, and the model request
constructor/error example. The manifest-to-policy link navigates correctly. README structure,
code fences, source links, and local guide links were reviewed and covered by documentation
contracts. No full executable gate was run because source behavior, manifests, test assertions,
and fixture data are unchanged.

Unresolved implementation findings are recorded on the
[whiteboard](../whiteboard/issues/context-policy-enforcement.md): policy session intent is not
enforced against the adapter request, and `StopAtFirstOverflow` can omit later required evidence
even with `fail_closed = true`. The docs disclose these limitations and the example avoids them.
They remain source-derived findings requiring a focused executable assignment; the existing
passing tests do not prove those missing cases. No other claim remains unresolved in this scope.

Next permitted action: a reviewer checks whether the explanation lets a contributor construct
the policy, follow its application, and interpret a limit failure without reading method bodies.
Acceptance and any subsequent phase assignment remain with that review; this handoff does not
dispatch further work.

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
