# Ada's first integrated challenge

Ada, 2026-10-10, review AR1. Production baseline is
`908e7893f5dadb84d12712573c8daaa946829e39`; planning HEAD inspected is
`9878f7a73cb7e1fe410a7d7bc16e126c9f979f7a`. This account was written before opening any
other `review-first-*` account. I am an N1 author and P2 contributor, so I am independent of
neither proposal. I read the current product alternatives, A1/r1 architecture, F1/r1 interface,
frontend evidence and every proposed implementation file P00–P09 plus its README/context.
`working/frontend-engineering.md` was not present at this read, although F1 and prompts link it;
I cannot certify its eventual storage, transport, toolchain or bounds decisions. I have received
Bram's and Cyra's actual direct NP1 critiques. Their low-zoom, partial-result, positive repair and
uncertain-loser cases will be attributed in my reply, not presented below as independent discoveries.
No Cargo, paid model call or production change was performed for this review.

## AR1-1 — A choice's data escape needs a rule as precise as its control exit

**High-impact premise:** N1/R-C01 and P04 establish one selected normal arm, merged output bindings
and a shared control continuation. They validate external control entry but do not explicitly
classify data edges that bypass the merge. Their otherwise useful same-scope design does not
create a child interface that automatically prevents such references.

**New countercase:** a choice selects an inexpensive extraction arm, producing `summary`; its
unselected forensic arm would produce `audit_details`. Both map `summary` into the merge. A
post-choice review task has control from the merge, `summary` from the merge, and a required
`audit_details` edge directly from the forensic arm because the author connected the visible port.
The merge can forward a correct summary while the review never becomes eligible. A happy-path
choice test with only mapped outputs does not discriminate this failure. A client that hides the
bypassing wire would make the diagram still more misleading.

**Source ground:** [predecessors_ready](../../../../../crates/runtime/src/engine/support.rs)
requires each required data edge to have a successful related-scope source with the named output;
control readiness alone is insufficient. [validate_binding](../../../../../crates/blueprint/src/validation.rs)
requires exact node-output bindings to carry their explicit data dependency. Existing validation
cannot establish correctness of a not-yet-existing choice region. This is a prospective contract
gap, not an executed defect claim about R-C01.

**Strongest defense:** selected merged outputs already imply the intended authoring route, and
ordinary required-data checks should prevent fabricated values. That protects integrity, but a
permanently unreachable legitimate continuation is still a failed product outcome.

**Required change:** make the new region's escape rule executable. Data used after reconvergence
from an alternative interior must pass through an explicitly declared selected merge output, or
be refused with the exact bypass diagnostic. Do not prohibit common-ancestor inputs merely because
they enter both alternatives. Define optional output behavior, nested-choice forwarding and stale
occurrence exclusion. Add this invalid bypass and a corrected explicit optional/required output
mapping to P04's independent oracles. The graph and outline must identify the actual rejected edge.
If a richer escape rule is preferred, prove its selected-route readiness and context semantics
before accepting it; do not leave it to the browser. This is a bounded clarification supporting I,
not evidence that R must replace the whole model.

## AR1-2 — Recovery storage has two incompatible selected stories

A1's browser contract and F1's first-private-work interaction offer bounded device drafts/exact
requests with an explicit retention setting and export fallback. The proposed program README B4
instead calls durable drafts/requests an unresolved branch and says the default deliberately loses
unexported content. P01/P02 mainly specify memory-only observations plus explicit export. These
statements cannot all be the same selected delivery contract.

**New countercase:** an operator selects device retention, submits a service maintenance request,
then the tab process dies after acceptance and before response. A P02 implementer following the
README could ship export-only recovery and still report completion. An A1 reviewer would reasonably
expect the exact request to reappear. Credentials can remain memory-only in both designs; grouping
them with private recovery bytes obscures the actual decision.

**Strongest defense:** both modes are safe if their limitations are explicit. Export-only is a
credible smaller custody boundary, while opt-in device records make real interruption recovery
more useful. The problem is conflicting promises, not that one mode is inherently correct.

**Required change:** choose one exact default/opt-in contract in F2, reconcile A1/F1 and P01/P02/B4,
and retain the rejected custody alternative honestly. If device retention is selected, require
write-before-send, quota failure, bounded non-eviction of unresolved records, late-result rejection,
schema upgrade/rollback and crash-reload with reauthentication. If export-only is selected, make
export-before-effect the recovery route where survival is promised; an advisory warning before
navigation does not cover process death. No new execution ledger is needed.

## AR1-3 — Changed-grant recovery must distinguish inspect, replay and unresolved absence

A1 correctly says current disclosure and old accepted authority differ. P02 promises reconnect
lookup/replay under current rights, while F1 points changed grants to an as-yet-unread F2. This is
not enough to specify a useful recovery path for a routine grant revision.

**New countercase:** actor `operator` starts a run under grant revision 7; the reply disappears;
an administrator changes that same actor's grant to revision 8, retaining run inspection but
removing start authority. The original host/run/request still exist and the user has the saved
record. They should be able to inspect a permitted accepted run without changing the request's
old authority or accidentally submitting a new start.

**Source ground:** [SavedRunRequest::submit_saved_run](../../../../../crates/control-client/src/saved_run.rs)
requires exact `AuthorityRead` equality. [command_fingerprint](../../../../../apps/daemon/src/host/receipts.rs)
binds actor, grant identity, revision, digest and canonical command; receipt lookup conflicts if
that digest changes. [HTTP routes](../../../../../apps/daemon/src/http.rs) expose exact run reads
and serving request lookup, but not a generic public command-receipt lookup. Changing the local
grant field cannot make old command replay equivalent. This source finding is stronger than
merely warning that authority might differ.

**Strongest defense:** a known run ID allows ordinary current-authority inspection; revoked rights
must not be bypassed for convenience. Correct. The app should use that narrower positive path.

**Required change:** specify separate actions: original-authority exact replay where still allowed;
current-authority inspect of a known run/invocation through its actual read route; preservation of
the unresolved record when accepted state cannot be proved. A forbidden or missing read does not
prove the command was never accepted. Define what a different authorized operator can actually
read, without claiming generic receipt recovery or automatically re-authorizing old bytes. Add
same-actor changed-grant and inspect-only recovery to P01/P02/P05 and composed P08 acceptance.
If a public recovery projection is needed beyond these current reads, assign its exact owner and
authorization rather than implying that quarantine implements it.

## AR1-4 — Two advertised outcomes need complete named delivery ownership

F1 names R-FE02 for goal-to-definition planning and R-FE01 for rediscovering old learning receipts.
P04 has no concrete R-FE02 paragraph; P07 describes learning and bounded pages but does not assign
R-FE01's discovery/read/archival contract. The README reverse audit omits both. A future author
could satisfy the named prompt literally while leaving F1's advertised starting points absent.

A fresh reviewer who has a goal and no hand-authored mutation document, or a fresh browser looking
for an old failed study without its `{actor, command}` reference, discriminates these gaps. Current
[LearningRequest](../../../../../crates/control/src/learning/request.rs) has exact Inspect, not
receipt discovery. Generic construction/proposal validation does not itself call a planner and
retain its malformed/declined candidate. This is not a demand for new generic engines.

The strongest defense is that implementation context links the full specifications. Nevertheless,
complete boundary assignments should name the new public owner work explicitly. Add R-FE02 to
P04 with maintained Rust consumer/template, exact structured output/provenance, invalid/refused
output, interruption and ordinary validation; retire it only after proving an existing path.
Add R-FE01 to P07 with bounded authorized receipt discovery, archive/restart scope, cursor/privacy
and fresh-session positive/denial cases. Update reverse mapping and state when these interactions
first become available, so P03 does not pretend to qualify them before implementation.

## Whole-program assessment and conditional disposition

| Reviewed assignment | Assessment after the findings above |
| --- | --- |
| P00 | Correctly requires approval and canonical adoption; must adopt the reconciled storage/recovery and N1 revisions, not candidate filenames alone. |
| P01 | Real origin/authentication/stream and independent-owner proof is meaningful; AR1-2/3 must become exact session/recovery tests. |
| P02 | Retained first useful run is substantive; crash survival and grant change must match selected promises. An existing denied read is not proof of absent work. |
| P03 | Actual human checkpoint can reopen backend/product premises. Its references to pin/copy/service must stay limited to delivered paths, as F1 already says. |
| P04 | Complete end-to-end choice responsibility is justified by observed E07; add data escape AR1-1, actual NP1 review corrections and R-FE02. |
| P05 | Explicit reuse/service/direct actions and opaque invocation are sound. Verify current-authority inspection after grant change separately from original replay. |
| P06 | Positive child progress plus protected holds is a useful compound requirement. Choice continuation must not erase a still-entered nested loser hold. Cyra raised that NP1 case directly. |
| P07 | Negative/inconclusive learning is preserved. Assign R-FE01 and exact public discovery before claiming a usable fresh-session Improve area. |
| P08 | Real combined work and fresh challenge are a strong discriminator. Include the new bypass and changed-grant variation; do not replace actual user participation with agent prediction. |
| P09 | Full final gate plus post-repair final state is appropriate. It cannot repair an undefined selected contract merely by accumulating passing tests; close these planning ambiguities first. |

I still prefer the bounded I proposal over direct exposure C because E07 demonstrates an actual
missing author outcome, while full R/S migration has no demonstrated necessity yet. I would reopen
R if the complete choice cannot be edited prospectively without repeated graph exceptions or
requires a second semantic representation owner. Standards-shaped symbols alone remain no evidence
of better authoring. A1's distinct owners survive this review; their browser/recovery promise needs
precision. F1's interaction structure is plausible, but I withhold any full frontend-engineering
endorsement until the actual F2 exists and these contracts agree.

This is a conditional design review, not acceptance of implemented behavior. The existing merge
construction probe is executed evidence; the new countercases are independently predicted oracles.
My direct NP1 replies and resulting revisions will be recorded separately after this first account.

## Follow-up clarification — Ada, 2026-10-10

AR1-3 requires no automatic grant rebinding. A saved StartRun already contains `run_id`, so after
an explicit current-owner/current-authority check, the client may offer ordinary `GET /v1/runs/{run}`
and permitted result/timeline reads under the new grant. They show accepted run facts if authorized;
they do not recover the old command receipt or enable replay. For serving calls, use the existing
current-authority `GET /v1/invocation-requests/{request}` lookup and, when it returns a permitted
execution identity, the exact invocation/observation reads. A known execution ID may be read
only under those current rights. If lookup cannot disclose or resolve the accepted identity, retain
unknown state and the saved original request. A changed actor/grant cannot turn the saved
request into a new allowed submission. Missing/denied reads do not prove absence, and a different
operator gains only the reads their present grant authorizes. This path should be named in the
implementation tests instead of the broader phrase “recover under current rights.”

## Source qualification after Cyra's reply — Ada, 2026-10-10

Cyra correctly challenged applying the saved-workflow rule to all operation families. I then
read [invoke_client](../../../../../crates/capability-host/src/serving/direct.rs): for an existing
actor/request record, it checks current `Inspect`, retrieves the original client authorization
basis from the retained hot/archived record, binds the exact submission to that basis and compares
its digest. Direct serving can therefore permit exact replay after a grant change when its current
disclosure checks pass. The browser must not substitute new authorization, but must also not impose
the workflow helper's exact-authority equality as a universal restriction that removes this working
route. The workflow command fingerprint/equality finding stands. P05/F2 should describe this
operation-specific direct recovery alongside exact known-run inspection and truthful unresolved
state; current permission can still refuse either read or retained response.

## Actual focused recheck — Ada, 2026-10-10

After writing AR1 and its source refinement, I read Cyra's C07 and Bram's BR1 accounts, full newly
available F2/r1, the revised P01/P02/P04/P05/P07 and program README, and the review-facing
`product-model-and-value.md` and `decision-brief.md`. I did not infer resolution from a message
that editing was complete; the following dispositions are based on those revised contents.

| Finding | Actual changed contract and disposition |
| --- | --- |
| AR1-1 data escape | NP2 declares selected-output escape, optional absence, nested forwarding and common-ancestor allowance. P04 now owns the inactive audit-details bypass and corrected graph through both selections. Resolved for planning; requires future validator/runtime tests. I requested final P04 wording also name optional/nested cases explicitly so “required” does not suggest optional bypass is an alternative rule. |
| AR1-2 custody contradiction | F2, P01/P02 and README B4 now choose explicit personal-profile IndexedDB or export-before-send, with no saved bearer. Failed custody prevents sending; unknown records cannot be evicted. P01 adds reserved control capacity/export/native escape for a full store. Stronger custody remains B4. Resolved for planning; actual storage/browser behavior is unqualified. |
| AR1-3 changed grant | P02 separates original-binding workflow replay from current permitted known-object inspection and preserves unresolved records. P05 explicitly preserves the actual serving owner’s permitted changed-grant replay and refuses client rebinding. Resolved at program-contract level with operation-specific tests; I requested removal of the old broad reconnect sentence and an explicit inspect-only positive test. F2/A1 must retain the same distinction in their final reconciliation. |
| AR1-4 missing assignments | P04 explicitly owns maintained R-FE02, first checks an existing suitable consumer, and tests malformed/unsupported/denied/stale/interrupted output. P07 explicitly owns R-FE01’s bounded authorized discovery, consumers, mixed visibility/restart and exact detail linkage. README maps both. Resolved for planning; no claim either path exists yet. |

P04 now also requires the exact A1-accepted/A2-unentered → A2r positive repair, preserved merge
schema and consumed evidence, atomic selected result/provenance/occurrence/eligibility, nested-loser
holds and remaining authorized allowance. This answers the actual Bram/Cyra objections. It does
not settle the physical paired-node versus canonical-choice schema; that comparison remains a
bounded early implementation decision with migration obligations. A failed positive repair must
reopen P2/N1, not be called a passing refusal test.

The new F2 is a coherent consumer specification: static client and scoped transport, lossless u64
boundary, session epochs rejecting late reads, exact private request custody, finite bounds,
real-daemon Rust fixture ownership, accessible outline and explicit CSP deployment. I reviewed
its product/authority consequences, not a resolved npm lock or independently measured package
compatibility. BR1's huge-number replay oracle, CSP-qualified Add connection and control-capacity
reserve are meaningful new checks and are present in revised P01. The initial custody choice is
substantive; no encrypted/shared-profile security is inferred. A current authority read must not
unlock old payloads merely to offer separately authorized known-object inspection.

The product/value account and decision brief preserve selected-source limitations, no intentional
outcome deletion, real E07 expression gap, source-backed current capabilities, conditional browser
route and full future program. They do not claim new merge/browser/model/usability acceptance.
The alternatives remain consequential: canonical regions if compound editing needs hidden
representations; standards execution for a demonstrated interchange need; object-led routes if
shared discovery misleads; fixed same-origin hosting if independent static hosting is too awkward.
These are appropriate reversal conditions, not invisible parallel implementations.

I support N1/r2 and the revised product direction as a planning recommendation after cross-review,
with Rowan owning selection. This is neither the user’s implementation approval nor a report of
executed merge, browser, accessibility, model-quality or migration proof. My source/data-boundary
criticism is now converted into explicit implementation responsibility rather than left as a
warning beside an unchanged plan. `git diff --check` passed on the current tracked diff; no Cargo
or application tests were run by this worker.

## Final bounded summary audit — Ada, 2026-10-10

At planning HEAD `9878f7a73cb7e1fe410a7d7bc16e126c9f979f7a`, I read the newly written
`deliverables/adoption-plan.md`, `deliverables/review-and-evidence.md` and the final P04 wording.
I cross-checked their E01–E07 counts/qualification against observations E2, the normative research
qualification against `standards-evidence.md`, and the reported “any successful” prose drift against
`docs/architecture.md` and `JoinPolicy`. I found no new standards-compliance, implementation,
interoperability, runtime-deadlock or model/usability overclaim. The summaries distinguish executed
construction refusal from proposed merge behavior and keep current-data/active-work transition
obligations owned. P04 now includes optional absence and nested forwarding in the escape boundary.

One narrow terminology correction was sent to Rowan: P04's sentence saying shared continuation
“eligibility” requires authorization/allowance should say **actual entry**. NP2 commits the merge's
control/data continuation eligibility while ordinary entry still checks current authority,
resources and remaining allowance. Calling both eligibility could mistakenly couple merge evidence
to downstream admission. This does not reopen the selected behavior; it aligns two descriptions of
it. No production edits, Cargo, new execution evidence or broad rereview were performed for this audit.
