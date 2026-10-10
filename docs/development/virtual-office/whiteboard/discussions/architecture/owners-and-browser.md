# Which owners and browser route should the independent client use?

## Current technical selection — A1, revision 3, selected for planning

Cyra-20261010 proposes retaining definition/runtime, serving-invocation and managed-resource
owners while adding bounded public discovery/authorized relationship reads and an explicit browser
transport. This is the selected planning recommendation under [G1/r1](../vision/remedial-purpose.md) and
[P1/r2](../product-model/reuse-service-and-placement.md), revised after actual 05/06 responses and
the 07 challenge and actual dependent rereads, selected by Rowan-20261010. It awaits user decision. No
implementation or canonical architecture change is authorized by this topic.

The initial browser candidate is an operator/user-hosted static Svelte client with an approved
daemon endpoint set, per-connection bearer fetch, and strict opt-in exact-origin daemon CORS.
Remote deployment uses an operator TLS proxy to the daemon's
loopback listener. It requires successful browser acceptance; current E04 proves the unchanged
cross-origin path fails. Same-origin fixed-target proxying is the strongest smaller alternative
where users own the hosting. A broker or native wrapper has no presumed role in first delivery.

The [architecture proposal](../../../milkdrift-remedial-planning-v3/working/architecture-and-federation.md)
owns the detailed responsibility/lifetime matrix, S21/S22/S26/S27 traces, browser constraints,
R-A01–R-A07 source-owner remedies and migration/rollback analysis. The
[behavior dossier](../../../milkdrift-remedial-planning-v3/working/behavior-evidence.md) and
[executed observations](../../../milkdrift-remedial-planning-v3/working/observations.md) separate
source support from actual probes. The review-facing
[execution and federation proposal](../../../milkdrift-remedial-planning-v3/deliverables/execution-and-federation.md)
explains the decision without requiring a source-code tour.

## Reasons and alternatives

Local runtime history and remote serving history describe different accepted facts; publication
already orchestrates through those owners without a third ledger. Managed editing additionally
needs physical stop evidence and retained lifetime holds. A uniform scheduler/ledger might reduce
query shapes but must replace all these atomic/recovery boundaries coherently. Current evidence
does not justify that cost. Fewer modules or one visual call shape is not sufficient evidence.

The preferred correction gains independent multi-owner observation and missing inspection paths
without making a tab an authority. It retains presentation complexity: users still need to know
what they can inspect, cancel or edit. Same-origin-only deployment avoids daemon CORS but narrows
independent hosting/multi-target convenience and does not repair resource discovery. Shared owner
migration might improve permanent-owner-outage recovery, but needs a demonstrated outcome plus
fencing, authority transfer, artifact and exact-request migration. It is not silently excluded
forever, nor implied by a remote location selector.

The source does not establish direct C→B relay through an execution-only C. Genuine workflow
delegation and direct client→B remain useful routes. R-A06 requires correcting misleading claims;
new relay needs concrete reachability demand and a full origin/authority/account/recovery design.
Likewise, a child-workflow workaround for exclusive branch merge adds semantic boundaries and
cannot substitute invisibly for the same-scope product choice in 03/05.

## Review, disagreement and reversal

2026-10-10 — Cyra-20261010, actual agent `/root/systems_cyra`: authored this candidate after
source investigation and the 02 gate. Proposed R-A01 transport, R-A03 managed discovery and
R-A04 qualified service relationship gaps; R-A05 remains a probe-before-remedy concern.

2026-10-10 — Bram-20261010, actual agent `/root/intent_bram`: independently proposed static
Svelte with fetch SSE and opt-in endpoint CORS; specified session-only in-memory tokens, bounded
private IndexedDB drafts/exact requests and no silent eviction of unknown submissions. Cyra agrees
these preserve owner separation. This coordination is not a human usability test or completed
joint endorsement of the frontend.

2026-10-10 — Ada-20261010, actual agent `/root/intent_ada`: requires owner-qualified accepted
observations and proposal overlays, with no global timeline or trust-from-connection. Cyra agrees;
ordinary child encapsulation must not disguise new context/authority boundaries for merge.

2026-10-10 — Joint review response: Bram read A1 and required actual selected-browser HTTPS
qualification before describing the default as a working independent multi-host client. He also
required a scan bound and truthful unknown remainder for filtered resource pages, plus removal
of cached internal relationship metadata on permission loss. Cyra incorporated those concrete
constraints into R-A03/R-A04. Direct-invocation history remains explicitly device-known because
the router currently supplies exact lookup without a global list; adding fresh-browser inventory
requires a demonstrated outcome. Ada read the owner/lifetime portion and added the distinction
between local recovery evidence and effect truth to NP1. Cyra's separate
[first review](../../../milkdrift-remedial-planning-v3/working/review-first-cyra.md) challenged
NP1 with a selected choice containing a satisfied join and unresolved physical loser, so agreement
on ownership is not treated as sufficient proof of the proposed merge.

2026-10-10 — Revision 2: Cyra challenged F2's exact CSP versus arbitrary Add connection promise
and recovery-store exhaustion versus usable cancellation. Bram's actual F1/F2/r2 reply specifies
the operator-hosted approved endpoint set and outside-set setup state; public arbitrary endpoints
remain unqualified. It reserves 16 records/4 MiB for control within the total 128/32 MiB recovery
budget, with explicit export or authorized native stop if storage still cannot retain a request.
Cyra reread those clauses and adopted them in A1/r2. The recommendation now names both daemon
CORS and static-host CSP operators, retains pending records, and exposes the narrower initial
deployment promise. Reversal remains meaningful if intended users require a general public-hosted
client. Current authenticated discovery also means origin/TLS trust precedes the host-ID check;
that check protects command/cache rebinding, not bearer confidentiality from a replaced trusted
origin. No new bootstrap protocol is silently added.

The same revision clarifies operation-specific grant-change recovery: workflow saved-start replay
requires exact authority, while direct serving can use current Inspect authority against the
retained original request basis. Neither path rebinds the original record or treats denied/missing
inspection as proof of no effect. This incorporates Ada/Bram's source correction instead of making
the native workflow helper a universal frontend rule.

2026-10-10 — Revision 3: Bram's final source recheck found that the S22 wording incorrectly
treated another author's save as a mutable-head conflict. Cyra traced `authoring::base/finish`,
`definitions::retain_revision` and `RedbStore::put_revision` and confirmed that the guard compares
the envelope with the immutable draft base; valid same-base siblings can both be saved. A1 S22
now preserves this supported ancestry, explicit revision selection and true base-mismatch refusal.
Rowan selected A1/r3 with F1/F2/r3 and corrected P02/P04/adoption descriptions. No new mutable-head
API or current behavior change follows from correcting the plan. Earlier review revisions remain
recorded as history; the final program recheck includes both sibling-save progress and real refusal.

Strongest unresolved objection: the preferred client may expose the complexity correctly but still
make users misjudge control, or demand more operator setup than a fixed same-origin deployment.
The first actual browser/human checkpoint must test this, not merely prove that fetch works.
Choose the same-origin alternative if it better satisfies the intended audience; reopen P1 if
common discovery causes persistent wrong commitments. Reopen ownership if a complete tested
alternative improves progress and recovery enough to justify accepted-state migration. These
changes invalidate downstream 06 deployment and 08 implementation assumptions; owner/product
changes also reopen 03/05. Current role-removal, opaque invocation and exact replay facts remain
in force until explicitly replaced and verified.
