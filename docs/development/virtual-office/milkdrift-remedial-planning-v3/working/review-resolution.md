# Combined review and resolution — RR1

Rowan-20261010 coordinates this record and authored the product synthesis/program; this is not
independent endorsement of that work. Separate first positions were recorded by
[Ada](review-first-ada.md), [Bram](review-first-bram.md) and [Cyra](review-first-cyra.md).
They read the combined candidate and primary sources before reading each other's first-review
files. Their exposure statements qualify independence: each authored a specialty; Bram had received
specific Ada/Cyra findings directly while finishing his initial review and attributes them rather
than claiming independent discovery. No reviewer represents a human usability study.

## Actual findings, responses and dependent rechecks

| Finding / affected value | Strong objection and actual change | Evidence and actual reread / remaining proof |
| --- | --- | --- |
| C07-01 / AR1-2, V12/S27 | Memory-only pending requests in B4/P02 contradicted reload recovery and F1. P01/P02/B4 now require explicit personal-profile IndexedDB or successful exact export before send, no stored bearer or unknown eviction. Storage failure prevents submission. | SavedRunRequest source requires pre-submit retention. Ada/Cyra reread the changed prompts and found the planning contradiction resolved. New storage/reload browser tests remain unexecuted. |
| AR1-3 / BR1-06, V11/V12/S11/S27 | “Current rights” is not a universal replay rule. P02 distinguishes exact-authority workflow replay, current-authority known-object inspection and unresolved lost acceptance. P05 preserves direct serving's different stored-basis/current-Inspect replay. No silent client rebinding. | Ada/Cyra checked saved_run and command fingerprints; Ada then checked `serving/direct.rs::invoke_client`, and Rowan read the actual branch. Direct lookup/replay is intentionally not generalized to workflows. F1/F2 and A1 must use operation-specific wording. |
| C07-02 / AR1-4, V02/V10/S05/S15 | Generic “fix missing APIs” left goal-planning consumer and study discovery unassigned. P04 now owns R-FE02, using existing consumer if sufficient; P07 owns R-FE01 bounded authorized receipt discovery, consumers and proof; program reverse audit updated. | Ada/Cyra/Bram reread actual revised tasks; assignment gap resolved. Route shape/query cost and whether a maintained generic planner already suffices remain execution source audits, not a new engine assumption. |
| AR1-1, V09/S06/S29 | Control merge alone can strand downstream work needing data from the unchosen arm. NP2/P04 now require every post-reconvergence alternative value through declared outputs, including optional absence/nesting; unconditional pre-choice values remain usable. | Ada traced runtime predecessor/data readiness and supplied audit_details bypass counterexample. Actual revised task reread supports resolution of the design gap. New static/runtime contract still needs implementation tests. |
| C07-03, V03/V07/V09/S21/S23 | Selected arm with satisfied FirstSuccess/quorum and entered uncertain loser must permit nonconflicting continuation while retaining physical holds/reservations. NP2/P04/P06/P09 now separate normal arm exit, run terminal and quiescence; selected result/provenance/occurrence/eligibility retained atomically. Entry still requires rights/remaining allowance. | Cyra independently added this combined variation and traced join completion, terminal deferral and managed claims. Ada responded in N1; Cyra reread NP2/P04/P06. No new merge exists yet, so this is a discriminating future oracle. |
| BR1-02, V03/S23/S28 | Freezing a whole selected arm would remove useful adaptation. NP2/P04 now expressly allow compatible A1 accepted/A2 pending → A2r, preserving selected route and output obligations; switch/consumed rewrite refuses. | Bram's actual challenge changed Ada's profile and Rowan's task. Positive and refused runtime/reconciliation cases required. |
| BR1-03 / C07-05, V09/S06/S29 | Any-completion satisfaction is not composite success or acceptable data; low-zoom choice/merge shapes can be ambiguous. NP2 separates join/result status and D/M text; P04 carries it. P00 must correct architecture's “any successful” drift. | Completion source distinguishes All/Any/FirstSuccess; Cyra/Rowan checked source and canonical sentence. Existing code is retained; no claim of a runtime defect. Human comprehension remains untested. |
| C07-04, V05/V11/S10/S11 | Public invocation can outlive private internal read rights. R-A04/F1/P05 now clear private relation IDs/titles and reject late session responses while preserving public progress. | Bram's architecture response originated the case, attributed by Cyra. Current relationship read is proposed; new grant-loss browser proof required. Previously exported material cannot be remotely erased. |
| BR1-01, V11/V12/S27/S30 | JS numeric rounding can change legal u64 guards and exact replay. F2/P01/P09 require qualified lossless parse/persist/export/serialize at 2^53 boundaries/u64::MAX, with meaningful conflicting-neighbor oracle. | Bram supplied a new large-guard/lost-reply/reload case. Contract fixture versus real-daemon scope must be reported separately. No wire schema change is inferred. |
| C07-06 / BR1-04, V01/V06/S18 | Exact CSP connect-src contradicts arbitrary Add connection. P01/decision brief choose operator-hosted static app and approved endpoint set, plus per-daemon CORS; generic public arbitrary-endpoint hosting remains unqualified. | Cyra raised the actual F2 contract clash; Bram agrees and revises frontend onboarding/practice. Real release CSP/HTTPS/browser tests remain acceptance. Same-origin fixed-target alternative stays live. |
| C07-07 / BR1-05, V11/V12/S16/S27 | Bounded recovery storage could obscure permitted cancellation. P01/F1/F2 reserve bounded control capacity and exact export/native control escape; no unknown eviction or unrecorded send. | Cyra raised the follow-up; Bram agreed. Test ordinary-store exhaustion with running work and legitimate stop, plus denied storage, before claiming useful recovery. |
| BR3 / PR-C4, V02/S22/S29 | The packet wrongly inferred a mutable latest-head CAS from authoring base guards. Source accepts valid immutable same-parent siblings. P02/P04/F1/F2/A1 now distinguish envelope/base mismatch from divergent saved revisions; explicit selection/comparison replaces a fictional stale-head refusal. | Bram traced authoring::base/finish → retain_revision → RedbStore::put_revision; Rowan read those exact owners, Cyra checked S22. A1/F1/F2 advance to r3 and dependent program text is reread. Add positive sibling-save plus actual mismatch refusal in P02; no new mutable-head API follows. |

Changes invalidate dependent readings until the affected text is actually reread. The review-first
files retain the reviewers' precise follow-up results; the final program read is recorded separately
below, not inferred from this table or a previous candidate endorsement.

## Actual debate and retained opposition

The early trial already changed P1/r1 to P1/r2 after Cyra exposed remote-pin, governed-copy and
role-removal qualifications; Ada and Bram actually exchanged arguments about discovery. P1 preserves
Bram's direct-route preference. Joint 04–06 review then exposed notation lifetime/data semantics,
public relationship disclosure and browser custody/deployment obligations. Authors revised their
contracts, rather than answering only “tests will cover it.”

The strongest current-design defense is real positive/refusal/restart evidence and one owner for
accepted facts; a rewrite must account for those benefits. The strongest challenge is canonical
structured regions, which could prevent malformed edits more naturally than paired nodes. Standard
foundation/interchange remains meaningful if desired, but an XML schema cannot establish execution
or replace authority/lifetime extensions. The selected bounded correction can be reversed by actual
compound authoring failures. No concept is retained solely because it has an ADR, and no difficult
outcome is deleted to make the proposal look simpler.

No loss is approved by a reviewer vote. Common discovery comprehension, operator setup burden,
private-record custody and alternate foundation tradeoffs go to the user. Missing transcripts,
new semantics and absent positive browser/human studies remain limitations even when planning
contradictions have been corrected.

## Root-cause lessons proposed for adoption

The actual mistakes were boundary mistakes: DTO shape without its receipt consumer suggested a
false layout-digest API gap; a generic recovery sentence erased different operation owners; control
flow without data escape or physical obligations underspecified reconvergence; native HTTP success
was insufficient browser evidence. Current source prose also compressed distinct join policies.

The narrow proposed practice is to trace each promised interaction through its public owner,
producer/consumer and one useful/denied/interrupted outcome, with source versus executed evidence
marked. New cross-client semantics require an independent meaningful oracle before fixtures mirror
implementation. A changed premise invalidates only its named downstream decisions until reread.
Do not add a universal approval committee or rule count; promote only the scoped frontend practice
and useful review guidance through P00 after user acceptance.

## Program fresh-read record

The final nonauthor read and separately labeled hypothetical premise variation are recorded in
[program review](program-review.md). Its actual findings and the affected rereads determine program
readiness. P00–P09 remain unexecuted and unassigned; human checkpoints and final gates are future
obligations, not certificates supplied by this review.
