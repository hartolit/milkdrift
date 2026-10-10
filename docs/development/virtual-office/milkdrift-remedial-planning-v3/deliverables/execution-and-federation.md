# Execution, federation and the browser client

**Proposal:** retain separate owners for workflow execution, accepted serving operations and
surviving managed installations because their accepted facts and lifetimes differ. Move managed-use
policy out of the redb mechanism, consolidate repeated serving admission and descriptor construction,
and add a control-owned commitment for bounded automatic ongoing work. Complete public inspection
and build the Svelte client against those shared routes. This is **A1/r5**, the reopened planning
selection by Faris-20261010 and Rowan-20261010 after independent execution/application reconstruction
and adversarial review. The user approved the continuous-work behavior in U18, not this architecture
or production changes. U19 further requires preserving legitimate graph-native editing and repair;
NP4 retains its versioned APIs alongside Program default authoring through shared owners.
The previous browser choice remains: explicitly configured cross-origin
bearer fetch, with an operator HTTPS proxy for remote deployment to the daemon's loopback listener.
The actual E04 probe failed authenticated cross-origin reads and the stream request; this is still
a proposed browser route requiring qualification.

The new [execution reconstruction](../working/execution-reconstruction.md) and
[application reconstruction](../working/application-reconstruction.md) supersede earlier blanket
retention claims and the earlier R-A05 interpretation. They compare actual current source, a
complete smaller correction and stronger ownership alternatives. Inspected tests support specific
recovery boundaries; they do not establish every package or the whole proposed browser/session journey.

## What connecting and running mean

A connection gives the client an authenticated view of one host. Its address and local label are
not its durable identity, and knowing two hosts does not connect them as peers or combine their
grants. The client checks protocol, stable host identity, actor and exact grant before exposing
cached data or submitting a saved command. Two hosts can both contain `release`; every reference
keeps its real owner and exact revision/generation. One failed connection leaves the others usable.

Definitions retain immutable ancestry. Two valid drafts from the same base may save as sibling
revisions; the UI must show and let the user select them explicitly. The current authoring guard
checks the submitted envelope against that draft's exact base, not a mutable latest-workflow head.
An actual mismatch refuses without discarding the draft. The proposal adds no hidden overwrite,
automatic rebase or new head-locking API.

Blueprint owns one typed source family: new convenience authoring defaults to Program, while the
existing explicit versioned GraphNative construct/genesis, import, copy and edit operations remain
supported. Each immutable revision has one authoritative source and lowers to one checked plan.
Graph-v3 keeps its complete bounded vocabulary; it does not acquire every new Program feature.
There is no age cutoff, eligibility registry or restriction to definitions present during upgrade.
Existing graph convenience behavior moves into shared blueprint helpers instead of disappearing
with the private daemon compiler. Graph, outline, agent and API clients use that same contract.

Verified conversion remains useful, but it is optional for a legitimate graph repair. Definition
equivalence and prospective adoption are separate checks: future-work changes can be intentional,
while accepted occurrences, scoped authority, agreements, history and unresolved effects retain
their exact constraints. A refused conversion leaves the supported graph edit/proposal path
available. An inactive reusable definition retains editing, copy and fresh invocation; a closed
historical run retains immutable evidence and does not become active through an edit.

| Action | What owns the accepted work | What the user can expect |
| --- | --- | --- |
| Invoke a process/model operation directly on B | B's serving record, exact capability generation and adapter entry evidence | No workflow is invented. Inspect public invocation/result, cancel when authorized, recover the exact request after a lost reply. |
| Run a workflow at A and delegate a task to B | A owns the workflow history/account; B owns the accepted operation/effect evidence | A broken connection does not transfer ownership or authorize a new target. Results cross the selected public boundary. |
| Use an exact reusable child | The current workflow owner resolves a saved local revision and records a real child with inherited authority/constraints | Later edits do not change an already accepted child. A revision merely readable on another host is not automatically pinnable here. |
| Make an independent copy | The accepting owner creates a new workflow identity with provenance | No run state or authority is copied. An identity-bound governing agreement can refuse copying. |
| Call a published service | Public invocation plus exact internal association; internal execution uses configured service authority | Invoke-only callers receive declared outputs without internal graph access. Retirement refuses new calls while preserving accepted ones. |
| Manage an installation | Its managed owner retains intent, generation, editing claims and lifetime holds | Installation life can exceed a call. Cancellation or a terminal run does not prove a physical writer stopped. |
| Continue bounded ongoing work | Control owns accepted goal/policy, next-work decisions and exact run associations; runtime owns the associated executions and persistence their cumulative account | Criticism can trigger reconsideration while target work is paused. New rounds retain prior evidence, authority bounds and consumed or uncertain allowance. |

Shared discovery may show these useful choices together, but the committing action must explain
which of them it will perform. A generic “Use” action cannot promise identical editing/control.
Service internals remain opaque unless separate current permissions allow actual inspection.
There is no assumed direct relay through an execution-only intermediary, transparent workflow
migration or release of a running service from management. Those outcomes require explicit later
decisions. Preserving data during removal is not release-management.

## Progress and recovery across the boundaries

Consider A's workflow calling a service at B whose child edits a protected area. A reserves the
task allowance and B accepts an exact request/generation. B records the planned child association
before starting it. If an acknowledgment is lost, recovery inspects or replays that exact request;
it does not create a new child or reset the allowance. A waiting publication wrapper releases its
worker so its child can run while retaining method/resource lifetime protection. Editing moves
only to the exact accepted child and only with the necessary quiescence evidence. Conflicting
maintenance refuses; unrelated work can continue.

If the child stops with durable evidence, editing returns and later work can progress. If stop
proof is lost, cancellation and restart retain uncertainty and holds. An authorized operator can
inspect and resolve the exact ownership condition; the system must not invent a successful or
failed effect merely to clear “busy.” Grant revocation can block future entry/disclosure without
rewriting already accepted history. Known usage settles once; unknown usage keeps its reservation.
This composition has source and focused test support, including legitimate child progress and
retained holds, but the complete multi-owner browser journey remains an implementation gate.

Continuous agent work uses the same scoped commands. Failed verification stays in history;
prospective repair changes future work under the governing constraints. The selected control
`WorkCommitmentOwner` in `control::controller::commitment` accepts a durable work policy and exact
pending actions, prepares bounded planning
packets, authenticates model proposals, and advances only actions permitted by that policy and
current authority. Runtime executes ordinary methods for planning, work, criticism and evaluation.
The session reads their results; it does not copy task state or decide that an uncertain external
operation succeeded. One-round manual assistance remains a policy of the same owner.

This choice follows a concrete comparison. Today's child Call returns its outputs at terminal
completion, and controller checkpoints occur between iterations. A dynamic child target would
solve selected-revision binding but would not let an enclosing root react to criticism while a
child remains paused. A complete root-workflow alternative needs new nonterminal child handles,
observation and concurrent continuation semantics. Its single history/account is attractive;
the selected control owner instead gives the continuous commitment a distinct lifetime without
adding those ordinary workflow primitives solely for S28. It is a real new orchestration state
machine, whose purpose is deciding whole activities; runtime retains execution scheduling.

Session creation must retain the exact policy, account, first action and external receipt before
effects. Each planned run records its exact revision/inputs and command material before creation;
runtime creation binds it atomically to the accepted association and originating account. A crash
between create and start recovers that same unstarted run. Proposal acceptance followed by lost
reporting recovers the same proposal instead of rerunning the model. A durable commitment admission
fence blocks new local start and entry admission after pause/stop acceptance. An entry admitted
before the fence may physically begin afterward and remains subject to ordinary cancellation.
Local publication
ancestry also carries this check even when the service has its own account. Across a peer boundary,
B owns its accepted request: it may still enter before A's exact cancellation reaches it. A retains
the outbound cancellation and unknown reservation; B enforces its own accepted cancellation,
deadline and current rights. No distributed instantaneous stop is promised. Already entered work
and physical holds remain with their actual owners. Grant revocation is rechecked at the relevant
action and disclosure boundaries.

Accounts gain a closed work-session origin with no invented originating run. Existing reserve,
settle and unknown-use transitions remain shared, so new rounds, children and restart cannot reset
the allowance. Session authority has its own explicit scope; associated runs still receive their
ordinary exact execution basis. An association grants no implicit approval, artifact-read or
publication rights. Existing marked controller methods may inherit the session account only when
the whole originating allowance fits their declared limits; otherwise local association refuses
before start and an existing published service can retain its bounded internal allowance. This
does not promise a new hierarchy of mutable subbudgets. Waiting for a human retains the exact
question/action identity. The full research/critic/reconsideration journey remains a required
implementation test, not an outcome established by these source traces.

Closing a tab or aborting a fetch stops observation, not accepted work. Logging out clears the
active token and authorized view caches. Forgetting a connection changes client configuration.
Revocation changes rights. Cancellation is a separate server command whose receipt is not proof
of physical stop. The frontend must explain each result and preserve explicit recovery records
without presenting them as an execution ledger.

## Responsibility changes selected in the reopened review

The demanding direct/workflow/private-service/resource scenario does not favor one universal
invocation journal. A direct request at B, A's workflow attempt, B's accepted serving operation,
the private child and an installation's surviving editing hold answer different recovery and
authority questions. Combining their storage cannot erase those distinctions. Publication stays
in control composition: it resolves exact service authority, public contracts and private run
association, while host owns serving acceptance and runtime owns child execution. Moving it
wholesale into either runtime or host would place one of those responsibilities across the wrong
dependency boundary.

The review does select concrete changes to those retained owners:

| Change | Producer and consumer responsibility | Consequence |
| --- | --- | --- |
| Managed-use transition policy | `persistence::managed::transition` validates bounded lineage/quiescence evidence and plans the installation/use change; redb loads actual facts and validates/writes under its existing atomic guards | Remove domain policy from redb execution/acquisition/transfer/quiesce/release helpers without moving physical evidence or introducing another resource journal |
| Fresh serving admission | Private capability-host `FreshServingAdmission` handles common descriptor/generation/operation/deadline admission after direct or peer authentication/replay handling | One common new-entry rule, while original-grant replay, current disclosure and peer-specific checks remain explicit |
| Descriptor extension construction | Capability owns a checked construction API, consumed by publication and managed-model descriptor producers | Delete JSON object surgery; keep peer descriptor remapping where local identity/trust/resource facts really differ |
| Application/control composition | Blueprint owns complete Program and versioned GraphNative editing/lowering; control owns response admission, evaluation and ongoing-work decisions; daemon owns transport, composition and exact external receipts | Move private daemon semantic helpers into shared blueprint owners with their supported version semantics intact; remove duplicate daemon ownership and external-driver stage bookkeeping after ordinary consumers migrate |

Managed transfer still requires evidence of the exact accepted parent/child relationship and
physical stop or justified non-entry. The new pure policy takes those typed facts, not booleans
such as `authorized` or `is_child`; redb remains responsible for their transactionally consistent
origin. A complete change includes refusal paths and all producer/consumer contracts, not merely
moving function bodies. The long-term test case is an operation that claims two working areas:
one policy owner must define independent per-resource handoff and writer exclusion, while storage
preserves each claim's evidence. That future feature is a comparison of maintenance cost, not
newly authorized implementation scope.

Keep exact external command receipts alongside runtime semantic retry receipts. The first binds
the original actor, grant and full wire envelope to a response; the second binds the runtime's
retry meaning, which can omit delivery/planning fields. Removing either silently weakens its
promise. Their acceptance can have separate commit boundaries, so compound session operations
must retain exact internal command material and associations before effects rather than depend
only on the final HTTP response receipt.

## Browser delivery and local data

The preferred initial deployment is an operator/user-hosted static Svelte application with an
approved daemon endpoint set, per-connection bearer fetch and bounded fetch-based SSE. CORS stays
disabled unless the daemon operator configures exact allowed
origins; preflight handles transport only and every actual request still authenticates normally.
Remote connections use trusted HTTPS termination in an operator proxy. The daemon remains bound
to loopback. Redirects are refused, cookies omitted and tokens kept out of URLs/logs. Native
EventSource cannot supply the required arbitrary Authorization header, so it is not the client
transport.

The static host also configures the browser's CSP connection policy. Adding an endpoint outside
its approved set requires a hosting-policy change, which the interface must explain. Arbitrary
endpoints from a generic public-hosted app remain unqualified. Origin/TLS trust governs initial
bearer delivery because version/authority discovery itself requires authentication; the subsequent
stable-host check prevents command/cache rebinding, not disclosure of the bearer to a replaced
already-trusted origin.

Tokens live only in memory; reload requires authentication again. Nonsecret connection metadata
can persist. Bounded private drafts and exact pending requests may persist in IndexedDB under an
explicit personal-profile choice; session/export mode keeps no private record unless deliberately
exported. Unknown submissions cannot disappear through ordinary cache eviction.
If durable local recovery cannot be retained, a deliberate exact export or refusal must precede
new effectful submission. A bounded part of the recovery capacity is reserved for control requests;
if even that cannot retain cancellation, offer exact export or an authorized native-client stop
route without silently discarding pending work or sending unrecorded effects. Browser profile
access or a same-origin script compromise can expose
retained content; omission of bearer persistence reduces that exposure but does not eliminate it.

On replacement host or changed authority, the client clears authorized caches/cursors and keeps
unfinished private work bound to its original identity for explicit recovery, export or discard.
It must not silently send that draft elsewhere or reveal the previous actor's results. Known run
inspection under new authority is separate from replaying a workflow command under its original
grant. Direct serving may permit explicit retained-request recovery under current Inspect rights;
the server compares its original accepted basis. Missing or denied reads do not prove no effect.
Independent
observation streams keep their own ordering/freshness; there is no global clock or permission union.
Capability snapshots replace the complete authorized view, including empty snapshots, and process
restart can require resynchronization. Reconnecting a feed never automatically retries a mutation.

The strongest smaller alternative is an operator-controlled same-origin proxy serving the static
client and forwarding to a fixed daemon. This avoids CORS but constrains independent hosting; an
arbitrary multi-target gateway adds target allowlisting, SSRF and possible credential-custody work.
A broker or native wrapper needs a concrete unmet requirement. Neither is a hidden prerequisite
or an assumed cure for browser restrictions.

## Work required before implementation can claim this outcome

- **R-A01:** opt-in exact-origin transport/config and actual browser-to-daemon qualification,
  including the supported HTTPS-proxy topology, authenticated streams and refusal paths.
- **R-A02:** a per-connection frontend identity/cache/exact-recovery adapter using the existing
  public authority/protocol contracts. No replacement identity API is needed.
- **R-A03:** bounded authorized managed-installation and approved-recipe discovery. Existing
  inspect/action routes work only when the client already knows the identifier.
- **R-A04:** safe forward navigation from a service invocation to its real internal run/definition
  only for independently authorized readers. Invoke-only behavior must stay opaque.
- **R-A05, corrected:** preserve receipt-bound administrative `ListMethods`/`InspectMethod`
  snapshots and the existing filtered invoke-capability catalog. Add a distinct current management
  query over the publication owner, checked per record with a bounded scan and opaque scoped cursor.
  Unauthorized records disclose no names/counts; empty pages retain truthful continuation. The old
  suggestion to filter the snapshot command as if it were invoke-only discovery was a contract error.
- **R-A06/R-A07:** correct misleading pin/copy/role/relay claims; decide whether release-management
  is required before adding a new managed lifecycle operation.

The reopened source, agent-directed-work and application investigations define richer authoring,
same-scope result selection, knowledge evaluation and session integration. A child workflow cannot
invisibly stand in for a merge because it adds identity, context and authority boundaries. A shared
diagram does not justify a uniform backend ledger. U17's human-reviewed knowledge can be approved
as a scoped reusable lesson without becoming an executable method; protected qualification retains
its separate technical obligations. Review identity, relationship, exact evidence and whether
criteria were predeclared remain visible rather than being inferred from agreement or actor count.

The implementation gate must exercise a real static build against two daemons: negotiation,
scoped reads, a controlled mutation, exact lost-reply recovery after reload, bounded SSE resync,
changed host/grant, forbidden origin and continued use of the second connection during first-owner
failure. Current actual native journeys establish useful ordinary/direct/delegated/publication
behavior. The browser probe establishes a missing route. Neither substitutes for that future gate.

The pure managed-policy/admission/descriptor corrections should preserve current semantic bytes;
prove that against readers and fixtures. The session/account/authority additions and canonical
typed source extension do need explicit versioned contracts. Keep old exact requests, origin digests,
uncertain effects, artifacts, publication associations and managed holds. New session origins must
be refused by old writers; never retrofit unrelated entered work into a fresh allowance. Active
graph runs continue through their exact source reader and common plan, with their existing
prospective graph repair path. A deliberate cross-family adoption must prove its accepted-frontier
and obligation mapping; it cannot reparent entered effects or re-seal a governing agreement.
Old command versions keep their exact graph compiler semantics even if a lost outer receipt follows
a committed revision. The same retry must not become a Program revision because a default changed.
Rollback is safe only when the older readers accept all
written facts and still preserve replay; restoring an old database while an external effect survives
can duplicate work. Role removal cannot abandon active obligations. Frontend rollback must preserve
or safely export newer unresolved local records.

Graph support has no automatic retirement in this program. A future release can remove its writers
only through an explicit compatibility decision with replacement/verified conversion or approved
capability loss covering reusable definitions, active/uncertain work, accepted proposals, pins,
publications, external clients and old imports/backups. Zero active runs is insufficient. Exact
historical readers, agreement checks, accepted-command recovery and receipts have separate lifetimes
from the editor; deleting the latter does not discharge the former.

The choice should reverse if browser deployment tests favor a same-origin route for the intended
users, if human use repeatedly misinterprets common discovery, or if a fully traced structural
alternative materially improves progress and recovery. Those changes reopen their dependent
product, notation and implementation decisions. The
[earlier architecture proposal](../working/architecture-and-federation.md) retains the detailed
browser and compound-trace evidence. The reopened [execution](../working/execution-reconstruction.md),
[application](../working/application-reconstruction.md) and
[agent-directed work](../working/agent-directed-work.md) comparisons own the changed responsibility
recommendations; the [observations](../working/observations.md) state exactly what was executed.
The [whiteboard decision](../../whiteboard/discussions/architecture/owners-and-browser.md)
records current authorship, alternatives and preserved dissent.
