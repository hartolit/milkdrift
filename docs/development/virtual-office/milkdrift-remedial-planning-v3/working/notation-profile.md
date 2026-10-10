# Production notation proposal

Profile revision NP2, 2026-10-10, authored by Ada after actual Bram/Cyra critique and AR1. Selection and dissent belong to
[N1](../../whiteboard/discussions/implementation/executable-notation.md); this file specifies its
selected planning profile, not a competing decision. Requires [G1/r1 and P1/r2](vision-baseline.md), the product
comparison and the owner/browser design. Implementation remains unauthorized in this planning sprint.

## Foundation and exclusions

Use a UML Activity-grounded visual grammar over Milkdrift's daemon-owned definition and accepted
execution model. Keep task/data/decision/parallel/call relationships legible. Attach explicit
annotations where the product's durable, scoped behavior is narrower or different. The exact
[UML 2.5.1 and BPMN 2.0.2 clauses and PDF observations](standards-evidence.md) support the comparison;
they do not establish conformance, interchange support or model proficiency.

The full BPMN-executable alternative earns a serious comparison through cross-owner communication
and event vocabulary; it loses as the immediate proposal because it requires a complete activation,
data, cancellation and migration contract without an established interchange need. Replacing the
definition with canonical structured regions remains a serious alternative if the selected complete
editing operations still duplicate graph structure or cannot express S28. Retaining current
behavior with arbitrary box-and-arrow decoration is insufficient because familiar symbols can
misstate control and because the convergence probe found a real authoring limitation.

There is one semantic owner. The graph, accessible outline, instruction-assisted proposal and
saved machine document must submit ordinary daemon operations and inspect its accepted result.
The browser cannot define a private compiler, mint semantic hashes or assign effect permission.
Layout, selection, disclosure toggles and unsaved edits do not alter an accepted revision.

## Visual and editing grammar

Every item has an accessible name, semantic kind, owning daemon, exact definition or occurrence
identity and status text. Color supplements text, icon and border/pattern differences. The compact
view may hide ports only when connection kind and inspectable schema remain unambiguous.
At every selectable zoom, choice and merge retain a visible `D`/`M` or `Choice`/`Merge` mark;
their accessible names retain kind and owning choice. At smaller overview scales, use an explicit
aggregate region with an omitted-detail count instead of two indistinguishable unlabeled diamonds.

| Construct | Production visual/editing proposal | Saved meaning and important refusal |
| --- | --- | --- |
| Capability task | Rounded action box; operation name; optional model/tool badge. Input/output ports have labels and schema details. | Existing Task plus capability requirement and explicit request/context. Refuse missing required input or unsupported schema. Text instructions do not replace structured operation fields. |
| Ordered choice | Decision diamond with an ordered condition list; default arm labeled. One “choice” editor manages its region. | Existing Branch order is port-key order; show that order explicitly. Changing a key that changes priority is a semantic edit. Do not infer disjoint predicates or randomly choose one. |
| Exclusive convergence | Separate merge diamond labeled with its owning choice; compact region view keeps the shared continuation visible. | Proposed paired ExclusiveMerge below. Until implemented, current daemon refusal is shown. Never draw this as an accepted existing construct. |
| Parallel branches | Fork bar with named isolated branch regions. “Add parallel work” edits all required structure as one reviewed operation. | Existing Fork ownership and separate scopes. Cross-branch writable sharing refuses unless an existing explicit supported boundary permits it. |
| Synchronization | Join bar with visible All, Any completion, First success, or Quorum k/n label and loser policy. | Existing Join; All refers to termination, not universal success. Active loser cancellation is a request with continuing obligations. A join does not select or produce a composed answer. |
| Result composition | Transformation action after the join; compact compositor can visually group the pair while exposing both settings and separate statuses. | Existing Reducer Collect, First in branch order, or named capability. This is a distinct result operation; missing required inputs cannot become an accepted result. Join satisfaction alone never paints the group successful. |
| Time wait | Labeled time-acceptance shape with duration/due time; inspector distinguishes definition from accepted timer. | Existing Wait. Persistence/restart determine wakeup; browser timers do not. No implied boundary interrupt or recurrence. |
| Signal wait | Event-acceptance shape with signal type, delivery mode and exact scope/correlation detail. | Existing SignalWait plus accepted delivery facts. One-shot and broadcast remain distinct. No generic broadcast glyph on a targeted request. |
| Bounded repeat | Structured region/call box labeled Repeat, body pin, condition timing and hard limits; unfold iterations from history separately. | Existing Repeat. Show limit disposition, including required decision. No arbitrary back-edge, hidden unbounded loop or automatic restart with fresh allowance. |
| Exact workflow call | Action with a call marker and exact local-owner revision; expansion opens a linked view, not copied nodes. | Existing Subworkflow. Revision must exist at the selected owner. Different owner requires an explicit supported transfer or service call. |
| Public service call | Operation action labeled public service, owner and generation; public contract/result view is complete without internals. Authorized internal navigation retains a “public invocation” breadcrumb. | Existing Task/capability invocation and service owner. Internal expansion appears only if actual additional reads are authorized; losing those rights collapses/clears the private view. Read access never converts it to an editable child. |
| End/result | Explicit End action labeled success, failure or cancelled; connect to result ports where required. | Existing Terminal. It reports scoped workflow outcome. Avoid a standard termination glyph implying all external effects ceased before source/evidence establish that promise. |
| Control connection | Solid directed line between declared control ports, labeled chosen route where applicable. | Explicit control dependency; no cross-owner control edge masquerading as a distributed transaction. |
| Data connection | Typed port-to-port arrow with data label and distinct accessible connection kind. | Exact binding plus corresponding data dependency where required. Control readiness remains explicit. No implied artifact copying, publication or disclosure. |
| Inter-owner relationship | Labeled relation connector in a federation view, visually distinct from a definition edge. | Projection of explicit capability/service invocation and owner-qualified evidence. A browser connection is not peer trust. |
| Authority and resources | Inspector/action summary fields and bounded annotations. Optional partition view states its dimension. | A lane or group is not a grant, resource owner or placement command. Movement across it cannot relocate accepted work. |

Normal authoring supports keyboard insertion, selection, connection by named endpoint, condition
reordering, region editing, validation and exact revision comparison. The outline exposes the same
constructs, policies, ports and diagnostics in reading order; every graph-only action has a form
or table equivalent. Focus returns to the affected item after validation; invalid unsaved input
remains editable. Large graphs use collapsed scopes and windowing with discoverable omitted counts,
not silently absent work. A screen reader can distinguish unavailable, forbidden, pending and failed.

## Owned remedy for exclusive convergence

The [observed construction refusal](merge-construction-evidence.md) blocks a simple meaningful
journey: choose one approach, obtain its result and continue with shared review. N1 proposes one
bounded same-scope choice region, authored as a paired Branch and ExclusiveMerge. The proposed
name describes the runtime meaning; the UI need not ask the operator to manually pair enum values.
This is a planned extension, not current source behavior or unrestricted UML merge execution.

The definition contract must establish:

- One owning Branch identity and one merge in the same workflow/scope. Every normal arm has a
  statically validated path to that merge. Alternative interiors do not overlap or admit external
  control entry; nested choices close before their containing merge. Deliberate failure/cancel
  exits terminate without manufacturing the shared continuation.
- The merge accepts only the selected arm of that exact decision occurrence and definition epoch.
  It continues once after that arm's successful normal exit with its required declared results.
  A normal exit is the selected path's arrival at the merge, not a Terminal that closes its scope.
  Unselected arms have no occurrences or completion obligation. Parallel Fork/Join work may be
  nested inside an arm with its own join/result rules; concurrent alternatives cannot feed this
  merge as though they were exclusive.
- Each declared merged output has one exact binding per normal arm, with compatible schema and
  requiredness. Runtime resolves and forwards only the selected arm's value with original
  provenance; it does not combine, copy private bytes, or wait for values from unselected arms.
  Absent required output fails/refuses through the owning boundary. Output selection and control
  readiness must be one validated construct, not a browser-only conditional expression.
- Data from an alternative interior that is consumed after reconvergence must leave through a
  declared selected merge output. A direct data edge bypassing the merge refuses with its exact
  edge/binding diagnostic; otherwise an inactive-arm dependency could strand an apparently ready
  continuation. Common-ancestor inputs may feed alternatives without a new scope. Optional merged
  outputs explicitly represent selected-arm absence, and only consumers accepting that optional
  contract may proceed without a value. Nested choices forward through their own declared output
  boundary before the enclosing one. This is a data-escape rule for the new region, not a claim
  that same-scope choice creates new context privacy or authority.
- The region creates no child run, service authority, workspace scope or extra mutable branch.
  Its ordered choice, output selectors and pairing are daemon-owned semantic content. “Wrap in a
  reusable child” remains a different explicit operation with its own identity/context boundary.
- Retained decision/merge occurrence evidence determines restart and replay. Selected-result
  provenance, merge occurrence and continuation eligibility become one atomic retained transition
  in their owning runtime/persistence boundary; lost commit replies and restart cannot expose half
  of that transition. Identity includes the selected decision occurrence and its governing
  revision/epoch, plus any accepted prospective adoption, so another iteration or old epoch cannot
  supply stale output. A restart cannot choose the other arm or emit a second merged result.
  Cancellation, revocation and uncertainty remain ordinary runtime obligations. If a nested
  FirstSuccess/Quorum join permits the selected arm's result-producing path to reach the merge
  while a losing physical writer is uncertain, the merge may forward that result and enable
  continuation once. Actual reviewer entry still requires remaining authorized allowance after
  the unknown loser reservation and every ordinary entry condition. It neither waits for
  unrelated loser quiescence as a new rule nor releases
  any outstanding reservation, editing claim or resource lifetime hold. Subsequent protected
  entry still performs its normal checks.
- Prospective reconciliation classifies region edits: before selection, ordinary validated future
  replacement is eligible. After selection, the recorded route and accepted work remain fixed,
  while compatible unentered future work inside that same chosen arm remains repairable through
  normal reconciliation. Required positive case: A1 has accepted output, A2 is pending, and an
  authorized proposal replaces A2 with A2r while preserving consumed inputs, governing obligations
  and the declared merged output schema/requiredness. Adoption records the new future binding and
  its relation to the retained selected occurrence; it does not make a new choice. Reparenting an
  active region, switching its selected arm, weakening its output contract or rewriting consumed
  inputs refuses. Completed-but-unconsumed outputs remain their exact accepted values; consumed
  outputs cannot be rewritten. The implementation must define the compatibility check and prove
  the positive repair, not blanket-pin the entire chosen arm or say only “safe edit.”

This paired representation costs one executable construct and validator rules, but its purpose
is an ordinary choice outcome rather than a new user concept. A canonical structured-choice node
is the strongest replacement alternative: it could eliminate exposed pairing and invalid halves,
but must show how nested content, mutable future regions, existing mutation commands and exact
history map without introducing a second representation owner. The implementation prompt should
compare that representation at the blueprint boundary before final API design, while retaining
the behavioral contract above. This is a finite representation decision, not permission to replace
the entire scheduler or defer shared continuation indefinitely.

## Proposed transition examples

These are modeled acceptance oracles written before implementation. The new merge rows are not
claims of executed runtime support. `A`, `B` and `C` below are labels, not cross-owner identities.

| Case and accepted starting facts | Required next transition | Visible interpretation/refusal |
| --- | --- | --- |
| Choice selects A; B is unselected; A yields artifact x. | Proposed merge forwards x once; shared C becomes eligible. | B is “not selected,” never “failed” or “completed.” C's data provenance names A. |
| Selected A reports failure; B could have succeeded. | Preserve A failure; no implicit B execution or merged success. | Retry/repair needs an authorized prospective action. Choice is not first-success racing. |
| Choice has both guards true. | Current first-in-port-order arm wins; selection is recorded. | Condition order visible before submit; display-only movement changes nothing. |
| Parallel A fails first; B later succeeds. | Any completion may satisfy on A; First success waits for B; All waits for both terminal facts. | Rule and subsequent result acceptance are distinct. Unknown loser effects stay visible. |
| Quorum is reached while an external writer is active. | Join records satisfaction and requests loser cancellation; effect owner retains unresolved work. | UI says “cancellation requested,” not “writer stopped.” |
| B completes before A; reducer is First in branch order A,B. | If required inputs are ready, choose A by configured order. | Label forbids interpreting this as fastest answer. |
| A fails or supplies no required output; B succeeds; Any completion has satisfied. | Required input readiness still prevents a composed result; no substitution of B for the configured A binding. | Show join satisfied and composition pending/failed according to actual owner facts, never whole-group success. |
| Choice selects extraction; unselected audit arm has a required direct data wire to post-merge review. | Reject bypassing edge at definition validation; author maps a declared compatible merge output instead. | Identify the inactive dependency; optional output requires an explicit optional consumer contract. |
| Selected arm has nested quorum with a valid result and a physically uncertain losing writer. | Forward selected result/continue once if normal arm exit is eligible; retain loser account/claim/hold. | Reviewer entry also needs remaining authorized allowance and nonconflicting resource rights. Merge is no proof that the writer stopped. |
| Merge commits while its response is lost, then daemon restarts. | Recover the same selected value/provenance, occurrence and continuation eligibility together; no duplicate continuation. | Ignore stale outputs from earlier choice occurrences/epochs. |
| One-shot signal is accepted while another matching wait exists. | Deliver according to exact accepted recipient/matching contract; other wait does not become satisfied by inference. | Broadcast requires its separately accepted delivery mode and scope. |
| Child/service box expands after extra read authority, then that grant is lost. | Fetch an authorized linked view with public-invocation breadcrumb; clear/collapse it on observed loss of internal read rights. | Opaque public progress remains a supported complete view when inspection is denied; no gained edit right. |
| Chosen arm A1 completed, A2 pending; proposal replaces A2 with compatible A2r. | Record adoption inside retained chosen route; A2r can run and supply the unchanged declared merge contract. | Refuse changing chosen arm/schema or rewriting A1; repair is not forbidden merely because selection occurred. |
| A completed, B running, C pending; proposal replaces C with D. | Show proposal separately; only accepted adoption changes future eligibility. | A stays recorded under its old definition; B's unknown effects remain unknown. |
| Repeat reaches its hard ceiling. | Apply its defined fail/latest/decision disposition; do not schedule a fresh unaccounted iteration. | Inspector shows cause, consumed allowance and allowed next decision. |
| Layout moves a node across an owner partition. | Change layout only, or start an explicit separate placement/ownership command if offered. | Never change accepted owner, peer trust or grant from geometry. |

A visually plausible exclusive merge connected to concurrent fork branches is invalid. A data
line without the binding/dependency required by the daemon is invalid. A cancellation outcome is
not evidence of effect rollback. These are required negative cases for graph, outline and agent
authoring. Disconnected clients show the last observed sequence and connection status, without
inventing a global ordering across owners.
An exact client recovery record is private evidence of what was sent, not authority over its
outcome. Current-authority inspection of a known run/invocation is separate from command replay.
Workflow command replay requires its original authority binding. Direct serving retains its own
original acceptance basis and can recover exact requests under current authorized disclosure;
the client never substitutes a new basis. Missing or denied reads leave uncertainty intact.

## Machine authoring and round trips

This proposed choice example names the meanings before a schema is chosen. Solid arrows below
mean control; `x` labels the selected typed output. It is an intended post-remedy diagram, not
a currently accepted Milkdrift graph:

```text
                 choice C: ordered predicates
                        /             \
                 arm a /               \ default b
                      v                 v
              approach A            approach B
                 result x              result x
                       \               /
                        v             v
                    exclusive merge of C
                      selected result x
                              |
                              v
                      independent review
```

The conceptual authoring record fixes C's identity, ordered predicates, arm membership, common
continuation and output `x`'s exact schema plus one binding per arm. The daemon validates the
complete candidate and returns the authoritative revision. The runtime trace for selecting A is
decision occurrence C selects a → A starts → A's accepted output x → C's merge forwards that
same provenance → review becomes eligible. B has no occurrence. If A's result-producing path
fails or required result evidence is unknown, there is no merged success. A separately uncertain
nested loser retains its obligations without inventing a new merge wait. Restart continues from
retained C/A facts. This is the same
oracle for a paired-node mutation batch or a canonical choice-region request; neither may alter
it to simplify implementation. Until the new owner contract exists, the current public constructor
must continue to refuse direct reconvergence truthfully.

For an existing parallel form, the source-backed authoring counterpart is the
[fork/join/reducer kernel example](../../../../../crates/blueprint/tests/kernel.rs), including
separate control and data ports. The profile presents that saved meaning as:

```text
fork F → isolated A ─┐
       → isolated B ─┴→ join F: all terminal → compose: collect → review
```

Here the two branch outputs connect explicitly to composition's typed data input. The join's
control edge permits the next step but contributes no answer bytes. A failed branch is recorded;
data completeness and result acceptance decide whether composition/review can continue. Unfolding
the compound compositor into join plus reducer changes no stored semantic object or execution.
These diagrams deliberately do not claim that dashed/color variants alone carry the distinction.

Use public `ConstructBlueprint` with the existing `BlueprintDraft` mutation contract for advanced
definitions, and existing ordinary authoring conveniences where they apply. Agents receive exact
operation schemas and bounded examples from the owner; an unsupported construct returns an
actionable diagnostic. The same owning construction validates compound choice and compositor edits
for Svelte, CLI/API and agents. A future convenience must live behind this shared boundary.

Required equivalence is specific: graph or outline edit → daemon-validated semantic definition →
reload into either view preserves operation contracts, dependencies, exact pins, conditions, bounds,
authority references and defined outputs. Layout round trips preserve presentation separately.
Accept/restart/reload must identify the same accepted operation and history; rendering is not a
re-execution. Unknown supported future constructs are represented as truthful read-only content
until the client has a compatible editor; it must not strip them on save.

No full UML XMI or BPMN XML import/export is selected. If a future concrete task requires it,
choose a finite mapping, validate behavior with the same oracles and reject unknown semantics
before committing a definition. Exporting a picture is not executable interchange. A lossy
documentation export must clearly say which meaning it cannot preserve.

## Reproducible authoring and review exercise

This exercise is planned. No paid model access or quantitative independent model evaluation was
authorized or performed. The current agents' planning work is not a controlled performance trial.

Prepare three equal-information author packets: this activity-grounded profile, a selected BPMN
candidate, and the current typed-document explanation. Each receives the same capability contracts,
grants, limits, owner facts, allowed inputs and public validator access, without a solution graph.
Freeze packet versions and test expectations before generation. Use separate fresh author contexts
and separately fresh readers; rotate case order and conceal other candidates' answers.

Training cases: (1) supplied brief → generate → independent review; (2) two isolated approaches,
wait rule and explicit judge; (3) exact local reuse contrasted with private service and direct
remote process. Held-out variants: conflicting guards with ordered priority, failed fastest branch,
quorum with uncertain loser, denied internal inspection, revoked grant after selection, and
prospective repair while a selected branch is running. Include S28 with a fresh reviewer changing
one consequential premise and preserving earlier dissent/evidence.

Retain exact prompts, source/packet versions, model/tool identifiers, generation settings, produced
documents, validation diagnostics, correction attempts and reader predictions. Evaluate semantic
omissions, invented operations, forbidden disclosures/effects, repair steps and ability to predict
the modeled transition. Record positive permitted completion as well as refusals. A small exercise
can falsify a claimed mapping or reveal a usability problem; it cannot rank general model quality
or prove superiority statistically. Without approved model access, keep these claims unmeasured.

## Adoption and verification obligations

The future semantic-authoring prompt owns the entire choice boundary: blueprint model/validator,
canonical reader/version/fixtures, runtime occurrence/projection/restart/reconciliation, persistence
events where needed, protocol/client/CLI authoring and interpretation, accessible Svelte editor,
examples and evidence. Do not add the glyph ahead of daemon support. Exact version numbers come
from the owning constants at implementation time; new formats require explicit reader refusal and
coordinated upgrades. Preserve old bytes, accepted histories, receipts and pin behavior without
retroactive normalization. A format change requires its canonical decision and compatibility review.

Focused evidence must include allowed same-scope choice with forwarded results; nested choice;
inactive-arm missing data and bypass refusal; compatible optional output; conflicting guards;
failed/cancelled selected arm; parallel-fed merge refusal; external-entry/referrer refusal;
missing output; atomic selected-result/continuation recovery after lost commit and restart;
positive A1-completed/A2-pending repair and selected-route/schema refusal; nested join progress with
retained loser obligations; and positive governed repair without altering protected obligations. Validate
both graph/table authoring and direct public clients. Existing rich graphs and all ten NodeKinds
remain readable and operable. Full integrated/runtime acceptance stays in the program's named final
prompt; focused regressions accompany the owning change. No production gate is claimed here.

Known limits: normative/source comparison and construction refusal are established; new merge
semantics, production browser interactions, accessibility, round-trip equivalence, human comprehension
and model advantage still require the specified proof. NP2 incorporates actual criticism recorded
under N1. Rowan selected N1/r2 for planning after Bram/Cyra reread the changed clauses and Ada
reread the affected tasks; that decision grants no implementation authority or user approval.
