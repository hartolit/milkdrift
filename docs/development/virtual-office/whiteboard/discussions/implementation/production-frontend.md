# How should the production workbench connect, explain work and retain recovery?

## Current recommendation — F1, revision 3, selected for planning

Bram-20261010 recommends the intent-led production interface and engineering contract in
[F1/r3](../../../milkdrift-remedial-planning-v3/working/production-interface.md) and
[F2/r3](../../../milkdrift-remedial-planning-v3/working/frontend-engineering.md), with
[FP1](../../../milkdrift-remedial-planning-v3/deliverables/frontend-practice-proposal.md) for later
canonical adoption. This is a conditional technical planning recommendation under G1/P1/P2,
A1 and N1, not implementation authorization or actual-user approval. The review-facing
[interface brief](../../../milkdrift-remedial-planning-v3/deliverables/production-interface.md)
links to the build annotations and complete operation/state tables.

The retained app is `apps/workbench`: static SvelteKit 3/Svelte 5/TypeScript 6, Node 24.21.0 LTS,
npm 12.2.0 and a committed package lock. P01 verifies compatible exact library patches and the
real production build. Svelte Flow 1.6.3 is the conditional rendering choice with a complete
outline/form editing path. No Node runtime, mandatory local broker, private compiler or fake daemon
is selected. Library documentation does not establish qualified interoperability/accessibility.

Navigation combines Work, Operations, Resources and Improve under explicit owner context, with
P1/r2 common discovery and distinct committing actions. Direct routes remain available. F1 covers
authoring, actual results, controlled adaptation, restricted services, direct execution, resource
lifetimes and evaluated improvement; a two-step workflow is only an integration sequence.

Initial deployment is an operator/user-hosted static build with an approved daemon-origin set in
its CSP, plus each daemon's explicit CORS configuration and remote HTTPS termination. Adding an
origin outside that set requires an explicit operator setup change. A general public website with
arbitrary private endpoints is unqualified. Version/authority reads themselves use a bearer;
trusted origin/TLS governs credential delivery before stable host comparison can protect later
saved request/cache reuse.

The first private draft/effectful request requires a real device-retention choice, never inferred
trust. Explicit personal-profile mode keeps bounded private drafts/exact requests in IndexedDB,
**unencrypted at rest**; session/export mode keeps private drafts in memory and requires completed
exact export before sending effects. Both keep bearers in tab memory only. Logout erases credentials
and views, locks retained records and leaves accepted backend work running. Pending/unknown records
are not evicted. Initial 128-record/32-MiB recovery capacity reserves 16 records/4 MiB for control;
export or an authorized native control path remains visible when that capacity is full. These
limits are client policy to measure, not changes to backend obligation limits.

Changed grants quarantine records without silent rewriting. Workflow saved-start replay requires
its original authority binding. Direct serving recovery can use current Inspect plus the retained
original basis where the existing owner permits it. Known-object read and exact command replay
remain different; denied/missing reads do not prove an effect absent. Exact numbers, bytes, IDs and
guards must survive browser storage/export/replay without JavaScript numeric rounding.

## Strongest alternatives and unresolved preference

Object-led navigation may communicate authority better than common discovery. Plain Svelte+Vite
may suffice without Kit routing. Cytoscape may outperform the selected graph renderer on actual
large graphs. These are live implementation/human-checkpoint reversals, not cosmetic variants to
implement simultaneously without need.

For custody, the strongest alternative is session/export as the ordinary experience for shared,
ephemeral or sensitive profiles. It reduces retained browser data but adds explicit file handling
before effectful actions and makes tab loss more costly for unsubmitted drafts. Persisting private
requests improves recovery but does not secure an unlocked profile or provide cross-device backup.
We cannot infer which friction the actual user prefers. P03 must exercise the choice, reload,
logout, quota and response-unknown states; a later encrypted/native custody design needs explicit
key/recovery requirements. Persistent login is not implied by persistent request records.

For deployment, same-origin fixed-target proxy is credible when users operate a small known host
set. It may be simpler than maintaining frontend CSP plus daemon origins, at the cost of per-host
deployment or a carefully scoped target gateway. Reverse direct CORS if actual supported browser
topology or operator use disproves its benefit. Do not introduce an arbitrary URL-forwarding proxy
as a hidden workaround.

## Actual contributions and revisions

2026-10-10 — Bram-20261010, `/root/intent_bram`: FE1 inspected real routes/readers/consumers and
current primary Svelte/browser/library documentation before the gate; F1/F2 were selected after
G1/P1. The layout handler inspection corrected an early false need for JS digest generation.
F1 explicitly assigns missing learning receipt discovery R-FE01 and verifies/finishes a maintained
goal-planning consumer R-FE02 instead of inventing browser semantics.

2026-10-10 — Cyra-20261010, `/root/systems_cyra`: independently challenged actual F2's exact CSP
against unconstrained Add connection and full protected recovery storage against useful cancellation.
F1/r2/F2/r2 now name the approved-endpoint deployment and reserved-control/export/native route.
Cyra also qualified pre-authentication bearer delivery to a replaced daemon at a trusted origin;
the revised text states that limit and adds no speculative identity scheme.

2026-10-10 — Ada-20261010, `/root/intent_ada`: challenged conflicting program storage, missing
R-FE assignments and changed-grant recovery. The program was corrected by Rowan. Ada and Cyra's
source refinement established direct serving's current-Inspect/stored-basis replay, and Bram
read `invoke_client` before correcting F2's operation-specific rule. This preserves an existing
useful route rather than imposing the stricter saved-workflow helper universally.

2026-10-10 — Bram independently challenged NP1's low-zoom diamonds, misleading compound success
and insufficiently specified selected-arm future repair. Ada's NP2 adds D/M or explicit labels,
distinct synchronization/result states, a failed/missing-output case, persistent public-invocation
context and A1-accepted/A2-pending→A2r positive repair. Bram reread the actual clauses and accepts
them as a planning contract, while their executable and usability proof remains future work.

2026-10-10 — Rowan-20261010, coordinator `/root`: accepted F1/F2's conditional technical direction
after cross-review and requested actual dependent prompt rechecks. The corrected P01 names exact
integer handling, CSP topology and reserved control recovery; P02 distinguishes workflow replay
and current reads; P04 contains the positive internal repair; P05 preserves qualified direct replay;
P07 owns R-FE01. Bram's [BR1 recheck](../../../milkdrift-remedial-planning-v3/working/review-first-bram.md)
records the inspected result and evidence limits. No human interaction or production acceptance is
claimed by reviewer agreement.

2026-10-10 — Bram's final BR3 source trace found that immutable authoring compares envelope/base,
then stores a valid revision; it does not guard one mutable workflow head. F1/r3 and F2/r3 correct
the earlier concurrent-save claim and show valid same-parent siblings as divergent accepted
revisions. Rowan independently inspected the handlers/store and corrected P02; Bram actually
reread its positive sibling-save and real mismatch refusal requirements. Cyra aligns A1/r3.
This is a current-behavior correction, not a new shared-head feature or a runtime test result.

## Reopening conditions

Reopen F1/F2 if P03 users predict wrong rights or lifetimes, cannot operate custody/setup coherently,
or need unsupported goal/advanced paths. Reopen P2/N1 if graph/outline/API round trips require a
second compiler or ordinary selected-arm repair is unintelligible. P08 must combine resources,
private service, another host, repair, uncertainty and negative learning in real work. A polished
single-owner happy path cannot close those questions. Actual support is limited to executed browser,
topology and accessibility evidence; the current E04 is a negative browser probe only.
