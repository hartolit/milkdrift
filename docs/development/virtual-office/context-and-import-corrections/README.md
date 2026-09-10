# Context and import corrections

Make context-policy decisions hold through selection and dispatch, and make imported revisions
identify their actual input schema. This sprint closes the remaining
[context enforcement](../whiteboard/issues/context-policy-enforcement.md) and
[import-label](../whiteboard/issues/prompt-sequence-version-label.md) findings from the documentation work.

Prepared against `b6798116eabafe4a38b28688843b85a092a6ddf7` on 2026-09-09. Phase 01 implementation
uses that base with the existing uncommitted sprint/whiteboard preparation preserved. The roadmap's
real external-interoperability qualification remains separate and still needs an operator-supplied
coding-agent profile.

## Outcome and boundaries

The completed change must establish that required context cannot be silently skipped after
selection stops, protected omission metadata stays protected regardless of its reported reason,
and a model invocation cannot contradict the governing task's session declaration. Newly compiled
prompt-sequence revisions must name the accepted import schema while prior revisions retain their
exact identity and bytes.

Own these corrections across their callers, tests, and maintained explanations. File lists in the
prompts are starting points, not limits on necessary work. Preserve the
[architecture](../../../architecture.md#context-and-artifacts),
[ADR 0011](../../../decisions/0011-causal-context-manifests.md), and
[scope freeze](../../../product/roadmap.md). This sprint does not add session-continuation protocols,
providers, workflow primitives, controller activation, or a general context redesign. Broader
findings follow the existing [scope policy](../../workflow.md#findings-beyond-the-assignment).

## Assignments

| Assignment | Owner | State | Completion point |
| --- | --- | --- | --- |
| [01 — Repair enforcement and import identity](01-implementation.md) | Codex | Complete | Both findings corrected, regression evidence and full gate reported, implementation ready for final reader review. |
| [02 — Review, explain, and close](02-review-and-close.md) | Unassigned | Ready | Remaining defects corrected, maintained explanations coherent and accurate, integrated evidence checked, sprint closed. |

To execute the whole sprint, read [AGENTS.md](../../../../AGENTS.md) in its required order and
complete both prompts. Each prompt directly names its practices. One agent can carry the sprint
through both assignments; phase boundaries do not require a new task or delegation. An instruction
to execute only one phase assigns that phase's stated scope and stop condition.

When execution begins, replace the owner and state in this table and mark the two whiteboard
topics assigned to this sprint. Keep one short current handoff here if work must continue in
another session: what is settled, changed files or commit, check results and log locations, and
the next concrete step. Keep raw logs under ignored `target/` paths. Phase 01 is an implementation
checkpoint; the sprint remains unfinished until phase 02 accepts the result.

## Phase 01 handoff

The implementation corrects selection, omission disclosure, retained-evidence reuse, model session
agreement, and the import label. [ADR 0031](../../../decisions/0031-context-enforcement-and-retained-evidence.md)
owns the compatibility choices. New manifests use the existing selection-policy version field
with value 2; document schemas and old fixtures stay unchanged. Legacy omissions that cannot
establish safe reuse refuse retry or leased-work recovery without rescanning history or rewriting
saved bytes. The conservative refusal can include an old reference that happened to be safe.

The session check lives at runtime's effect claim boundary and reads the actual inline or exact
artifact document. It uses independent artifact read authority and the model owner's byte limit.
Legacy capability snapshots without category do not exempt `model.generate`; an exact schema-v1
event/store-reopen regression first reproduced that bypass and now requires a durable rejection.
The actual runtime/host/provider test covers all nine declaration/request combinations in both
forms: Fresh completes and publishes outputs, mismatches never enter the host, and matching
continuation reaches the existing provider refusal without HTTP. That last case retains the host's
existing conservative uncertainty classification. Process stage data still belongs to the configured
process contract. No continuation protocol is added.

The artifact-backed success case exposed missing journal metadata for a direct artifact binding.
Dispatch now records the artifacts derived by `NodeScheduled::required_artifacts` before scheduling,
so output provenance can cite its request. Existing exact-reference, causal, and accounting checks
remain in force. This needs no extra initial workspace value and changes no run-creation API.

Sequence assertions isolate the reason-only identity change: the new fixture revision is
`rev_6bfe94ac55d6950ae351d156baf23ad34f98f5b58e916b6f89ec0bf8eb229557`, with descendant
`rev_0d9a5e37e0ea86cdea711c347e6e181e9c13505adad829d923260bb518328c42`.
Import/profile digests, semantic content, and remediation mutation identity are unchanged.
The production revision reader still accepts and exactly re-encodes the known historical reason
and revision ID; altering only the reason while retaining the wrong ID fails.

New selector regressions reproduced the stopped-required and reason-dependent-redaction defects
before correction. Production discovery tests check serialized protected omissions and selected-only
materialization. Legacy schedules are seeded through the real atomic store port; retry and full
store-reopen tests observe refusal, no additional executor entry, and unchanged historical evidence.
A temporary session-check bypass fails the new regression (`target/context-import-corrections/session-fault.log`)
and was restored before verification. `legacy-session-before.log` in that directory records the
valid historical-category reproduction; `context-final-focused.log` records all nine corrected
production regressions passing.

The final Windows/MSVC full gate passes: 725 workspace tests, five existing manual longevity tests
ignored, formatting, fixture builds, all-target/all-feature checking, warning-denying Clippy and
rustdoc, deny/machete/duplicate-tree review, test discovery, and all 24 repository contracts.
The initial unrestricted parallel test build exhausted compiler memory; the final run used
`CARGO_BUILD_JOBS=1` and the workflow's real Python prerequisite. No test assertion or bound was
relaxed. Manual release longevity and real external interoperability were not rerun.

Raw gate commands/results are in `target/context-import-corrections/full-gate.ps1`, `results.json`,
and the adjacent named logs. `source-identity-at-gate.json` records the base and per-file hashes,
including untracked implementation files; `tracked-before-gate.diff` and `status-before-gate.txt`
record the dirty checkout. Final handoff/status edits are prose only and receive a separate
documentation-contract check. The checked code remains uncommitted on the preparation base.

All 21 library roots and 42 default/all-feature API inventories were reviewed. The only added
export is model's `MAX_MODEL_DOCUMENT_BYTES`, a workspace adapter contract consumed by runtime's
bounded artifact reader. The limit already governed the model reader and encoder; runtime now
uses that same ceiling. Existing test/evidence helpers remain
outside default features. Raw inventories and comparisons are under
`target/public-api/context-import-corrections/`.

Phase 02 should review the conservative policy-version-1 reuse decision, the division between
runtime session agreement and adapter support, direct artifact provenance through dispatch, and
the related explanations before accepting topic disposition. Both whiteboard topics remain assigned;
phase 01 does not close the sprint or qualify external interoperability.

## Acceptance

- Combined-case tests establish required-loss refusal and omission redaction through production
  discovery, persisted manifests, and the consumers affected by the correction.
- Session tests cover supported agreement, contradictory declarations, and unsupported requests
  before external entry. Process-stage declarations remain distinct from model-session support.
- Retry and restart checks preserve frozen selection, immutable history, and exact identity;
  they cannot bypass the new enforcement through reuse of a prior manifest.
- Import tests check the generated reason against the validated document version and account for
  its effect on revision identity without rewriting historical imports.
- A reader can follow how a task's policy becomes the context an invocation receives and understand
  meaningful refusal and compatibility consequences from the maintained documentation.
- Verification follows the [workflow](../../workflow.md#choose-verification-for-the-change).
  Preserve successful evidence for an unchanged tree; rerun when edits, failures, or unresolved
  concerns invalidate it. Full-gate failures remain failures until resolved and accurately reported.

Phase 02 owns final topic disposition and cleanup under the
[office procedure](../README.md#close-and-remove-a-sprint). Retain the results in their existing
source, tests, documentation, and Git; remove this sprint and resolved whiteboard topics after
acceptance. Stop there rather than beginning the roadmap's next milestone.
