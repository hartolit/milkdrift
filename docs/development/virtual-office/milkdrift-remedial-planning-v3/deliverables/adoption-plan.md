# Reconstruction and adoption plan

This specifies complete proposed changes after the deeper review. It is not production authorization.
The investigated production baseline remains `908e7893f5dadb84d12712573c8daaa946829e39`, identical in
production bytes to archive `c016cd3`. Current source, constants/readers and actual consumers must be
checked at implementation time for drift; the owner/meaning choices below are already made for planning.
The user has approved [U17/U18](../intent-source-excerpts.md) product behavior, not this implementation.

## Complete units, concrete removal and usable results

| Unit / owner | Exact change and consumers | Usable result, compatibility and proof |
| --- | --- | --- |
| Canonical method source — blueprint/runtime (P01) | Add structured WorkflowProgram and complete ProgramEditBatch, explicit clause order/results/Parallel exports/waits; one ExecutionPlan used by runtime, context, reconciliation, authority/risk, publication and inspection. Historical graph reader lowers directly to that plan. | Full CLI/JSON structured method execution and future repair before frontend gestures. Preserve old bytes/order/occurrence pins. New source-only writing; unproved legacy conversion refuses without losing old execution. D1–D8, old graph and active-frontier fault cases. |
| One external task — blueprint/runtime/control (P01) | Ordinary TaskConfig plus bounded explicit/join collection sources replaces operation-only capability reducer. Migrate dispatch/authority/support/state/reducer and control policy/service/publication consumers, new writers, fixtures and examples. | Full profile/placement/context for a judge; exact selected outcomes/refs remain. Delete special external reducer entry paths. Legacy lowering preserves old invocation identity, source ordering and default requirement/context without inventing missing outcomes. |
| Shared source construction — blueprint/control/daemon (P01) | Pure edits/lowering belong in blueprint; authorized application operations in control. Replace daemon private ModelWorkflow new writer/shape recognizer, migrate wire DTOs, CLI, model conveniences and prompt-sequence output. | One authoring route for all clients. Keep useful input dialect and shared compiler; exact immutable siblings remain legal. No frontend compiler or alternate legacy writer. |
| Real browser/client (P02/P03) | Existing HTTP/config owner adds exact opt-in origins; standalone apps/workbench uses release CSP endpoint policy, lossless numbers, bounded authenticated streams and exact request custody. | First actual source/input/run/result/recovery journey; real early user review. Keep memory-only tokens, explicit personal-profile IDB or export-before-send and reserved control capacity; supported local-record migration cannot discard unknown requests. |
| Ongoing commitment — existing control/controller family (P04) | WorkCommitment policy/action/question/evidence owner, shared packet/model admission, exact accepted action/run associations; explicit authority scope, account-origin establishment and final-entry fence. Runtime owns all execution. | Product new-goal and criticism→assessment→continue/stop without external orchestration. Manual mode shares owner. Delete driver common orchestration/provenance construction. Exact canonical commands and underlying receipts recover each gap. |
| Current discovery/relationships — existing read owners (P05/P06) | Derived authorized invocation/internal-run relationship; current publication management reads distinct from retained admin snapshots and invocation catalog; installation/approved-recipe pages from existing store/config. | Useful permission-filtered progress, truthful scoped cursors and no private IDs/counts. No parallel catalogs or durable relationship copies. Existing receipts/reverse published_source remain exact. |
| Managed rule ownership — persistence/redb/host (P06) | Pure managed acquire/entry/quiesce/handoff/return/release decisions move to persistence::managed. Redb supplies exact accepted facts and applies changes under one transaction's guards. | Same supported records and physical obligations. Delete moved redb policy bodies. Prove forged/stale facts refuse, faults preserve atomicity, one-worker child progress and unsettled writer exclusion. No mandatory data migration for moving code. |
| Serving and descriptors — capability/host/control/adapters (P06) | Common fresh direct/peer admission after distinct auth/replay/rate checks; checked typed descriptor extension reconstruction migrates publication and managed model producers. | Preserve caller distinctions and canonical descriptor bytes/digests; remove repeated fresh rules and JSON field surgery. Remote descriptor mapping keeps its real scope transformation. |
| Knowledge/evaluation — existing control learning/knowledge, workspace/persistence (P07) | Closed Knowledge versus ExecutableMethod subjects, exact KnowledgeItem provenance, Controlled/Retrospective assessment basis, accepted slot bindings, typed judgment evidence and separate reuse approval. | Research/architectural lesson without fake method/resource; self-review visible, required independence enforced, negative/inconclusive retained. Remove mandatory managed/method/publication coupling from ordinary knowledge, not protected-method verification. Old records map only to their exact protected profile. |
| Complete interaction and evidence (P08/P09) | Full NP3 graph/outline/machine round trips plus commitment, resource, service and knowledge interaction; actual combined human review, final full gate. | S21/S26/S28 and all S01–S30 outcomes covered. No deferred 'advanced' editing hole or provider/platform quality claim unsupported by actual environment. |

The source boundary deliberately precedes the first new frontend slice. The browser checkpoint still
comes before automatic coordination, resource consolidation and broad knowledge assessment. Human
feedback can reopen a choice on new evidence; it does not substitute for the comparison completed here.
Each unit includes production consumers/refusals/evidence and removes superseded owning paths when its
replacement is complete. P09 is not a backlog for known failing intermediate work.

## Source and active-work compatibility

Immutable legacy definitions and new structured definitions have one current checked-plan consumer.
An old graph need not decompose into regions merely to execute. Its version-specific reader preserves
all-predecessor/join rules, lexical branch order, outputs/context and protected meanings. New source
uses explicit order/containment/results. Historical events are never synthesized to make old runs look
as though they used new regions.

Editable conversion is a separate proved operation. It returns source plus source-to-old/plan mapping
only when ordering, activation, visibility, result signatures and agreement identity correspond. For
live prospective adoption it additionally maps accepted prefix, selected clauses, running/uncertain
occurrences and pending frontier. A refusal identifies the noncorresponding elements. Old work remains
inspectable/executable/reusable; an authorized new method can select retained evidence, but it is not
presented as conversion of that live work. This is a material supported-editing limit, accepted as the
planning tradeoff rather than hidden in an implementation audit.

Old capability reducers lower to one Task plan with their historical source collection/default contract.
Do not insert a second durable task into existing history or turn a protected reducer into an editable
Task merely because the new source kind has that name. Unstarted successor normalization and agreement
changes use the ordinary prospective/authorization boundary. Legacy source editing is not retained as
an alternative new writer; exact old receipt replay remains available without constructing new old data.

Version public/durable shapes at their actual owners and coordinate daemon, CLI, independent clients
and workbench. Old clients visibly refuse unsupported semantics. Keep old layout decorations distinct
from semantic conversion; preserve or explicitly discard only optional rebuildable caches/snapshots,
never accepted events, artifact references or request results. Current supported legacy state remains
readable; this proposal makes no migration promise for previously unsupported pre-release generations.

## Commitment accounts, authority and recovery

Commitment identity differs from a method, run, occurrence, attempt and publication: it owns the accepted
ongoing intent before and across those executions. Extend the existing controller application family
with that missing owner; do not create a second task scheduler or a global conversation-memory database.
An exact closed account origin distinguishes controller occurrence, published invocation and commitment.
Commitment establishment creates an account without a fictitious originating RunId. Existing arithmetic,
reservations, settlement, unknown usage and byte charges retain one owner.

One transaction accepts the commitment policy/first action, account establishment and external receipt.
A later run-creation transaction checks exact accepted action/association/authority/account/fence and
commits creation plus BindRun. Persist complete canonical internal commands before submission. Start,
proposal, apply/resume and cancellation use ordinary durable owner receipts; lost reporting recovers the
same action without another model invocation. Control records consumed exact results, not copied run
status. Pause/cancel/deadline fences must be checked atomically with owner-local durable entry acceptance,
including every actual local published ancestor even where the immediate account is different. An
effect admitted before closure may physically begin afterward; ordinary cancellation/uncertainty still
owns it. The gate must not rely only on the continuation driver. It cannot prevent B's
already accepted operation from entering while cancellation from A is delayed/lost. B enforces its own
recorded cancellation/deadline/current authority. A sends the exact ordinary cancellation and retains
the remote operation, attributable reservation and uncertainty until durable evidence resolves them.
Test A stopping after B acceptance but before its child entry; do not promise a distributed stop fence.

Existing marked controller methods can inherit the commitment account only when its entire allowance
fits their declared ceiling; otherwise local composition refuses before start. Do not invent a partial
balance/reset to make it fit. Existing bounded publication composition remains available under its own
service authority and separate attributable caller charge. Unrelated already-entered runs cannot be
adopted into a new allowance; their evidence or prospective revision can be considered with separately
shown existing ownership. Old accounts/receipts are not rewritten into commitments.

User scope/requirements changes are explicit authorized prospective policy decisions. The controller
cannot grant itself authority, weaken success obligations or accept criticism as fact. A new grant or
policy version never retargets an old accepted action. `AmendCommitmentPolicy` atomically pauses local
admission at a new generation and retains an immutable successor with exact carry-forward action/frontier
references. Current rights and containment must pass before reopening; affected future work stays held
for fresh action/proposal, and old authority/entered effects remain unchanged. Original aggregate ceilings
remain fixed; this contract has no budget top-up. Equivalent-plan/evidence no-progress ceilings,
clarification and escalations stop dependent work without inventing permission. Outstanding uncertainty
and resource holds survive stopping coordination.

## Knowledge data and claims

Preserve exact old learning declaration/comparison/promotion receipts. Their supported reader maps them
only to the protected-method profile, without changing bytes/digest or retroactively inferring broader
knowledge approval. New KnowledgeItem and assessment records are versioned and bounded. Criteria timing,
subject versions, selected inputs/outputs, role bindings, reviewer relationships, reasoning, limitations
and counterevidence remain inspectable accepted evidence.

Controlled assessments fix roles/inputs/criteria before comparison and bind generated IDs when they
exist. Two already-existing design artifacts can be compared under such a declared rubric; stronger
pre-generation/held-out isolation applies only where the chosen executable profile requires it.
Retrospective reviews record their later basis honestly. Human preference cannot be converted into
an automated verification pass. Knowledge approval makes specified evidence eligible for declared future
use; it does not alter a current run, select it everywhere, qualify a protected effect or publish a method.
The same learning/control family owns selection/assessment/approval, avoiding a parallel engine.

## Upgrade, rollback and canonical adoption

Before enabling a new writer, close admission and use supported orderly shutdown/reopen and verified
backup/version gates in an isolated upgrade rehearsal. Preserve unsettled obligations; do not require
an unknown physical effect to be falsely terminal. Old binaries refuse incompatible new generations.
Rollback requires both readable retained data and explicit accounting for every effect accepted after
the backup. Restoring files cannot undo remote calls, release editing holds or reset consumed allowance.
When rollback cannot meet those conditions, refuse and retain the newer store for authorized recovery.

P00 adopts only the reviewed/approved intent, architecture and durable decisions into their canonical
owners; status/evidence continue to describe what currently runs. The roadmap/office register finite
execution and P09's named final full-system gate. Promote the frontend practice through the selector.
Correct existing Any/FirstSuccess/All prose drift from source without changing its behavior. Keep useful
planning checkpoints/dissent until actual review/adoption is complete.

No additional discovery phase must decide whether the selected features exist or where their primary
owner belongs. Implementers still inspect exact current constants/readers/callers, choose local factoring
and repair newly discovered defects. Direct relay/owner migration, active management release, standards
interchange, stronger custody and broader platforms remain excluded extensions. No production changes,
prototype, live deployment, paid calls, pushing or execution of this program occurred in planning.
