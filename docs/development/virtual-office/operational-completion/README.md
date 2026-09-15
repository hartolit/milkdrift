# Operational completion — reviewed core to bounded continuous operation

## Outcome

Complete the specific gaps in the September 11 implementation review without reopening the
architecture. The desired result is a headless workflow that can reject an unusable stage result,
run the existing authorized controller lifecycle, propose and approve prospective remediation,
continue within durable cumulative limits, and remain inspectable through failures and restart.

The source review is `milkdrift-current-implementation-review.md`, based on archive commit
`aa77c483626b100e0f550c63648875ba28182238`. That commit is a reference, not a required checkout.
Every assignment rechecks its findings against the actual current source and consumes the previous
assignment's accepted result. Do not reset to the reviewed commit or resurrect the old Pass-2
failure log, wildcard-authority findings, JSON peer store, or discarded UI work as current defects.

The review found a credible core. Preserve it. A limitation is not automatically a defect, and a
large file is not by itself permission to refactor a subsystem.

## Installation and coordination

Place this directory at `docs/development/virtual-office/operational-completion/`. Add one link in
the existing virtual-office README's current-sprints section. Do not replace that README or its
whiteboard. The plain Markdown assignments use the existing office procedure; no importer, task
manifest format, plugin, or new execution framework is required.

The user or designated coordinator accepts results. Each numbered assignment belongs to the next
assigned implementation agent; assign a real owner when execution begins. Run **01 through 08
sequentially**, one responsibility per context. README is coordination material, not an execution
task. Use the repository as the handoff; do not rely on another agent's chat history.

| Order | Assignment | Required result |
| --- | --- | --- |
| 01 | [Workflow result acceptance](01-workflow-result-acceptance.md) | Required outputs and verified decisions—not transport completion—control continuation. |
| 02 | [Model external-effect stages](02-model-external-effect-stages.md) | Proven no-request failures are distinct from genuinely uncertain external outcomes. |
| 03 | [Qualified controller and budget clarity](03-qualified-controller-and-budgets.md) | Existing lifecycle is integrated and qualified through the daemon, with explicit accounting scope. |
| 04 | [Read-only recovery and safe storage operations](04-recovery-and-storage-operations.md) | Blocked stores can be inspected and backed up without dispatch or historical rewriting. |
| 04A | [Authorized recovery controls](04a-handoff.md) | Reviewed prospective repair can unblock the same generation before normal startup validates execution. |
| 05 | [Explicit continuation](05-explicit-continuation.md) | Existing exact-reference continuation works through supported model mappings without hidden sessions. |
| 06 | [Constrained peer placement](06-constrained-peer-placement.md) | Task locality/peer constraints are expressible and enforced through admission, selection, and entry. |
| 07 | [Trusted-process cleanup bounds](07-trusted-process-cleanup-bounds.md) | Owned I/O cleanup is interruptible and bounded without false descendant-containment claims. |
| 08 | [Integrated operational acceptance](08-integrated-operational-acceptance.md) | Combined behavior, measured query costs, and current evidence support a precise operating decision. |

Assignment 01 is committed as `bf8cbd5`; Assignment 02 is accepted at `741b230`. Assignment 03
has completed corrective 03A model-budget implementation and integration review, including fixes
for streamed usage evidence, nullable details and billed-call monetary permission. It has demonstrated two zero-spend local
model calls, accepted repair, and refusal of a third model request before transmission. Its
[handoff](03-handoff.md) owns the finite activation decision and evidence. Production activation
remains blocked pending coordinator acceptance of the corrective qualification. Assignment 04 is
implemented by the current Codex task, explicitly assigned by the user at base `d537059`.
Its [handoff](04-handoff.md) records the completed offline operating path, full verification and
preserved execution restrictions; the result is ready for coordinator acceptance.
Assignment 05 is owned by the current Codex task, explicitly assigned by the user at base `9f6bd0f`.
Its [handoff](05-handoff.md) records implementation, verification and review state. Assignment 06
is owned by the current Codex task, explicitly assigned by the user at base `6967da8`. Assignments
07–08 remain unstarted and require separate assignment/coordinator acceptance. The
[06 handoff](06-handoff.md) records the completed placement/artifact boundary, verification and
compatibility changes, reviewed and committed to `main` at the user's request.

The user additionally authorized implementing the missing repair path after assignment 04.
The same task owns 04A; [its handoff](04a-handoff.md) records the result and evidence. This addition
does not change the separately assigned scope of 05–08. Control protocol advances to 2.6 for
recovery health; storage formats, proposal/command schemas and daemon configuration remain unchanged.

The canonical acceptance owner is `crates/control/src/acceptance.rs`, exposed through the existing
`workflow.accept_result` operation. Assignment 03 should compose `result_acceptance_task` and
`result_acceptance_gate` at its stage boundary and continue only from the gate's `pass` arm.
`acceptance_result` records the decision; `accepted_result` aliases that exact artifact only when
the requirement passes. Compiled sequence associations identify `acceptance_node`; use saved
associations rather than guessing node names. Assignment 03 installs the existing lifecycle before
recovery when explicit development qualification is configured; ordinary startup stays disabled.
The [acceptance guide](../../../guides/result-acceptance.md) and
[ADR 0032](../../../decisions/0032-purpose-specific-result-acceptance.md) own the durable explanation.

Prompt-sequence schema 3 removes the optional success artifact and requires a typed verifier
checkpoint/check/coding report. Schemas 1/2 are refused on import; stored revisions keep their
behavior, and schema-2 stage associations remain readable. Both initial and remediation paths
gate verification and review. The existing remediation builder targets the original verification
failure hold; an unusable reviewer has a separate rejection hold requiring an explicitly authored
prospective proposal. Assignment 03 advances control protocol to 2.5; CLI JSON remains 2, and acceptance contract/result
schemas are 1. The corrective work advances endpoint/controller policy to schema 2 and redb internal
document format to 16; [ADR 0033](../../../decisions/0033-explicit-controller-qualification.md) owns compatibility.

Assignment 02 makes the model adapter's private preparation path the owner of validation,
materialization, and exact HTTP request construction. Entry consumes that retained request;
the former late construction path is removed. The host keeps its exact generation permit while
runtime rechecks authority and commits entry intent/account admission against the checked run head.
Only a durably recorded pre-intent refusal proves no request. Complete-response reporting failure
has a typed stage, while missing terminal evidence remains uncertain. Existing schemas, historical
readers, result acceptance, and account reservation/settlement policy are unchanged. The
[model adapter guide](../../../../adapters/model-provider/README.md#prepare-once-before-external-entry),
[architecture](../../../architecture.md), and ADRs 0012/0019 own the lasting explanation.

Assignment 02 verification passes: the full workflow gate (727 workspace tests, 24 doctests, five manual tests
ignored), all 24 repository contracts, dependency audits, test discovery, affected default/all-feature
API review, and both actual-binary operator/deterministic-model lanes. Nine new model tests cover
zero-request refusal, exact prepared entry, authority revocation, duplicate delivery, transport loss,
publication/report faults, and crash/reopen. The controller regression proves unchanged account
state after preparation refusal. Logs, source/binary hashes, and real-model limits are recorded in
the [evidence guide](../../verification-evidence.md#actual-binary-scenarios).
The first Bonsai alias passes; the second reaches its 180-second limit and remains one uncertain
attempt through two reopens. Thinking settings remain unknown. No new combined external qualification
or controller activation is claimed. Coordinator review found no blocking implementation issues.

Assignment 03 preserves the preparation boundary and acceptance routing above. Supported explicit
text counting and billing contracts now produce request-specific reservations and settle from
frozen usage facts. Unknown profiles remain refused. The existing installed scenario connects model
review to prospective repair, separate approval, acceptance, cumulative stopping and restart; its
[handoff](03-handoff.md) distinguishes implementation, demonstrated operation and coordinator acceptance.

Keep one short current handoff per active assignment here, only when needed. Record actual owner,
base/result commit, accepted coverage, checks, and any remaining blocker; update rather than append
progress diaries. Raw logs, measurements, and temporary fixtures belong under ignored `target/` or
CI artifacts. At closure, move lasting explanations to their canonical owners and remove completed
sprint material according to the existing office procedure. Git retains history.

## Scope and rules shared by every assignment

Read `AGENTS.md`, `docs/product/vision.md`, `docs/architecture.md`, `docs/product/status.md`,
`docs/product/roadmap.md`, `docs/development/workflow.md`, and the applicable implementation and
documentation practices. The detailed vision stays at its current path. Do not reconstruct it,
restore root-level duplicates, or copy the constitution into new policy files.

Inspect current source, consumers, tests, and production composition before editing. An earlier
review is evidence to investigate, not an instruction to ignore subsequent fixes. If an assigned
behavior is already correct, demonstrate it and complete the missing tests/explanation without
inventing replacement code. Finish complete ownership boundaries, including error paths, schemas,
callers, configuration, and docs. Do not choose a known-worse temporary design or start a framework
for hypothetical future consumers.

Use one owner for Cargo jobs and shared files. Preserve unrelated working-tree changes and the
coordinator's commit/branch policy. No mandatory tag, exact SHA, push, branch reset, or clean-tree
ritual is introduced by this bundle. An interruption resumes the current assignment from its actual
diff and handoff; it does not restart a broad audit or open an unlimited polishing loop.

These prompts authorize code and tests for the described product work, **not** production deployment,
spending provider credits, altering grants/secrets on running hosts, deleting stores, or restarting
the Milkdrift instance executing this office. Use isolated temporary stores and loopback fixtures.
Use a real external provider only with an explicit configured endpoint, allowance, and authorization.
Never weaken identity, authorization, evidence, or unknown-usage checks to make a demonstration pass.

Existing accepted external evidence may be reused only within its recorded commit/platform/model/
configuration/boundary scope. A missing required external qualification is a specific blocker to
that claim or activation—not permission to fabricate a pass, and not a reason to abandon unrelated
implementation. Complete available work, retain safe defaults, and report the exact remaining case.

## Verification and handoff

Follow `docs/development/workflow.md#choose-verification-for-the-change`. Executable changes require
its full gate, including all-feature tests, dependency inspection, test discovery, and repository
contracts. Build required daemon/process helper binaries before isolated integration tests. Python
and Git requirements, and Windows Python-alias handling, remain as documented there.

Each prompt names focused suites for iteration. At acceptance, also run the affected **actual-binary**
operator/model evidence lanes defined in the workflow and `.github/workflows/quality.yml`; library
mocks alone do not establish production installation. Do not duplicate evidence commands or create
another reporting framework. Reuse successful checks while the relevant diff is unchanged; rerun
affected checks after changes. Never claim a test, platform, or external session that did not run.

Completion handoff: what became canonical; what was removed; exact tests/commands and results;
schema/compatibility consequences; remaining supported limitations; and the next assignment's
necessary context. A receipt saying `Success` or an agent summary saying “done” is not acceptance.
The coordinator accepts the completed diff and evidence before advancing.

## Review coverage and explicit dispositions

| Review concern | Owner/disposition |
| --- | --- |
| Invocation succeeds but review/output is unusable | 01: purpose-specific gates, missing-final/exhaustion cases, preserved invocation truth. |
| Endpoint pre-request rejection appears uncertain | 02: typed stage evidence, one-shot preparation, conservative crash behavior. |
| Controller libraries not installed in production | 03: one composition path, finite prerequisite assessment, qualified opt-in activation. |
| Per-request ceilings mistaken for lifetime budget | 03: explicit operator/read-model scopes and existing cumulative accounts. |
| Unsafe retained context blocks all normal inspection | 04: explicit read-only recovery with no execution and bounded diagnostics. |
| Cold receipts/tombstones and no general migration/export/delete lifecycle | 04: consistent offline backup/verify/inspection export and safe generation guidance. Do not prune replay identity or build universal migrations/online deletion. |
| Fresh-only model mappings | 05: implement existing Milkdrift-owned `ExplicitContinuation`; keep unsupported provider-managed/process sessions explicitly refused. |
| No locality/peer task selectors | 06: exact placement constraints for a concrete two-host workflow, not a cluster scheduler. |
| Inherited pipes can delay cleanup; process is not a sandbox | 07: bounded owned I/O cleanup; host privileges, OS quotas, and malicious-descendant containment remain explicit limits. |
| Historical queries may scan heavily | 08: measure real inspector/context queries and change only a demonstrated expensive path. No assumed performance defect. |
| Large hotspots/evidence-tooling maintenance cost | All assignments: change-local decomposition only. 08 reviews marginal complexity and duplicate test machinery; no LOC quota or global cleanup. |
| Stale macOS/current evidence summary | 08: reconcile exact CI/evidence scope without upgrading selected suites to full-platform qualification. |
| Whiteboard generation/thinking and configuration-provenance topics | 01/03/08 consume relevant evidence and update only inspected scope. Do not treat these open proposals as authorization for native LM Studio APIs, adaptive budgets, or new providers. |

The deliberately retained limitations are part of addressing the review honestly. A general
sandbox, arbitrary provider-managed sessions, cluster discovery, universal storage migration, and
online destructive garbage collection are not fixes established by that review. They need separate
requirements and authorization, not silent implementation inside this sprint.

## Final acceptance scenario

```text
fresh work -> verification -> unsatisfactory result
                               |
                               v
                   inspect frozen causal evidence
                               |
                               v
                   authorized controller proposal
                               |
                               v
                   approval -> prospective revision
                               |
                               v
                    remediation -> verification
                               |
                               v
                    useful result or explicit stop
```

Prove restart, authority changes, cumulative limits, and quality gating on this path. Then exercise
explicit continuation, constrained remote placement, blocked-store inspection, and bounded cleanup
with the same current product—not a demonstration-only runtime. Stop when the stated outcomes and
evidence are accepted. Do not start a UI or another sprint automatically.
