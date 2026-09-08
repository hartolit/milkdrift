# Documentation clarity sprint

Status: revised after reader feedback. Earlier edits remain in the tree as drafts to reassess.
No rewrite result has independent acceptance under the revised standard. The assignments below
replace the former six-phase plan; the context example is no longer a prerequisite or style model.

Make Milkdrift understandable as a system: why its parts exist, how work moves between them, and
how a contributor or operator uses them. Improve explanations where comprehension is missing;
shorten or remove prose that only describes visible code. Apply the
[documentation standard](../../engineering-rules.md#7-documentation).

## Scope

Review maintained Markdown, all workspace package introductions and Rust documentation, and
non-obvious implementation/test comments. Complete related explanations across packages, rather
than stopping after one constructor, storage path, or README. Coverage means examining an area
and exercising editorial judgment; it does not mean adding comments to every item.

Preserve executable behavior, public names and visibility, dependencies, serialized data, fixtures,
test assertions, and qualification claims. Supported documentation examples are in scope.
Implementation redesign, new providers, UI, workflow primitives, and new documentation tooling
are excluded. Investigate source/doc disagreements under the
[findings policy](../../engineering-rules.md#findings-beyond-the-assignment).

## Execute substantial assignments

An instruction to execute one of these phases assigns its entire area by default. Start from the
current tree and record the worker in the table; no additional form or per-package approval is
needed. If the user explicitly limits the assignment, honor that limit. Preparing or reading this
plan alone does not dispatch its work.

Each worker owns a complete area, including package READMEs, crate/module introductions, API
explanations, and private/test comments. The table lists primary editing ownership to coordinate
work, not a restriction on which consumers may be read. Include related corrections needed for
the outcome; coordinate shared edits with the other owner if work is running concurrently.

| Phase | Reader outcome and primary ownership | State / worker / remaining work |
| --- | --- | --- |
| [01: Define work and its inputs](01-definitions-and-inputs.md) | Understand how a workflow is described before execution, how inputs and results are represented, and how shared document checks support those definitions. All of `crates/contracts`, `crates/blueprint`, `crates/workspace`, `crates/model`, and `crates/prompt-sequence`. | Unassigned; full area, including reassessment of earlier contracts, blueprint, and model edits. |
| [02: Execute, control, and recover work](02-execution-and-recovery.md) | Follow authority and commands through execution, storage, interruption, and future changes. All of `crates/authority`, `crates/control`, `crates/runtime`, `crates/persistence`, and `adapters/redb-store`. | Unassigned; full area, including reassessment of the persistence commit documentation. |
| [03: Run external capabilities](03-external-capabilities.md) | Understand how a capability is chosen, invoked, observed, and stopped locally or through a peer. All of `crates/capability`, `crates/capability-host`, `crates/peer-protocol`, `adapters/local-process`, `adapters/model-provider`, `adapters/local-secret`, and `adapters/peer-http`. | Unassigned; full area. |
| [04: Use the applications and maintained guides](04-applications-and-guides.md) | Operate the system and find coherent explanations of its design and development. All of `crates/control-protocol`, `crates/control-client`, `apps/daemon`, `apps/cli`, and `tools/evidence`; root entry documents, `docs/`, and maintained prose/comments in `examples/` and `.github/`. | Unassigned; full area. Preserve active sprint records and whiteboard contributions. |
| [05: Review and close](05-reader-review-and-close.md) | Check the complete reader experience, correct gaps, verify the integrated result, and digest/remove this sprint. | Unassigned; final acceptance follows completed coverage from 01–04. |

These four execution areas account for the current 22 workspace packages. Recheck workspace
membership before execution and final review. Phase 04 owns the final consistency pass over
shared guides; other workers can make coordinated supporting corrections there.

Phases 01–04 can proceed without waiting for a pilot or unrelated acceptance. Phase 04 may start
with existing source and align its guides with the other results before final review. Delegate
only when the user authorizes delegation; otherwise the assigned agent handles the area itself.
Follow the [office procedure](../README.md) for shared files and Cargo ownership.

Work through an area's related operations in internal iterations, carrying forward what has
already been learned. Do not stop merely because one package or path is finished. Split an
assignment only for a real ownership conflict, explicit constraint, or demonstrated inability to
complete the area coherently. A long file or phase number is not such a reason. If execution is
interrupted, retain one short checkpoint with completed coverage and the concrete place to resume;
resume the same assignment instead of treating each continuation as a new planning exercise.

## Editorial approach and acceptance

Read enough source and consumers to understand the operation before rewriting. Organize each
explanation around the reader's task and the relationships that make it work, then place details
at the API that owns them. Revisit the earlier edits under this standard; their passing tests and
length are not reasons to preserve weak writing.

The receipt limit in `crates/persistence/src/journal.rs` is a useful starting example: the
constant needs an unambiguous unit and scope, while the receipt explanation should teach why
retaining command identity and intent helps recover a lost reply. Avoid scattering constructor
validation details across constants. Also review the expanded contracts, context-policy, and
persistence introductions for repeated disclaimers, checklists, and walkthroughs that obscure the
main idea. Retain details that actually help callers make a choice.

Use short examples or ASCII diagrams where relationships or ordering are easier to see than
describe. Omit them where prose or code is already clear. Readers should not need to navigate a
forest of boilerplate to understand a simple operation. There are no quotas for rewrites, examples,
diagrams, removed lines, or comment length.

A completed area has coherent introductions and useful API documentation throughout its packages,
including areas inspected and intentionally retained. Its main operations can be followed through
the docs, and consequential choices and failures are explained in the right place. Reader scenarios
in each prompt are entry points into that review, not substitutes for the rest of the area.

## Progress and checks

Keep planning state in the table. During execution, add or update one short handoff for each
active area: reviewed packages/modules (including retained material), remaining gaps, changed
explanations worth reviewing, checks with the checked commit/working-tree context, and unresolved
findings. Do not maintain a per-symbol ledger or paste test output here. Detailed logs and any
temporary inventories belong under ignored `target/documentation-clarity/`.

Use the [verification policy](../../workflow.md#choose-verification-for-the-change). Group
doctests and rustdoc checks for changed packages, and select behavior suites to support specific
claims rather than repeating them after each comment edit. Reuse successful checks only while
their relevant inputs remain unchanged. Report actual checks and limits; no new external-service
qualification is part of this sprint.

The [context-policy issue](../whiteboard/issues/context-policy-enforcement.md) remains an open,
source-derived implementation finding. Phases 01–03 must recheck the relevant behavior before
making claims about session intent or required evidence. This plan does not revalidate or resolve
it. Keep broader findings on the whiteboard and fix ordinary editorial gaps within the assignment.

## Earlier work retained for reassessment

These records preserve useful starting points, not acceptance. Detailed former handoffs remain
in Git at `e93d749` and `7061eb6`; local logs may be available at the paths below.

| Earlier work | Existing result and verification record | Revisit in |
| --- | --- | --- |
| Context example, `e93d749` | Blueprint/model READMEs and selected policy, task, manifest, and limit rustdoc. Worker reported focused source/consumer tests, doctests, warning-denying rustdoc, documentation contracts, and rendered review passing. Logs: `target/documentation-clarity/phase01/`. | 01; runtime/adapter interactions in 02–03. |
| Contracts package, included in `7061eb6` | README and current export/private/test documentation reviewed by worker. Unit/consumer tests, doctests, rustdoc, documentation contracts, and direct HTML inspection were reported passing; browser visual review was blocked. Logs: `target/documentation-clarity/phase02-contracts/`. | 01. |
| Persistence commit path, included in `7061eb6` | README, crate entry, receipts/results, atomic commits, related constants/errors/test comments. Worker reported persistence/doctests, selected redb/runtime tests, rustdoc, documentation contracts, and rendered review passing. Other persistence areas were untouched. Logs: `target/documentation-clarity/phase03-persistence-commit/`. | 02, as part of the whole area. |

Those checks were for earlier trees and do not verify later revisions. None of these results
received independent acceptance, and the user's readability feedback applies to all of them.

## Finish

Submit the complete areas for [reader review](05-reader-review-and-close.md). Resolve concrete
comprehension or accuracy gaps, check all current packages and maintained document areas, and
verify the integrated changes. An accepted deferral must remain explicitly unfinished with an
owner; do not present a sampled or partial rewrite as completed coverage.

Put lasting explanations in their canonical owners and preserve unresolved whiteboard topics.
Then follow [sprint cleanup](../README.md#close-and-remove-a-sprint), replacing temporary links
before removing this directory. Do not turn this plan into root documentation or begin unrelated
product implementation.
