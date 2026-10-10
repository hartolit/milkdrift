# Independent first review — Cyra, 2026-10-10

This first position was recorded before reading any other `review-first-*` file. Cyra is the
actual separate agent `/root/systems_cyra`. I authored A1 and the behavior/fresh-trial evidence,
so this is independent criticism of P2, NP1, F1 and Rowan's generated P00–P09 program, not an
independent endorsement of my own architecture. I read P2's current topic/product comparison,
the full NP1 profile, P00–P09 plus program README/context, and the production interface as it
appeared. F1's referenced frontend-engineering file was not yet present during the initial reads;
it needs a separate follow-up, not a presumed pass. No Cargo, new product probe or human study
was run for this review. Source remains the unchanged production baseline in the behavior dossier.

## Initial position and material findings

P2's complete authoring direction and NP1's same-scope selected-route contract have a credible
value argument. They are not ready merely because they align with A1. The generated program owns
most migration/refusal boundaries, but an actual client-storage contradiction and two public-route
assignment gaps need correction. The new merge also needs a compound oracle that distinguishes
control progress from settled effects. These findings are readiness changes, not cosmetic wording.

| Finding | Evidence and consequence | Required disposition |
| --- | --- | --- |
| **C07-01 — contradictory recovery prerequisite** | Program README B4 calls memory-only draft/request loss the default; P02 makes sensitive export necessary only when reload survival is requested and allows unexpected tab loss to lose the pending record. A1 and F1 instead select bounded retained exact records or explicit export before effectful submission, with memory-only bearers. P02 also promises client interruption/lost-start recovery. Those cannot all govern one implementation. | Reconcile B4/P01/P02 to the selected F1 storage choice and pre-submit recovery prerequisite, or explicitly reopen F1/A1 and narrow the promised recovery. This is not a request to persist tokens. Prove quota/refused-storage progress via deliberate export and refusal before effect, plus crash/reload exact recovery. |
| **C07-02 — new frontend remedies not explicitly assigned** | F1 names R-FE01 bounded learning-receipt discovery and R-FE02 maintained goal-planning consumer/template. P07 speaks broadly of bounded comparison pages but does not name the missing receipt query/owner; P04 covers structure but not the model-output/template/provenance route. General instructions to fix any missing API do not identify these complete boundaries. | Assign R-FE01 to P07 and R-FE02 to P04 or a named earlier whole slice, update dependencies/reverse audit, include exact authorized positive/refusal/restart proof. Investigate whether an existing maintained consumer suffices before adding new backend machinery. A fresh-browser study list or reusable goal planner must not silently become client-owned state. |
| **C07-03 — new merge needs composed lifetime oracle** | NP1 says selected arm successful completion forwards output; ordinary join may already satisfy while a loser remains entered/cancelling. If “arm complete” means every inner effect stopped, new merge loses legitimate continuation; if it means all obligations settled, it may incorrectly release holds/account reservations. Current source distinguishes these facts. | Add the variation below to NP1/P04 and P06/P09 integration. Define a normal arm exit independently of Terminal/run completion and physical stop; preserve unresolved descendants/holds. Atomically retain selected result/occurrence and continuation eligibility across a lost commit/restart. |
| **C07-04 — authorized link metadata is still private state** | R-A04/F1 allow internal expansion after independent read permission. A public invocation card may outlive later loss of that permission; a late old-session response can put the internal run ID/title back into a now-public view. The standard partition rule needs an explicit relationship-cache example. | Treat internal IDs, titles and relationship metadata as private, clear from current views/cache on grant change, reject late responses by session epoch and recheck before expansion. Previously exported material/browser history cannot be promised remotely erased. Test public-call progress while internal metadata disappears. |

C07-01 and C07-02 were sent to Rowan before writing this record. C07-03 was sent to Ada for
the actual revised NP1 contract; C07-04 derives from Bram's reply to the A1 review and is recorded
as a cross-review contribution, not an independently invented discovery. I will check actual
changed text before marking any finding resolved.

## New variation — selected choice, satisfied join, unresolved writer

Take proposed exclusive choice C at workflow owner A. It selects arm `a`; unselected arm `b`
has no occurrence. Within `a`, a Fork starts two permitted operations. One returns an accepted
successful result x. A `FirstSuccess` or quorum-one Join satisfies; the other operation has already
entered a managed physical writer and loses its stop report. The normal arm continuation forwards
x through C's proposed merge toward a read-only reviewer that needs no conflicting editing claim
and fits within the remaining authorized allowance after the unknown loser's reservation.
Between satisfaction and the merge result commit, interrupt/restart A. A maintenance request for
the writer's protected resource arrives from another authorized actor. Later the chosen arm has a
prospective unentered review task that an authorized repair proposes to replace.

The demanded positive behavior is one selected-result forwarding and one eligible shared review
after exact recovery, without executing `b`, resetting the account or requiring unrelated physical
work to stop before a nonconflicting continuation can become eligible. The replacement of the
unentered future reviewer should be possible if normal reconciliation proves compatibility and
governing restrictions allow it. The new merge must not freeze the whole chosen arm merely because
selection occurred. The resource's unrelated installations remain usable.

The demanded refusal is maintenance/editing transfer that relies only on Join/merge success or
cancellation acknowledgment while the writer's physical state is uncertain. Its lifetime hold
and unknown reservation persist after restart. A proposed change that switches chosen arm, rewrites
already-consumed x, redirects the entered writer or weakens the protected result contract refuses.
An authorized exact-use resolution may fence the writer/restore later resource progress without
inventing its old outcome. The outer workflow's terminal completion may still need to drain its
outstanding work even when the merge/control continuation is complete.

This is a modeled oracle for a proposed construct; no source currently implements ExclusiveMerge.
It changes a meaningful premise beyond isolated successful choice: an arm can reach its normal
control exit while its earlier parallel descendants retain real obligations. If P2 intends to
forbid that composition instead, it must state the lost expression and compare the alternative;
neither a collapsed graphic nor a hidden child run resolves the question.

## Primary-source grounding and existing focused evidence

- [Runtime join completion](../../../../../crates/runtime/src/engine/completion.rs),
  `RuntimeService::try_satisfy_join`, finds the governing fork occurrence in the same scope, records
  cancellation requests for active losers under Any/FirstSuccess/Quorum, then records
  `JoinSatisfied` and deterministic node completion. It does not wait for all losing external
  effects to become physically quiescent before recording satisfaction.
- [Structured progress](../../../../../crates/runtime/src/engine/structured.rs),
  `extend_structured_progress` and `drive_eligible_execution`, use bounded transitions and the
  execution's governing revision. This supports occurrence-qualified recovery and future
  eligibility; it is not evidence that arbitrary new merge events will be atomic automatically.
- [Terminal deferral test](../../../../../crates/runtime/tests/structured_runtime/lifecycle/cancellation.rs),
  `explicit_terminal_waits_for_an_already_dispatched_any_join_loser`, observes a completed Join and
  terminal node while the run remains Running with an active attempt and no `RunTerminal` event.
  Only release of the entered executor allows run completion. This guards against conflating
  visual end/control success with complete lifetime settlement.
- [Join policy tests](../../../../../crates/runtime/tests/structured_runtime/structured_graph.rs),
  `first_success_and_quorum_cancel_losers_without_dispatching_them` and
  `impossible_first_success_and_quorum_fail_deterministically_instead_of_deadlocking`, support
  legitimate policy progress and a negative path. Their non-entered losers do not alone cover the
  new physical-writer variation.
- [Managed use transactions](../../../../../adapters/redb-store/src/managed/uses.rs), `release`
  and `transfer`, require current exact claims/accepted lineage and quiescence; release also
  refuses unresolved child ownership. Return can refuse resuming a cancelled/unauthorized parent.
  These checks must still apply when a new merge has completed.
- [Managed publication tests](../../../../../crates/control/tests/control_service/published/managed.rs),
  `publication_hands_editing_to_exact_internal_writers_and_releases_every_hold` and
  `lost_child_stop_proof_survives_cancel_and_reopen_without_returning_editing`, give positive
  sequential child progress and retained-uncertainty evidence. The coordinator owns any executed
  results in [observations](observations.md); this review inspected assertions only.
- [Saved run recovery](../../../../../crates/control-client/src/saved_run.rs), `SavedRunRequest`
  and `submit_saved_run`, explicitly direct the native consumer to persist the complete bounded
  JSON privately before submission and require equality of current and saved `AuthorityRead`.
  A changed grant refuses the helper; frontend recovery cannot promise arbitrary current-grant
  replay by silently rebinding a saved request. Later recovery/operator authority must be explicit.

Useful existing focused filters for the coordinator, if new changes warrant execution, are
`cargo test --locked -p milkdrift-runtime --test structured_runtime --all-features explicit_terminal_waits_for_an_already_dispatched_any_join_loser`
and the two join-policy test names above under the same target. The new merge variation needs
a new meaningful integration case after implementation; re-running these old tests cannot qualify it.

## Alternative and migration judgment

The strongest challenge to P2 remains canonical structured regions: a complete Choice region
could prevent invalid halves and express selected outputs more naturally than paired nodes.
P04 appropriately compares those physical representations against the same behavior before
publishing the schema. If paired editing still requires each client to repair graph structure,
the intended simplification has failed. Full standard replacement remains a different decision
because its token, data, cancellation and history semantics require a complete mapping.

P00/context/P04/P09 make migration and one current writer explicit, including old definitions,
exact digest identities and active/uncertain obligations. That is the right scope. The proposed
merge adds a concrete migration pressure: readers must preserve old Branch semantics while new
definitions add continuation rules; historical old events must not be reinterpreted as evidence
of a merge that never existed. One current runtime may read historical forms without a second
writer/engine. Rollback requires actual reader compatibility and retained external obligations;
an old backup alone is insufficient. This remains an implementation acceptance obligation, not a
completed migration design or a reason to block all independent browser work.

Initial verdict: continue independent design work and correct C07-01/02 before labeling the
generated program ready. Incorporate or explicitly reject C07-03 with its value consequence;
carry C07-04 into the client/relationship boundary. No whole-system or human-usability pass follows
from this favorable direction on owners and syntax.

## Follow-up after the first position

This section follows the independently recorded position above. It may use actual replies and
newly available files; it does not recast them as evidence available before the first review.

**C07-05 — confirmed canonical Join wording drift.** Rowan identified possible ambiguity and
Cyra checked [architecture](../../../../architecture.md):313–314 against
`RuntimeService::try_satisfy_join`: the prose says “all, any successful, or a satisfiable quorum.”
The implementation distinguishes All terminal, Any completion, FirstSuccess success, and quorum
success count. NP1 already stated the distinction correctly. P00 must repair canonical wording;
no runtime change is justified by this prose error. The behavior dossier never equated Any with
FirstSuccess. The source/test trace above establishes the correct planning premise.

**C07-06 — initial deployment needs an identifiable policy owner.** Full F2 became available and
was read. It selects exact operator-configured CSP `connect-src` as well as daemon CORS. A user
entering a new daemon URL cannot connect if the static host's policy omits that origin, even when
the daemon allows the frontend. F2 acknowledges this but initial Add connection can still suggest
arbitrary endpoints. Sent to Bram/Rowan: name the initial operator-owned static hosting/approved
endpoint-set topology, show the outside-set setup state, and qualify general public-hosted
arbitrary-endpoint support. A broader CSP scheme allowance is a real alternative with a different
containment property, not an invisible fallback. Pending their concrete disposition.

**C07-07 — recovery storage pressure must preserve a stop route.** F2 requires every effectful
request, including cancellation, to retain/export exact recovery before send. With the protected
pending-record store full, the ordinary UI could block the very stop/cancel decision needed to
contain outstanding work. Sent to Bram/Rowan: define a bounded reserved control-recovery capacity
or explicit export/current-authority CLI route and user-visible consequence. No silent eviction of
unknown records, unrecorded cancellation retry or unlimited emergency queue is proposed. This is
a client progress design pressure, not a current daemon defect or an executed failure.

Full F2 strengthens exact numeric handling, session epochs, bounded records, real Rust fixture
ownership and graph/outline equivalence. Those are useful implementation contracts, not installed
dependency/browser qualification. The first reviewed package setup must still prove current
compatibility. C07-06/07 concern the resulting deployment/progress promise rather than style.

## Recheck of actual revisions

Cyra reread the changed P00/P01/P02/P04/P06/P07/program README, NP2's complete selected-route
clauses, and F1/F2/r2's policy/recovery sections after the authors' replies. This is a planning
readiness recheck, not execution of their future tests.

- C07-01 is resolved in the program: P01/P02 require a personal-profile retained choice or exact
  export before send; failed commitment prevents send, pending records cannot be evicted, and B4
  now concerns stronger custody rather than contradicting current selected recovery. Original
  grant workflow replay and separately authorized known-run inspection are distinct.
- C07-02 is assigned explicitly: P04 owns R-FE02 after testing existing maintained consumers;
  P07 owns R-FE01 through the existing receipt/read owner, with fresh-browser positive and private
  denial/pagination evidence. The reverse audit names both. Implementation still has to earn them.
- C07-03 is resolved in NP2 and P04/P06's planning oracle. Normal exit is not Terminal; a nested
  join may permit continuation while ordinary loser holds/reservations remain. Reviewer entry
  needs remaining authorized allowance. Selected provenance/occurrence/continuation commit together,
  and compatible unentered chosen-arm repair remains possible. NP2 also adds Ada's inactive-arm
  data-bypass refusal. I sent actual support for N1/r2 as a technical candidate, without a runtime
  or usability claim.
- C07-04 is carried into A1 R-A04 and F2's private metadata/session-epoch clearing. A later lost
  internal read must not leave an expandable ID/title in a public invocation card. This remains a
  required implementation test, with previously exported information outside remote-erasure claims.
- C07-05 is explicitly assigned to P00. The current canonical architecture wording has not been
  silently edited in this planning sprint; the source meaning governs the proposed profile.
- C07-06 is resolved as a narrower, named initial topology in F1/F2/r2 and P01: the user/operator
  hosts the static client with an approved endpoint set and matching CSP. Outside-set connection
  shows the required policy change. A generic public arbitrary-endpoint site remains unqualified.
  This is a real deployment consequence for user review, not proof of successful browser support.
- C07-07 has a concrete F2/r2 remedy and P01 prerequisite: reserve 16 records/4 MiB for control
  within 128/32 MiB total; ordinary use is capped at 112/28 MiB. Full reserve or unavailable storage
  offers exact export or an authorized native-client stop route. No unknown eviction or unrecorded
  automatic send is introduced. P03 must test the human consequence; no finite reserve guarantees
  infinite control requests.

After the actual Bram reply and reread, A1/r2 incorporates C07-06/07, bounded scanned pagination,
private relationship metadata and operation-specific direct serving recovery. Current direct
serving may compare a retained request against its stored original authorization with current
Inspect permission; the strict workflow saved-start equality is not generalized to every protocol.

An additional scope clarification was accepted by Bram: authenticated version/authority discovery
sends a bearer to the chosen trusted origin before learning/comparing durable host identity. The
host check protects later command/cache use, not the bearer against a replaced already-trusted
origin. A1/r2 states this without inventing a new public bootstrap/cryptographic identity scheme.
No remaining finding above requires another broad design round; final program review must verify
these corrected premises have survived in the deliverables and assignments.
