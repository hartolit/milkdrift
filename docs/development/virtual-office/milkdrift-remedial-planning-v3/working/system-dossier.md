# System dossier — reopened breadth and depth audit

## What the earlier review actually established

The user reopened the investigation after `ff818e5`; [the critique](../reopened-critique.md) is
the current assignment. Production source is unchanged. Rowan audited Cargo's actual workspace
membership (`cargo metadata --no-deps --format-version=1`), prior working evidence and the inspected
source/consumer references. There are **24 workspace packages**, including evidence and the independent
Slotbook example. A family/scenario row is coverage, not comparative architectural approval.

Depth below is retrospective at `ff818e5`: **B** means a concrete behavior/producer-consumer path was
examined (sometimes executed); **M** means mapped with source references but without comparable
design alternatives; **I** means mainly inherited canonical/historical evidence; **U** marks a
consequential relationship not resolved. These labels describe the cited scope, not every line in
the package. None of the old tests makes an adjacent unreviewed relationship approved. The current
investigation's completed comparisons are linked below. This table preserves the honest starting
point; its final column records the actual division of investigation, not a completion claim.

| Workspace package / source | Responsibility, neighboring owners and important consumers | Prior depth and unresolved relationship | Reopened investigation |
| --- | --- | --- | --- |
| [contracts](../../../../../crates/contracts/src) | Canonical bounded JSON/validated primitives used by every domain and both protocols; domains still decide meaning | M/I: shared mechanics accepted from repository rules; alternative placement not compared | Rowan: actual multi-consumer boundary, numeric/canonical rules and adding an input contract |
| [capability](../../../../../crates/capability/src) | Requirement/descriptor/selection/effect contracts; blueprint, host, adapters, authority | B for direct selection, M for generic operation/managed contract placement | Faris: selection and invocation-service alternative |
| [blueprint](../../../../../crates/blueprint/src) | Definitions, revisions, structured graph, agreements, mutations; runtime/control/authoring/importers | B for immutable siblings and merge refusal; M for the whole representation; U identifier-priority rationale | Delta: same authoring packets through current, complete correction and regions/standards |
| [workspace](../../../../../crates/workspace/src) | Scoped values, artifacts/producers/provenance/budgets; runtime/context/model/persistence/adapters | B selected branch/privacy paths, M for artifact/working-area organization | Elin with Delta: causal evidence versus editable work, continuous fresh context |
| [authority](../../../../../crates/authority/src) | Grant/resource decisions, frozen basis and secret references; control/runtime/host/protocol adapters | B revoke/replay/protected effects, M for authority/account split and human configuration burden | Faris: acceptance/entry/disclosure/account relationship alternatives |
| [model](../../../../../crates/model/src) | Provider-neutral task/response/manifest; runtime/model adapters/control parsers | B selected-input path, U actual goal-plan response consumer; M model abstraction breadth | Elin/Faris: response admission and provider contract distinction |
| [persistence](../../../../../crates/persistence/src) | Durable documents and ports/account transitions; runtime/control/host/redb | B exact replay/uncertainty scope, M for journal/state-owner division | Faris: transaction cuts, invocation unification and managed transition placement |
| [runtime](../../../../../crates/runtime/src) | Occurrences, scheduling, control, entry, reconciliation, causal context; host executor/control/store | B joins/recovery/future repair, M alternative representation and scheduling ownership | Delta with Elin/Faris: same-case activation/data/recovery/continuous control |
| [capability-host](../../../../../crates/capability-host/src) | Live generations/prepared entry/serving/workers/resources; adapters/runtime/control continuation | B direct/peer/publication lifetime paths; M unified effect owner alternative | Faris: origin-specific facts versus repeated admission rules |
| [control](../../../../../crates/control/src) | Proposals, controller/accounts, publication, learning/knowledge; runtime/host/daemon | B publication/recovery, M controller composition; U goal orchestration and general evaluation scope | Elin/Faris: actual orchestration, application commands, evaluated method contract |
| [prompt-sequence](../../../../../crates/prompt-sequence/src) | Trusted-process sequence import/compile/remediation; daemon/CLI/evidence | M/I ordinary compilation; U duplication with editor/agent constructors and future changes | Rowan/Elin: exact independent import contract and compiler locality |
| [control-protocol](../../../../../crates/control-protocol/src) | External command/read/layout DTOs/codec/cursors; daemon/client/CLI/future browser | B identity/recovery/stream/layout routes; M transport/domain command duplication | Rowan: concrete public-authoring/configuration change through consumers |
| [control-client](../../../../../crates/control-client/src) | Authenticated bounded HTTP, safe reads/artifacts/SSE/saved starts; CLI/evidence | B saved start/stream/integer browser implications; M client/domain placement | Rowan: replay ownership, helper dependencies and direct public JSON alternative |
| [peer-protocol](../../../../../crates/peer-protocol/src) | Session/catalog/execution/artifact wire; host/peer-http/daemon/direct client | B direct versus delegated origin, M control/peer wire boundary | Faris: direct and peer admission convergence without erasing owners |
| [local-process](../../../../../adapters/local-process/src) | Byte-pinned argv, materialization, process ownership/outputs; host/runtime/daemon | B controlled direct/managed paths, I platform stop evidence; no full mechanism alternative | Faris: narrow platform port and independent process-lifetime oracle |
| [managed-linux](../../../../../adapters/managed-linux/src) | Linux recipe/Podman/Quadlet effects and adapters; host/persistence/daemon | B child claim behavior, I physical qualification; M lifecycle/policy placement | Faris: mechanism versus managed transition policy, unchanged hardware scope |
| [model-provider](../../../../../adapters/model-provider/src) | Feature negotiation, provider HTTP/SSE mappings; model/host/daemon | B controlled requests/lost response, I provider-family evidence; M normalization costs | Faris: shared transport versus provider semantics, future feature case |
| [local-secret](../../../../../adapters/local-secret/src) | Restricted explicit environment/file secret resolution; host/daemon | I/M: no package-level source comparison recorded | Faris: direct minimal port versus embedding resolver in application |
| [peer-http](../../../../../adapters/peer-http/src) | Configured transport/auth/catalog/remote adapter/artifact frames; host/peer wire/daemon | B two-daemon/replay/direct-origin restrictions, M transport/lifecycle split | Faris: network mechanism versus durable serving owner |
| [redb-store](../../../../../adapters/redb-store/src) | Transactions/records/indexes/artifacts/admin/recovery; persistence consumers | B revision retention/managed claims/exact replay, M policy in storage and alternative ledgers | Faris: pure transitions, atomic evidence collection, origin of policy placement |
| [daemon](../../../../../apps/daemon/src) | Config/auth, queue/composition, commands/reads/streams/lifecycle/admin; all adapters and clients | B HTTP/authoring/publication; M application orchestration/config duplication | Rowan: command owner versus conversion layer, composition and configuration alternatives |
| [cli](../../../../../apps/cli/src) | Terminal inputs/presentation/recovery and local imports; client/protocol/blueprint/sequence | B real workflow/independent-host use; M private construction versus shared application action | Rowan: remove justified duplicate helpers, retain offline import where it earns cost |
| [evidence](../../../../../tools/evidence/src) | Maintained actual-binary/fixture/operational drivers, leaf only | B executed E01–07 scope; U orchestration supplied by driver but promised as product | Elin/Rowan: product-supporting proof versus hidden operator/planner responsibilities |
| [adaptive-slotbook](../../../../../examples/adaptive-slotbook/src) | Independent application and deliberate defect; external target for managed evaluation | I scenario evidence, U special-case learning assumptions promoted to general product | Elin/Rowan: example specificity versus product validators/outputs |

The first review's strongest actual design comparison was reuse/reference/copy/publication and
its permission/placement counterexamples. Its weakest foundations were whole-workflow structure,
agent orchestration, evaluation breadth and whether storage/application boundaries own rules in the
right place. Narrow adapters can receive lighter review when their source has one external-mechanism
purpose and multiple concrete consumers; their platform/quality claims remain limited to retained
evidence. Broad source coverage does not claim every platform implementation has been requalified.

## Reopened coverage and dispositions

Four substantive investigations cover the 24 actual packages and their consequential relationships.
Each includes source/consumer/history anchors, independent outcomes, compared alternatives, concrete
changes and limits. Their package-level tables refine this audit rather than giving blanket approval:

| Completed investigation | Package coverage joined here | Comparative depth and resulting direction |
| --- | --- | --- |
| [Structure and prospective change](structure-comparison.md) | blueprint; runtime activation/reconciliation; workspace/control/persistence consumers | Concrete current child-choice graph, complete corrected graph, canonical regions and actual BPMN subset on D1–D8. After constructive challenge, select canonical structured source lowered to one checked execution plan. Preserve legacy graph meaning; no asserted universal conversion. Remove operation-only external reducer path in favor of ordinary Task plus explicit collection. |
| [Agency, context and learning](agent-directed-work.md) | control; runtime context/controller; workspace; model; evidence; adaptive-slotbook and daemon learning | Actual producer search establishes missing general planner and managed-publication-only evaluation. External driver, finite round and continuous bounded control compared; U17/U18 resolve product scope, not architecture. Full root-run versus control-owned commitment comparison and shared knowledge-evidence reconstruction supply the selected implementation boundary. |
| [Execution/resource reconstruction](execution-reconstruction.md) | capability, capability-host, authority, persistence, peer-protocol, model; local-process, model-provider, managed-linux, peer-http, redb-store, local-secret | Same lost-reply/lost-stop/revocation/restart case through current, complete correction and unified invocation service. Retain distinct accepted facts; move managed transition policy out of storage mechanism, converge fresh serving admission and use typed descriptor construction. |
| [Application reconstruction](application-reconstruction.md) | daemon, cli, control-protocol, control-client, prompt-sequence, contracts; evidence/example relationships | Current raw/template authoring versus shared graph builder versus structured source; exact application/runtime receipt purposes; configuration and contract alternatives. Move pure semantic construction to blueprint and authenticated orchestration to control; preserve independent wire/client/importer boundaries. Correct the old publication-list finding's contract mismatch. |

This completes broad package coverage at deliberately unequal depth. Deep comparative review is
concentrated where choosing another representation or owner changes the product: source structure,
prospective repair, ongoing work, evaluation, entry/accounts/resources, public construction and
recovery. Small mechanism packages receive lighter source/consumer review because their alternatives
either put ambient effects into core or repeat one real shared mechanism. Their platform behavior
is not newly qualified by that conclusion.

Specific limits remain: no proof every currently accepted graph decomposes into regions; no measured
human/model authoring advantage; no new live provider/hardware/platform qualification; no complete
audit of every authority-fact producer, parser path, storage scanner or fairness/overload edge. These
do not inherit an overall passing grade. The selected compatibility rule supports old graph execution
without requiring that proof and refuses unsupported source conversion. Existing evidence continues
to own physical/platform claims. No newly uncovered issue is called a runtime bug merely because a
modeled contract is more useful.

New executed evidence is the [priority runtime diagnostic](priority-runtime-evidence.md): two true
guards, fixed destinations and swapped port IDs change the actual selected business route. That
supports explicit semantic ordering and exact preservation of legacy ordering. It does not execute
any proposed structured source or continuous-work design. E01–E07 remain the original observations.

The earlier scenario table below is retained as the `ff818e5` exposure record. Its provisional
R-C01/P2/NP2 and missing-orchestration claims are superseded by the linked reopened investigations
and current deliverables; it must not be used to reinstate a graph-only implementation or manual
reinitiation ceiling. S01–S30 are still required outcome coverage in the revised program.

## Earlier dossier at ff818e5 — preserved provisional analysis

Investigated source: `908e7893f5dadb84d12712573c8daaa946829e39`; production-identical to
`c016cd3`. Source findings are not newly executed behavior. Current version facts belong to
[status](../../../../product/status.md), owning constants/readers and fixtures.

Cyra-20261010 owns a bounded investigation of public command/read paths, ownership, permission,
lifetimes and browser gaps. Rowan owns Cargo and isolated observations. Ada and Bram independently
recover source intent before reading the synthesis. [Handoff](../handoff.md) records exposure.

Initial critical question: do reuse, restricted invocation and execution on another machine
require one mechanism? Establish independent predictions before the 02 trial. Browser investigation
must distinguish native HTTP success from browser permission to send/read authenticated requests.

Detailed inspected symbols, consumers and existing tests live in [Cyra's source evidence](behavior-evidence.md).
New results live in [E1 observations](observations.md); old status reports are retained evidence,
not newly run tests. The matrices below are coverage and interpretation, not a replacement source owner.

## Concept coverage and challenge

| Family | User purpose and actual owner | Disposition to compare; reopening trigger |
| --- | --- | --- |
| Definition/revision | Save a method and understand a changed future; blueprint graph/revision/mutation | Retain immutability; challenge authoring representation. Standard migration must preserve old identities. |
| Run/occurrence/attempt | Distinguish repeated work and actual capability entry; runtime journal/projection | Retain distinction in evidence, collapse detail in ordinary display. Reopen if public read lacks occurrence meaning. |
| Task/requirement/descriptor | Request work and choose authorized generation; blueprint/capability/host | Generic typed invocation already exists. Compare operation schema with control instructions; never delegate scheduling truth to prose. |
| Control/data/branch | Explain what starts and what it consumes; blueprint validation/runtime scheduling | Investigate exclusive reconvergence under actual all-predecessor readiness; no standard merge symbol without equivalent execution. |
| Fork/join/reducer | Explore alternatives, wait and transform selected results; structured runtime | Keep responsibilities distinct; one author action may construct them through daemon validation. Cancellation remains asynchronous. |
| Repeat/wait/signal/terminal | Durable bounded continuation and correlated external decisions; runtime | Retain behavior; compare standard notation and completion/cancellation meanings. No arbitrary cycle or implied broadcast. |
| Pinned child/copy/publication | Reuse without rebuilding; runtime child, daemon copy, control publication | P1/r2 distinct committing actions. Source-store and governing-identity restrictions persist until justified replacement. |
| Direct/remote invocation | Use tools without fabricated workflows; capability-host serving, peer adapter | Preserve origin and effect ownership. Third-host direct relay is an unproven demand/route, not a general promise. |
| Actor/grant/service identity | Permit useful bounded action without handing out internal privilege; authority/control | Retain scoped path and service boundary. UI role names never become grants. |
| Approval/adaptation/accounts | Evolve method without revising its acceptance or resetting ceilings; control/runtime/persistence | Retain prospective decisions; explicitly model agreement-changing escalation in S28. |
| Context/artifacts/knowledge | Supply selected evidence, explain omissions and reuse lessons; runtime/workspace/control | Retain frozen selections and separate current knowledge. Transfer/publication is not implied by local read. |
| Managed installation/working area | Keep useful tools/files alive, coordinate writers; capability-host/redb/managed-linux | Preserve owner and quiescence proof; release-to-operator is absent, conditional product choice. |
| Providers/adapters | Use external tools/models with honest features; adapter/host ports | Keep external inference, native tools, attached services. No new provider family authorized. |
| Connections/commands/pages/feeds/layout | Operate independent daemons; protocol/client/daemon; layout nonsemantic | Real browser is missing. Stable host identity/layout digest already exist; add no competing owners. |
| Evaluation/promotion | Compare an improvement before changing future selection; control learning owner | Retain negative/inconclusive outcomes and separate promotion; no successful-live-learning claim. |
| History/retention/recovery | Resume and understand effects without replaying them; persistence/redb/runtime | Preserve exact request results and uncertain holds. No automatic store migration or destructive cleanup. |

## Scenario coverage

“Source” means inspected path/tests, not full scenario execution. “Partial” means the current
backend provides ingredients but the required interface/composition is absent or untested.
Every row is retained in the target program; conditional additions remain separately named.
Authority is always checked again by the listed owner at action time.

| Case | Current evidence/support | Owner, rights and accepted meaning | Intended human path / interruption and required proof |
| --- | --- | --- | --- |
| S01 First interaction | Backend E01; browser E04 negative | Connection authenticates daemon host/actor/grant; construct/save/start are separate accepted commands | Zero connections → add → verify → author/run/result; disconnected owner does not block another. Real browser proof required. |
| S02 Different briefs | E01 and independent JSON E05 | Runtime creates separate runs from immutable revision and uploaded authorized inputs | Inspect frozen prompts/results, exact request recovery; no input sharing or duplicate provider entry. |
| S03 Future repair | E01; reconciliation source | Control proposes, approver decides where needed, runtime adopts prospectively | Compare past/current/proposed layers; stale/refused plan leaves history intact. |
| S04 Missing model | E01 correction journey/source | Host selects permitted exact generation, no invisible fallback | Explain saved definition versus currently unavailable route; explicit repair or wait. |
| S05 Agent-authored work | Partial: construct/proposal routes | Untrusted bounded proposal becomes definition only after daemon validation and authority | Goal → reviewable structured candidate → permitted save/adopt; malformed or unavailable operation cannot execute. |
| S06 Advanced graph | Source; E07 shared exclusive continuation refused | Runtime owns isolation, join, reducer, bounded repeat and signals; R-C01 adds bounded same-scope reconvergence | Full graph/list authoring; branch failure/cancel tests and reconvergence proof prerequisite. |
| S07 Three identities | Role/identity source, E02 two owners | A/B own separate revisions/runs; C can serve without workflow engine | Simultaneous client sessions; same names never merge authority. Three-connection browser test required. |
| S08 Remote without method | E02 | A owns workflow/account, B serving acceptance/effect | Inspect each boundary; lost result retains uncertainty/allowance, not reassignment. |
| S09 Reuse/service | P1 source; E03 service | Local pin inherits basis; publication uses explicit service authority | Distinct actions and public/private views; remote readable revision not silently pinned locally. |
| S10 Service lifecycle | E03 plus constraint/retirement sources | Accepted generation/association retained while future admission changes | Invoke-only result/recovery remains scoped; revoked disclosure can deny reads without changing past acceptance. |
| S11 Authority transitions | Stream regression sources | Daemon filters before projection, binds cursors to authority/feed | Flush stale observations, reject late responses, reconnect under new context; no token sharing. |
| S12 Locality/installation | E02 direct; managed source/prior evidence | Invocation may end while installation/service persists | One discovery vocabulary with explicit invoke/install/attach/publish consequences. |
| S13 Uncertain effect | E02 unknown response; E03 boundary losses | Runtime/serving/resource owner retains effect uncertainty | Inspect or authorized resolution; timeout/abort/cancel acknowledgment is not safe retry. |
| S14 Protected effect | Agreement/managed verifier sources | Exact candidate/target/config/verifier evidence at effect entry | Show precise refusal, preserve failed check; no direct-call bypass. |
| S15 Evaluation | Learning source and prior inconclusive report | Frozen declaration/slots and separate authorized promotion | Evidence selection → candidate → comparison → explicit decision; missing measure stays inconclusive. |
| S16 Decisions/deadlines | E03 cancellation; control sources | Grants govern propose/approve/cancel independently; accepted work persists | Explain waiting versus cancellation request versus proven stop; approve after recheck. |
| S17 Evidence visibility | Artifact/history sources | Metadata/content have independent read checks; pages bounded | Denied/missing/expired/truncated/unknown/zero distinguished; no hidden identifiers in omissions. |
| S18 Networks | E04 negative only | Transport conveys authenticated commands; operator provisions connectivity/TLS | Selected browser path must prove JSON, streaming/resync, artifacts and two origins; desktop remains separate. |
| S19 Core migration | Current exact-version readers | Each schema owner governs current supported bytes, CLI/protocol peers | Drain/refuse or explicit conversion before new writer; rollback cannot reinterpret new bytes. |
| S20 Edit versus execute | P1 and public authority source | Read/propose/approve/invoke are separate operations | Visible read-only or opaque service view; UI hiding is no security guarantee. |
| S21 Nested resource use | Managed/publication source, E06 three focused tests passed | Lifetime hold protects generation; editing claim transfers only with quiescence | Child progress with one worker; maintenance waits/refuses with blocker; revocation/uncertainty never releases unsafe writer. |
| S22 Same names/replacement | Authority/SavedRunRequest and immutable authoring/store source | References include owner/auth context; authoring checks exact envelope/base, not current workflow-head CAS | Same-parent valid revisions may diverge; explicit selection/comparison. Quarantine on identity change, never silent retarget. |
| S23 Live notation | Reconciliation source; proposed notation | Old occurrences stay on governing revision; future adoption validates dependencies | Show join/signal change refusal or prospective adoption; layout cannot prove validity. |
| S24 Private lesson transfer | Context/serving source, partial combination | Read, context use, transfer and public result disclosure are distinct grants | Review exact selected bytes/destination; denied export is not absence. Combined privacy test needed. |
| S25 Drift and release | Managed source; no detach action | Managed owner retains configuration/holds; external attachment has other owner | Inspect drift and safe supported remove/preserve. Release-to-operator is conditional B2, no invented button. |
| S26 Continuous work | E01/E02 and controller/managed sources; partial whole | Revisions/children inherit relevant authority/accounts; agreements unchanged | Repair, develop tool, reuse, wait and resume without reconstructing history; integrated acceptance remains required. |
| S27 Client versus work lifetime | Control-client/source; F1 proposed browser custody | Client abort/logout/removal stop observation/caches; explicit backend cancel changes intent | Persist exact request through explicit personal-profile storage or export before send; restore only under verified original binding. Unsubmitted session drafts may be lost. |
| S28 Iterative project | Components exist; whole workload unproven | Coordinator/reviewer/implementer are granted actors using artifacts/proposals | Preserve competing analysis, changed premise and failed verification; changed protected agreement explicitly escalates. |
| S29 Round trip | Source construct/layout; proposed mapping | Daemon defines semantics and digest; view owns layout only | Diagram/list/agent input/save/reopen equivalence, unknown construct refusal; no silent stripping. |
| S30 Upgrade | Source readers; future plan | Supported stored definitions, receipts, active obligations retain one owner | Version negotiation and supported upgrade/refusal with safe rollback; no second permanent semantics. |

## Deep traces and root causes

The [trial](review-method-trial.md) follows reuse in both directions from user need to accepted
identity, tests neighbor meanings and applies permission/role variations. [Behavior evidence](behavior-evidence.md)
traces direct acceptance through preparation, durable entry, external observation, uncertainty and
recovery; it separately traces publication's public invocation/internal child and resource claims.
Architecture/notation/interface documents extend these into S21/S26/S28 rather than repeating them.

Confirmed mismatch E04 comes from a native headless route being treated as sufficient browser
infrastructure. It is not proof that bearer auth or a loopback listener is wrong. Historic stream
corrections independently show why append-only client catalogues and cross-incarnation cursors
are invalid: `3059745`, `4e3c1db`, `e3adddf`, `18db4b2` changed those exact boundaries.
Source type-only review also nearly invented a layout-digest gap; the complete receipt consumer
disproved it. Proposed recurrence check: every promised UI action traces through the public owner
and one allowed and one refused/interrupted route, with source versus observed support marked.

Bounds are not automatically requirements: graph/document maxima belong to validated domains,
worker capacity to deployment configuration, per-call allowance to accepted authority, held-out
slots to a frozen evaluation declaration, and browser rendering/cache budgets to measured client
limits. The learning comparison's 4,096-event bound is a real finite scope, not a universal learning
law. Do not remove limits or make them all configurable by slogan; each extension must preserve
truthful overflow and its owning evidence.
