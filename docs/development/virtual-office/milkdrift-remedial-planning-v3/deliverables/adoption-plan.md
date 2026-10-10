# Reconstruction and adoption plan

This specifies complete proposed changes after the deeper review. It is not production authorization.
The investigated production baseline remains `908e7893f5dadb84d12712573c8daaa946829e39`, identical in
production bytes to archive `c016cd3`. Current source, constants/readers and actual consumers must be
checked at implementation time for drift; the owner/meaning choices below are already made for planning.
The user has approved [U17/U18/U19](../intent-source-excerpts.md) product directions and migration
constraints, not this implementation.

## Complete units, concrete removal and usable results

| Unit / owner | Exact change and consumers | Usable result, compatibility and proof |
| --- | --- | --- |
| Method sources — blueprint/runtime (P01) | Add structured WorkflowProgram and complete ProgramEditBatch, explicit clause order/results/Parallel exports/waits; retain supported graph source and MutationBatch. One ExecutionPlan serves runtime, context, reconciliation, authority/risk, publication and inspection. | Full CLI/JSON structured execution and future repair before frontend gestures; native graph create/import/copy/edit/repair remain available. One source per revision; optional verified conversion, separate live adoption. D1–D8, old graph and active-frontier fault cases. |
| One external task — blueprint/runtime/control (P01) | Ordinary TaskConfig plus bounded explicit/join collection sources replaces operation-only capability reducer. Migrate dispatch/authority/support/state/reducer and control policy/service/publication consumers, new writers, fixtures and examples. | Full profile/placement/context for a judge; exact selected outcomes/refs remain. Delete special external reducer entry paths. Legacy lowering preserves old invocation identity, source ordering and default requirement/context without inventing missing outcomes. |
| Shared source construction — blueprint/control/daemon (P01) | Pure versioned edits/lowering belong in blueprint; authorized application operations in control. Replace daemon private ModelWorkflow semantic ownership, migrate wire DTOs, CLI, model conveniences and prompt-sequence output. | One application path for all clients and both source families. Old convenience adapters call the shared graph editor where required by their supported contract. Exact immutable siblings and graph copies remain legal under existing agreement rules. No frontend compiler. |
| Real browser/client (P02/P03) | Existing HTTP/config owner adds exact opt-in origins; standalone apps/workbench uses release CSP endpoint policy, lossless numbers, bounded authenticated streams and exact request custody. | First actual source/input/run/result/recovery journey; real early user review. Keep memory-only tokens, explicit personal-profile IDB or export-before-send and reserved control capacity; supported local-record migration cannot discard unknown requests. |
| Ongoing commitment — existing control/controller family (P04) | WorkCommitment policy/action/question/evidence owner, shared packet/model admission, exact accepted action/run associations; explicit authority scope, account-origin establishment and final-entry fence. Runtime owns all execution. | Product new-goal and criticism→assessment→continue/stop without external orchestration. Manual mode shares owner. Delete driver common orchestration/provenance construction. Exact canonical commands and underlying receipts recover each gap. |
| Current discovery/relationships — existing read owners (P05/P06) | Derived authorized invocation/internal-run relationship; current publication management reads distinct from retained admin snapshots and invocation catalog; installation/approved-recipe pages from existing store/config. | Useful permission-filtered progress, truthful scoped cursors and no private IDs/counts. No parallel catalogs or durable relationship copies. Existing receipts/reverse published_source remain exact. |
| Managed rule ownership — persistence/redb/host (P06) | Pure managed acquire/entry/quiesce/handoff/return/release decisions move to persistence::managed. Redb supplies exact accepted facts and applies changes under one transaction's guards. | Same supported records and physical obligations. Delete moved redb policy bodies. Prove forged/stale facts refuse, faults preserve atomicity, one-worker child progress and unsettled writer exclusion. No mandatory data migration for moving code. |
| Serving and descriptors — capability/host/control/adapters (P06) | Common fresh direct/peer admission after distinct auth/replay/rate checks; checked typed descriptor extension reconstruction migrates publication and managed model producers. | Preserve caller distinctions and canonical descriptor bytes/digests; remove repeated fresh rules and JSON field surgery. Remote descriptor mapping keeps its real scope transformation. |
| Knowledge/evaluation — existing control learning/knowledge, workspace/persistence (P07) | Closed Knowledge versus ExecutableMethod subjects, exact KnowledgeItem provenance, Controlled/Retrospective assessment basis, accepted slot bindings, typed judgment evidence and separate reuse approval. | Research/architectural lesson without fake method/resource; self-review visible, required independence enforced, negative/inconclusive retained. Remove mandatory managed/method/publication coupling from ordinary knowledge, not protected-method verification. Old records map only to their exact protected profile. |
| Complete interaction and evidence (P08/P09) | Full NP4 graph/outline/machine round trips plus commitment, resource, service and knowledge interaction; actual combined human review, final full gate. | S21/S26/S28 and all S01–S30 outcomes covered. No deferred 'advanced' editing hole or provider/platform quality claim unsupported by actual environment. |

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

U19 changes the writer transition: retain the existing versioned graph construction/import/copy/edit
contract within blueprint/control, with its current bounded schema and permission rules. No eligible-lineage
registry, inferred historical timestamp, or forced conversion is needed to authorize a graph mutation.
Default structured authoring and retained graph authoring are two explicit source forms owned by the
same module family, not competing runtime facts. Source shape cannot change behind an existing revision
identity. Exact request replay continues to use the original command and result.

Distinguish three operations. Graph-native mutation validates an exact immutable graph base and admits
its successor through ordinary proposal/control. Equivalent conversion returns source and correspondence
only when ordering, activation, visibility, result signatures and agreement meaning match. Prospective
adoption instead validates accepted prefix, selected clauses, running/uncertain occurrences and pending
frontier against a deliberately changed successor. Cross-family adoption requires total correspondence
for preserved occurrences/obligations and validates changed future work; it does not require unchanged
whole-definition meaning. Where correspondence cannot be established, that cross-family adoption refuses
while graph-native repair remains available. Refusal names actual unsupported elements and recovery
actions. Cancellation or new work must never be labeled equivalent repair.

| Existing state | Execution and inspection | Editing, repair and reuse | Recovery and migration constraint |
| --- | --- | --- | --- |
| Closed historical revision/run | Exact old source, IDs, ordering, events and receipts remain readable/replayable; new invocations retain old semantics. | History stays immutable. Authorized successors and permitted independent copies use the graph owner; optional equivalent conversion creates a new revision with provenance. | No retrospective events, reidentified revisions or fabricated conversion proof. Closed work does not require conversion. |
| Reusable inactive definition | Graph-native execution and inspection continue. | Existing versioned import/create/copy and successor operations remain; governed identity-bound copy retains its existing refusal. Verified conversion is a convenience, not reuse eligibility. | Retain artifact/pin/dependency facts; copy does not inherit ungranted authority or reset an existing account. |
| Active run with unentered future work | Accepted prefix and active pins stay exact. | Native mutations can change/remove pending work under existing reconciliation. Cross-family successor may do so only with preserved-fact/obligation correspondence; whole unchanged-graph equivalence is unnecessary. | Pause/plan/authorize/apply with current sequence guards; stale plans replan through the same owner. No unrelated unconvertible subgraph may by itself block valid native repair. |
| Active run with started or uncertain effects | Original governing revisions, effect identities, reservations and uncertainty remain authoritative. | Repair eligible future work while preserving those occurrences. A source edit cannot relabel an entered action or discharge unknown work. | Exact cancellation, status reconciliation, settlement and resource holds continue. Unmappable entered facts refuse adoption; retain native repair for unaffected future work. |
| Governed/protected work | Original agreement and verifier obligations survive. | Native edits use original agreement validation. Converted/cross-family candidates require checked legacy protected-shape correspondence, or an explicitly authorized new agreement where existing rules permit; not a pasted digest. | Task-only editable regions stay task-only. Existing inherited-child adoption refusal remains; reject unsupported change precisely, without pretending cancellation preserves repair. |

Retirement has a defined later decision, not a date in this program. First establish actual retained
revision/run/reference and client usage, positive conversion and native-to-new repair coverage (including
protected and uncertain work), then migrate consenting clients/definitions and stop new graph admission
only under separately reviewed deprecation. Existing accepted obligations retain their native operations
until safely completed/migrated. Inactive reusable definitions cannot silently lose reuse because their
last run ended. Any residual capability loss needs explicit approval; preserve historical readers and
exact replay even after writer retirement. No implementation agent may choose that product tradeoff.

Newly accepted Program agreements use NP4's separate v2 source-skeleton contract: fixed named
disjoint Task-only Sequence scopes, sealed outer program/input bindings/output signatures, exact
allowed requirements, aggregate task/revision limits and effect policy. Only direct Task bodies and
their internal output implementation are mutable; outside consumers use stable scope results.
This is an explicit new agreement, not automatic conversion of v1's protected boundary-edge rules.
Native graph v1 and converted v1 retain their original exact validation. Future source configuration
can change while a permitted active occurrence finishes under its old governing revision; source
version changes cannot weaken the existing live-state or effect checks.

Old public convenience command versions keep deterministic graph compilation, including the window
after a revision is stored but before its outer receipt. Changing a new convenience default requires
an explicit new version/source choice; retry cannot compile different source under the same key.
Carry the actual selected proposal base through delta/risk/reconciliation instead of inferring it
from the first sorted merge parent. These are concrete compatibility consumers, not future audits.

Old capability reducers lower to one Task plan with their historical source collection/default contract.
Do not insert a second durable task into existing history or turn a protected reducer into an editable
Task merely because the new source kind has that name. Unstarted successor normalization and agreement
changes use the ordinary prospective/authorization boundary. Retained graph edits can still construct
the legacy reducer shape under its original contract; its lowerer uses the shared Task plan, so there is
no second external-effect dispatch path. New structured authoring uses full TaskConfig.

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

Prepare allocates bounded exact workflow identity slots with reviewable purposes before definitions
exist and fixes maximum associated runs/slots. Current workflow permissions must cover each actual
import/proposal/create/start action; a commitment scope only permits commitment operations. Missing
workflow scope needs a separately authorized grant change, never controller-generated authority.
Existing broad scope can cover the prepared slots while policy narrows actual work; already authorized
rounds proceed without another human initiation. Revisions reuse their workflow slot and original account.

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
No-progress normalization is source-version-aware: materialize old lexical branch priority before
normalizing only proven nonsemantic IDs. A port rename changing that priority is a plan change, while
a new artifact ID alone is not new evidence. This remains a bounded stop proxy, not a correctness proof.

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
Unapproved or negative evidence can still be selected under actual read authority with its status
visible. Ordinary method adoption uses its normal authority/risk/agreement/reconciliation gates; it
does not acquire mandatory learning qualification merely because protected publication has one.

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
