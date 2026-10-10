# Early trial — Cyra's fresh non-author read

Read date: 2026-10-10. Source checkpoint:
`908e7893f5dadb84d12712573c8daaa946829e39`.

I first read [Ada's trial](trial-ada.md) and [Bram's trial](trial-bram.md), including their
cross-examination, after writing [the behavior evidence](behavior-evidence.md). I did not author
either trial or see their recommended interaction while investigating source. I am fresh to the
trial, but not independent of the implementation evidence that the coordinator also supplied.
This is an agent's reconstruction and source review. It is not human-participant usability
evidence, a browser experiment, an architecture approval or an executed product comparison.

## What the combined candidate actually recommends

The strongest combined candidate is **shared discovery followed by an explicit committing
action whose meaning persists afterward**. Search may find a saved definition, a public service
or an ordinary operation. Finding them together does not make them interchangeable. Before the
client sends a mutation, the selected action identifies:

- the actual owner and exact revision or capability generation;
- whether this creates a pinned child, a new editable lineage, or a public invocation;
- which inputs/results are promised and which inspection/editing rights are actually available;
- the accepted identity and original owner through which a lost reply can be recovered.

For an invoke-only caller, the client presents the service's public contract and declared results.
It does not promise that a collapsed call can expand into the internal graph. A readable workflow
can instead offer exact reuse or an independently editable copy when those operations are supported.
A directly invocable process on an execution-only host stays an ordinary operation. No durable
union object, implicit grant union or second execution ledger follows from the common chooser.

I can reconstruct that recommendation without assuming common discovery is easier. Ada narrowed
her recommendation after Bram's objection to a generic “Use” commitment; Bram accepts shared
discovery as a candidate when the actual committing actions remain distinct. Separate entry routes
remain a credible fallback and a possible fast route for frequent operators. They disagree about
navigation/comprehension costs, not about which owner may execute or reveal private work.

## Rejected alternative and strongest surviving objection

Universal publication would require every reusable workflow to become a callable service. Its
strongest benefit is one advertised contract and call lifecycle. It also adds configured service
authority, publication administration, a workflow-enabled service owner and recoverable public
invocation state to cases that currently need only a pinned local definition. Ordinary remote
process work does not acquire a reason to become a published workflow. The trial therefore rejects
universal publication as an unexamined default; it has not proved that every possible future
unification is impossible.

The strongest surviving objection is **false interchangeability at discovery time**. A common
result list may make two same-named items look equally inspectable, editable or locally owned
before the operator reaches the explicit committing action. Fixing the final button text is
insufficient if the selected result, accepted diagram item and recovery view later lose the owner
and action meaning. Repeatedly requiring operators to disambiguate already-known work may also be
more burdensome than separate routes. No current observation establishes which interaction is
better for humans; later production comparison must test this instead of teaching participants
the desired taxonomy and counting recall as success.

## Source grounding and qualifications to carry into 02

The core semantic comparison is grounded in the current implementation:

- `PinnedSubworkflow` and `runtime::engine::structured::subworkflow::ensure_child_created` bind an
  exact child revision/interface and inherited execution basis. There is no publication call in
  that creation path.
- `PublishedWorkflowService::prepare_publication` requires a governed revision and exact configured
  service grant. Public direct/peer acceptance retains one planned internal run association;
  runtime still owns its execution. Invoke-only disclosure is a tested contract, not an inference
  from a diagram.
- `DaemonConfig::validate` requires workflow mode plus controller accounting for local publication
  services. `Owner::open` constructs no workflow runtime in execution-only mode.
- `PeerService::invoke_client` and `accept_serving` preserve exact request replay independently of
  discovery. Current read permission still governs replay disclosure.

Three qualifications prevent the trial from turning a correct distinction into an overbroad
implementation premise:

1. **A readable definition at B is not automatically a pinned child at A.** The current child owner
   loads its revision from its own store. A multi-owner chooser must identify which workflow owner
   will compose it and whether a supported import/copy path is involved. Neither a UI connection
   nor a peer capability catalogue creates a remote editable revision reference. Treating all
   readable definitions as immediately pinnable is not justified by this trial.
2. **Permission is not the only copy prerequisite.** `host::commands::authoring::copy` refuses an
   independent copy with an identity-bound governing agreement. The permitted alternatives are
   exact reuse or separately authorized authoring of another governed method. A chooser must not
   promise “edit one use” by automatically copying every readable method.
3. **Execution-only is not a live conversion of active workflow work.** Role removal refuses
   outstanding workflow obligations. Closed history is preserved for offline inspection. The
   hypothetical role change must say whether the user chose another already execution-only owner
   or whether an operator is attempting a restart with a changed role. The second can refuse
   before there is any new execution-only session.

There is one additional open route ambiguity in the behavior evidence: current peer-mapped
adapters are absent from the direct invocation catalogue and require workflow provenance.
Therefore “execution-only host consumes a remote service” must not silently become “direct caller
uses execution-only C as a relay to B.” Direct client→B and a properly delegated workflow-origin
route are different candidate meanings. This qualification does not invalidate either trial's
ordinary direct-process case.

These are limitations to carry, not evidence that the shared-discovery candidate requires a new
backend registry. They also do not select whether cross-owner definition reuse or direct relaying
is a desired product outcome. That goal question remains with 02.

## Hypothetical dependency change A: the caller gains internal read permission

This is a deliberately hypothetical premise change. No live grant was altered.

| Dependent work to reopen | Exact change required | Fact that does not change |
| --- | --- | --- |
| 03, “Required deep comparisons”: reuse/service inspection | Add an authorized internal-view action to the service case; distinguish reading from authoring and from converting the call into a local child. Revisit whether the default discovery explanation remains clear. | The operation was accepted as a service invocation under its exact generation. Read access grants no edit/publish authority. |
| 04, “Model an independent client with several owners” | Refresh the session's authority-bound visible data and the link to the internal owner. Scope caches by host and current authority; do not infer permission from a prior catalogue item. | Host identity, service principal, child run owner, accepted request and resource/account obligations remain exact. |
| 05, “Establish meaning using concrete traces” and “Specify the production visual grammar” | Revisit expansion/inspection grammar: an authorized internal view can be shown as a view of existing work, with the service boundary retained. Do not rewrite the executable diagram into an editable local child. | Layout/expansion is presentation state; it cannot alter definition identity, accepted history or authority. |
| 06, “Specify surfaces as interactions with real owners” | Add or reveal the authorized inspect action; define loading, renewed denial and revocation while the view is open. Keep distinct disabled/unavailable/absent disclosures where the public API permits them. | The old call is still recovered through its original owner and exact request. A tab/fetch closing is not cancellation. |
| 04/06 public-operation mapping and later 08 assignments | Map the new view to the existing revision/run/artifact reads with current rights; identify any missing safe link from public invocation to permitted internals before claiming readiness. No new permission inference belongs in Svelte. | Replay does not reevaluate the old accepted operation into a different kind of call. Disclosure can still be refused under current rights. |

No final implementation assignment IDs exist yet. The table identifies exact phase responsibilities
to revise; 08 must propagate them into the named implementation prompts it eventually writes.
Inventing completed task IDs now would conceal the dependency rather than trace it.

## Hypothetical dependency change B: the selected target is execution-only

This is a different hypothetical, not a change to the source checkpoint or an authorized role edit.

| Dependent work to reopen | Exact change required | Fact that does not change |
| --- | --- | --- |
| 03 product meaning and alternatives | Split “compose a workflow on this owner” from “perform an authorized direct operation here.” Refuse local workflow composition/publication at the execution-only target or ask the user to select a real workflow owner. | Direct installed process/fresh-model calls remain meaningful without synthetic workflows. |
| 04 owner/connection and lifetime model | Establish which daemon owns the workflow and which performs the operation. If role removal is attempted, preserve/refuse active obligations before restart. Do not silently migrate work to another connected daemon. | A UI connection grants no peer relationship; A still needs explicit authority and connectivity to delegate to C. |
| 05 call/placement notation | Keep a direct operation or remote capability task's placement visible without showing a local child graph owned by C. Any notation implying C owns the workflow must change. | Accepted execution history retains its actual owner and origin; a collapsed visual form cannot create a service or runtime. |
| 06 discovery, target selection, unavailable states and reconnect | Remove or refuse ineligible composition/publication actions under the reported role. Keep allowed direct actions usable, with explicit host identity. Existing workflow references remain associated with their original owner even if that owner is unavailable. | Existing receipt/recovery identity is not rebound to whichever endpoint is currently reachable. |
| 04/06 public-operation mapping and later 08 assignments | Map direct discovery/prepare/invoke/lookup/output/cancel to the target's serving API; route workflow commands only to a selected workflow owner. Explicitly test the direct-relay ambiguity before advertising C→B service calls. | Authentication, exact generation, final authorization, current disclosure and permanent replay/conflict semantics remain in the existing owners. |

The changed premise affects action availability, ownership explanation, diagram annotation and
API routing. It does not justify new authority, a new workflow for a direct call, changed historical
origin, safe retry after possible entry or automatic reassignment after disconnect.

## Trial result and remaining gate consequence

The written cross-examination allowed this fresh reader to reconstruct the preferred candidate,
its rejected universal-publication baseline and its strongest remaining objection. The hypothetical
changes expose concrete dependent work in 03–06 and the later 08 program while leaving accepted
authority/history/replay facts intact. That is evidence that the review method can preserve a
changed premise and trace its consequences in prose.

It does not establish browser feasibility, human comprehension, a chosen notation, implementation
readiness or superior architecture. Before the 02 gate closes, preserve the three source
qualifications above and keep shared discovery's comprehension benefit explicitly conditional.
The two credible navigation alternatives can proceed to concrete later comparison without
blocking source investigation or endorsing a redesign here.
