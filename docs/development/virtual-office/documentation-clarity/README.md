# Documentation clarity sprint

Status: phase 01, phase 02's contracts assignment, and phase 03's persistence commit assignment
are ready for review.
No rewrite result has been independently accepted.

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
| 02 | `crates/contracts` | ready for review | All current exports, private/test comments, README, and crate introduction reviewed by the worker. Independent acceptance and browser visual inspection remain; see the 02-contracts handoff. |
| 02 | `crates/capability` | queued | — |
| 02 | `crates/blueprint` | queued | Include remaining APIs after phase 01. |
| 02 | `crates/workspace` | queued | — |
| 02 | `crates/authority` | queued | — |
| 02 | `crates/model` | queued | Include remaining APIs after phase 01. |
| 03 | `crates/persistence` | partial; ready for review | `03-persistence-commit`: package introduction and command commit/result/replay path. See the handoff; discovery reads and the remaining persistence ports are still queued. |
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

### Phase 02 contracts assignment

Assignment `02-contracts` follows the user's 2026-09-08 request to execute
[phase 02](02-foundation-packages.md). Codex coordinates and executes the first queued package.
The explicit request authorizes this next assignment; phase 01 at commit `e93d749` supplies the
style example, but its independent acceptance remains pending. Reviewer: to be assigned after
handoff. The whiteboard's context-policy issue is outside this package and is left open.

The reader should be able to compose the JSON checks used by a document owner, interpret a
structural refusal, and distinguish shared mechanics from the caller's byte limits, schema,
semantic validation, and digest verification. Include all current exports in
`crates/contracts/src/lib.rs` and `src/text.rs`, their private implementation/test comments,
the crate introduction, and a new package README. Trace model document readers, capability
documents/identities/schema conversion, blueprint digest parsing, and host diagnostic truncation.
Other foundation packages are consumers for this assignment, not rewrite targets. Preserve
executable source, visibility, manifests, fixtures, versions, and existing test assertions.

Codex owns Cargo jobs and final diff review. Run contracts unit tests and doctests, model and
capability contract suites, formatting, warning-denying contracts rustdoc, and documentation
contracts under the verification policy. Inspect rendered introductions, linked APIs, and
examples; verify that source and fixture bytes outside comments remain unchanged. Keep raw
evidence under ignored `target/documentation-clarity/phase02-contracts/`. Stop with this package's
reviewable handoff and explicit remaining coverage; do not start another package or phase 03.

### Phase 02 contracts handoff

Codex completed the `02-contracts` edits on 2026-09-08 against base commit
`e93d749753d3e86b8e4fe22a14699a1e1bfbd13c`. This is a documentation-only result awaiting reader
review, not an independently accepted package. Logs, generated-HTML inspection, final diff,
working-tree identity, and file hashes belong under ignored
`target/documentation-clarity/phase02-contracts/`.

Changed and reviewed explanations:

- [Package README](../../../../crates/contracts/README.md) and
  [crate introduction](../../../../crates/contracts/src/lib.rs): the model document's read/write
  path, shared checks and caller responsibilities, a supported use with observable output,
  feature/setup requirements, adjacent owners, and verification commands.
- In `src/lib.rs`, `validated_string_type!` and its generated `new`/`as_str` explain validator
  inputs/results, ownership, exact text preservation, serialization, and invoking-package Serde
  requirements. `deserialize_via!` explains wire decoding, constructor conversion, and both
  failure boundaries. Both have executable construction/refusal examples.
- `JsonLimits` and all four fields, `JsonBoundKind` and its variants, `JsonBoundViolation` and
  its three accessors, and `CanonicalJsonError` and both variants: units, inclusive/zero bounds,
  encoded versus decoded text, scalar depth, diagnostic-path limits, and caller error mapping.
- `canonical_json_bytes`, `parse_json_without_duplicates`, `validate_json_value`, and
  `preflight_json_structure`: order of checks, allocation and total-byte responsibilities,
  recursive key sorting with preserved array order, decoded duplicate keys, trailing input,
  first-refusal behavior, and the remaining domain checks. Examples cover sorted output,
  duplicate escape spellings, byte-limit errors, and preflight/decoded-validation differences.
- [Text helpers](../../../../crates/contracts/src/text.rs): `truncate_utf8` retains its useful
  longest-borrowed-prefix explanation and adds zero/multibyte behavior and examples.
  `is_canonical_blake3_digest` retains its exact-spelling and owner-boundary introduction and
  adds content-verification responsibility, case/whitespace refusals, and an example.

The existing `JsonBoundKind` string/key/array/object variant descriptions, `JsonBoundViolation::kind`,
and `CanonicalJsonError::Bounds` were already sufficient under their expanded owning types and
were retained. Private JSON sorting/validation, visitor methods, text implementation, and all six
unit tests were inspected. Three ordinary comments now explain encoded-byte counting, checking
duplicates before map insertion can replace a value, and why the depth test's scalar matters.
Straightforward traversal, primitive visitor conversions, and remaining test setup needed no
extra narration; no existing test assertions changed. There are no additional source files or
unreviewed exports in the current contracts package.

The behavioral trace includes [model readers and error mapping](../../../../crates/model/src/document.rs),
[capability documents](../../../../crates/capability/src/document.rs),
[capability bounds](../../../../crates/capability/src/bounded.rs),
[identity construction](../../../../crates/capability/src/identity.rs),
[`SchemaContract` wire conversion](../../../../crates/capability/src/descriptor.rs),
[blueprint digest parsing](../../../../crates/blueprint/src/identity.rs), and
[host diagnostic truncation](../../../../crates/capability-host/src/adapter.rs). These consumers
were read for evidence and remain unchanged; this handoff claims no broader package coverage.

Two misleading descriptions were documentation drift: the old depth field mentioned only
containers although validation and its existing test also count scalar children, and the old
violation introduction promised a precise location although preflight always returns `$` and
decoded key paths are unescaped diagnostics. The revised docs and executable examples reflect
those established behaviors. No new unresolved implementation finding was identified.

Executed checks:

| Check | Result / log in the phase 02 contracts target directory |
| --- | --- |
| `cargo fmt --all -- --check` | Passed; `format.log`. |
| `cargo test -p milkdrift-contracts --all-features` | 6 unit tests and all 8 new doctests passed; `contracts.log`. |
| Model and capability `contracts` targets, all features | 4 model and 7 capability tests passed, including golden bytes, invalid shape/version, and constructor refusal; `consumer-contracts.log`. |
| Contracts rustdoc, all features, no dependencies, `RUSTDOCFLAGS=-D warnings` | Passed; `rustdoc.log`. Previous environment value restored. |
| Source and fixture preservation | Both changed Rust files have identical nonblank, non-line-comment source to HEAD; all 10 capability/model fixtures are byte-identical; `source-and-fixtures.log`. |
| Generated HTML inspection | Read all 13 crate/public-item introduction blocks and verified 35 local documentation links/anchors; `generated-html.log`. |
| Repository `documentation::` contracts, all features | 8 tests passed, including local links, Markdown structure, source-derived versions, and maintained example readers; `documentation-contracts.log`. |
| Final diff review and `git diff --check` | Scoped documentation-only diff reviewed; whitespace check passed; `final-diff-check.log`. |

The browser security policy blocked the local-file rustdoc URL. Generated HTML content and links
were inspected directly, and Markdown structure was reviewed, but this is not browser visual
evidence. Browser visual inspection remains a review action. No full executable gate was run
because implementation, manifests, test assertions, fixtures, and schema data are unchanged.

Next permitted action: review whether a contributor can compose the shared checks, explain the
sorted output and a refusal, and identify the caller's remaining obligations. The current
contracts package has no remaining source portion to rewrite; acceptance and browser visual
inspection remain. Capability is the next queued package for a separate bounded phase 02
assignment. Blueprint, workspace, authority, and model retain their queued portions, including
the phase 01 exclusions. No other package or phase 03 was started.

### Phase 03 persistence commit assignment

Assignment `03-persistence-commit` follows the user's 2026-09-08 request to execute
[phase 03](03-execution-packages.md). Codex coordinates and executes the first queued execution
package's command commit and replay portion. The explicit request authorizes this assignment;
phase 01 at `e93d749` supplies the style example, with independent acceptance still pending.
Reviewer: to be assigned after handoff. The whiteboard's context-policy topic does not affect
this storage contract. Existing uncommitted phase 02 contracts edits are preserved.

The reader should be able to follow a runtime command into an atomic storage request, identify
which facts commit together, and recover a saved result after an ambiguous storage error without
mistaking command replay for permission to repeat external work. Include the persistence README,
crate introduction, journal receipt/result and commit contracts, related error guidance, and
consequential private/test comments on this path. Inspect adjacent query, artifact, snapshot,
account, runtime, and redb owners as evidence. Detailed discovery, workspace provenance, artifact
streaming, snapshots, application/peer ports, controller transitions, and event families remain
separate portions. No executable behavior, API names/visibility, manifests, fixtures, or claims
of qualification may change.

Codex owns Cargo jobs and integrated diff review. Run persistence tests/doctests, redb command
replay and fault-boundary tests, relevant runtime restart/denial tests, formatting,
warning-denying persistence rustdoc, and documentation contracts. Inspect rendered introductions,
examples, links, and the final diff; retain raw logs and source/fixture preservation checks under
ignored `target/documentation-clarity/phase03-persistence-commit/`. Stop with the reviewed portion,
evidence, unresolved findings, and next portion to assign. Do not start another package or phase 04.

### Phase 03 persistence commit handoff

Codex completed `03-persistence-commit` on 2026-09-08 against base commit
`e93d749753d3e86b8e4fe22a14699a1e1bfbd13c`, with the pre-existing phase 02 contracts changes retained.
This portion is ready for independent reader review; the whole persistence package is not
complete. Logs, final file hashes, tracked diff, and working-tree identity are under ignored
`target/documentation-clarity/phase03-persistence-commit/`.

The reader can now follow run creation into a storage request, explain why its event, root scope,
input values, usage, discovery summary, and response must save together, and distinguish a durable
rejection from a storage error after commit. The receipt example demonstrates identical intent
with changed delivery metadata and a different fingerprint for changed intent. Recovery guidance
uses the saved result and its fingerprint or exact redelivery; it does not authorize another
external attempt.

Changed and reviewed scope:

- [Package README](../../../../crates/persistence/README.md) and
  [crate introduction](../../../../crates/persistence/src/lib.rs): purpose, run-creation trace,
  storage-facing entry points, constructor versus commit, publication/checkpoint ordering,
  lost responses, feature/setup requirements, and relevant verification.
- [Receipt and result](../../../../crates/persistence/src/journal/receipt.rs): both receipt
  constructors, identity/audit/intent accessors, fingerprint example, result construction,
  authorization retention, sequence interpretation, canonical writing, and versioned reading.
  The result reader's field checks are distinguished from the writer's generic JSON structure
  check. Straightforward identity, disposition, event-ID, and payload accessors were inspected
  and retained where the owning explanation supplies their context.
- [Commit contract](../../../../crates/persistence/src/journal/commit.rs): workspace mutation
  and accounting obligations, `RunIndexUpdate`, `AtomicRunCommitRequest` construction and both
  attachments, `AtomicRunCommitOutcome`, and all `RunJournal` methods. Existing field/accessor
  descriptions and the lease guard were inspected for this commit path and retained. Detailed
  index discovery and scheduling remain separate coverage.
- [Journal constants](../../../../crates/persistence/src/journal.rs) and
  [related errors](../../../../crates/persistence/src/error.rs): which lists/bytes are bounded,
  where refusal occurs, internal versus authorization-bearing result formats, conflicts,
  publication recovery, and the limits of storage-failure classifications. Unrelated application,
  peer, artifact-read, and administrative error variants remain for their owning portions.
- Private receipt validation/fingerprinting, result decoding/building, request validation and
  index validation were read. One private comment now explains why exact workspace/event equality
  also prevents hidden writes. Two
  [test comments](../../../../crates/persistence/tests/contracts/atomic_commands.rs) explain the
  zero-event rejection and hidden-value setup. Other straightforward validation needed no narration;
  executable source and assertions are unchanged.

The source trace includes [runtime receipt construction](../../../../crates/runtime/src/command.rs),
[accepted/rejected commit planning](../../../../crates/runtime/src/engine/command_planning/commit.rs),
[redb append/replay and guards](../../../../adapters/redb-store/src/journal/append.rs), and the
adjacent [query](../../../../crates/persistence/src/journal/query.rs),
[artifact](../../../../crates/persistence/src/artifact.rs),
[snapshot](../../../../crates/persistence/src/snapshot.rs), and
[account transaction](../../../../adapters/redb-store/src/controller_account.rs) owners.
ADRs [0003](../../../decisions/0003-redb-transactions-and-content-addressed-artifacts.md) and
[0004](../../../decisions/0004-side-effects-retries-and-uncertain-outcomes.md) supply the intended
transaction and uncertainty boundaries. Those adjacent packages were evidence sources, not
rewrite targets.

The old crate introduction's blanket “schema-v1” description and the result constant's “current”
label were documentation drift: the existing readers, writers, and golden tests establish the
current event format and separate internal/authorized result forms. The updated introductions
defer exact current versions to their owners. No schema, compatibility, durability, or
interoperability qualification changed, and no new unresolved implementation finding was
established in this portion.

Executed checks:

| Check | Result / log in the phase 03 persistence commit target directory |
| --- | --- |
| `cargo test -p milkdrift-persistence --all-features` | 48 unit/integration tests and 3 doctests passed, including the new receipt example and two existing compile-fail examples; `persistence.log`. |
| Redb `contracts` exact-name filters, all features | One command fault-boundary test and one reopen/replay/conflict test passed; `redb-faults.log`, `redb-replay.log`. The first injects failure before and after commit; the second reads retained history after reopening. |
| Runtime `durable_runtime`, all features | 8 tests passed, including full object teardown/recovery and durable authority denial without reevaluation on replay; `runtime-durable.log`. |
| Warning-denying persistence rustdoc, all features, no dependencies | Passed; final text in `rustdoc-final.log`. Previous `RUSTDOCFLAGS` restored. |
| Repository `documentation::` contracts, all features | 8 tests passed; `documentation-contracts.log` and final handoff check in `documentation-contracts-final.log`. |
| Formatting and final diff checks | `cargo fmt --all -- --check` and `git diff --check` passed; `format.log`, `final-checks.log`. |
| Source and fixture preservation | All six changed Rust files retain identical nonblank, non-line-comment source to HEAD; all 25 persistence fixtures are byte-identical; `source-and-fixtures.log`. |
| Rendered documentation | Browser inspection of the crate introduction, `RunJournal`, receipt example, atomic request, saved result, and a rustdoc-rendered README; `browser-review.log`. The crate-to-trait link navigated correctly; repository contracts check README source links. |

No full executable gate was run because the changes are prose, Rust documentation, and comments
only. These checks support the described software boundaries; they do not qualify filesystem
power loss, remote interoperability, or exactly-once external work. Browser review is the worker's
visual check, not independent acceptance.

Next permitted action: independently review whether a contributor can construct a receipt,
follow the accepted and rejected commit paths, identify which state must be atomic, and recover
an ambiguous command result. Then assign persistence's bounded journal/discovery reads and their
recovery consumers. Remaining portions also include detailed workspace provenance, artifact
publication/reads, snapshots, revisions, event/identity documents, application/peer ports,
controller accounts, clock, and administration. Existing historical-version labels in the event
and peer introductions need review in those portions. Runtime, capability-host, control, and
prompt-sequence remain queued. No additional package or phase 04 was started.

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
