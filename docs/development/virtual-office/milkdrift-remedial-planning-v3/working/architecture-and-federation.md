# Architecture and federation proposal — A1/r3

Cyra-20261010 recommends preserving the current distinct owners of immutable definitions,
workflow execution, accepted host operations and managed installations, while completing the
public inspection and browser boundaries. This is a technical planning selection under
[G1/r1](vision-baseline.md) and [P1/r2](../../whiteboard/discussions/product-model/reuse-service-and-placement.md),
revised after actual joint 05/06 criticism, 07 challenge and the final source recheck. Rowan selected
it as the planning recommendation; it awaits user decision. It does not authorize implementation or amend
canonical architecture. Source names are evidence of current behavior, not reasons to retain it.

## Grounds and compared systems

The [behavior dossier](behavior-evidence.md) records exact producers, consumers and tests against
production source `908e7893f5dadb84d12712573c8daaa946829e39`. Planning HEAD when this proposal was
written is `9878f7a73cb7e1fe410a7d7bc16e126c9f979f7a`; intervening commits do not change production.
[E01–E07](observations.md) contain coordinator-executed evidence. Cyra ran no Cargo or browser
probe. E04 establishes a failed cross-origin browser route, not a functioning frontend. Additional
coordinator observations become grounds only when recorded there. E06 verifies three managed
publication cases; E07 verifies exclusive reconvergence refusal at construction. Compound traces below are
source-backed models with named partial tests, not newly executed end-to-end scenarios.

| Alternative | Complete consequences | Decision and strongest countercase |
| --- | --- | --- |
| Current owners plus narrow public reads and explicit browser transport | Runtime retains local atomic entry/account semantics; serving retains remote/direct admission and uncertainty; control orchestrates publication through existing owners; managed resources retain intent, claims and holds. Clients gain honest discovery/recovery without durable execution authority. | Preferred. It retains several contracts the UI must explain and several test boundaries maintainers must coordinate. Common discovery could still mislead despite correct APIs. |
| Existing daemon APIs with operator same-origin proxy only | Smallest backend correction; proxy can pass bearer requests to one fixed loopback daemon. Per-owner deployment or an explicitly allowlisted multi-target proxy is required. Missing resource discovery and authorized service drilldown remain awkward. | Strongest near-term alternative if target users control their hosting and do not need an independently hosted multi-connection client. Choose it if real deployment evidence shows CORS adds more operator burden than it removes. |
| Uniform operation ledger, scheduler and backend call object | Could simplify some queries and unify waiting/cancellation representation, but must replace runtime-local atomic entry, inherited account/authority, serving-origin provenance and managed-use acquisition together. Existing accepted records need durable migration and compatible replay. | Not justified by a shared UI card. Reopen if demonstrated cross-owner correctness defects or repeated divergent policy changes outweigh migration cost; require a full equivalent progress/refusal trace first. |
| Shared history or workflow-owner migration | Could help a workflow continue when its owner is permanently unavailable. Needs authority transfer, writer fencing, request conflict/replay retention, artifact movement, active peer reconciliation and failure recovery. | No selected scenario currently demands transparent relocation. Owner outage remains visible and other connected owners remain usable. Reopen for a concrete availability outcome, not seamless drawing. |

The recommendation gains a usable independent client and explicit public relationships. It does
not gain direct relay through an execution-only intermediary, arbitrary owner migration, managed
detach, unrestricted copy, universal standards execution or completed S28 orchestration. Those
are visible decisions or investigations, not features hidden in a networking implementation.

## Responsibility, authority, lifetime and state

| Fact or action | Authoritative producer and dependency direction | Authority and lifetime | Client/public consequence |
| --- | --- | --- | --- |
| Workflow identity, immutable revision, structured graph and governing agreement | `crates/blueprint` constructs/validates; persistence stores exact definitions; runtime consumes them. Daemon authoring commands invoke these owners. | Explicit create/edit/read grants; new revisions are prospective. Accepted child creation retains exact pin and basis. | Owner-qualified revision reference. Layout is separate presentation data. A remote readable revision is not automatically local composition. |
| Run state, occurrence, attempt, context and inherited account | `crates/runtime` owns execution decisions; journal/store transactions commit history, entry and allowance together. | Frozen execution basis and current entry checks; bounded branch context; append-only accepted history survives sessions and later revisions. | Commands and reads project runtime facts. Drafts/proposals remain overlays until accepted. No client-generated timeline event becomes history. |
| Direct or foreign invocation admission, generation, external-entry evidence | `crates/capability-host/src/serving` owns the durable serving queue and workers; peer protocol carries exact foreign coordinates. | Actor/caller scoped request digest, exact generation, deadline, limits, current admission/entry/read checks. Terminal or uncertain durable evidence outlives transport. | Direct work needs no synthetic workflow. A's delegated attempt and B's serving record describe different facts; neither substitutes for the other. |
| Published service contract and internal continuation | `crates/control` prepares/publishes and drives the host's narrow continuation port; existing runtime or serving record holds the association. | Configured service grant, governed starting revision, public contract and bounded allowance. Accepted calls survive retirement; new admission refuses. | Invoke-only results remain opaque. An authorized relationship read may reveal internal references only after real internal-read checks. |
| Capability selection and adapter entry | Capability host checks registry/generation and prepares exact handles; adapter performs process/model/peer effects. | Profile requirements, controller reservation and entry rechecks. Replacing a generation does not redirect accepted work. | Show accepted generation, separate from freshest catalogue. Stale catalogue refresh is not consent to a new target. |
| Managed installation, intent, generation, editing claim and lifetime hold | `ManagedResources` orchestrates `ManagedPlatform`; redb transactions own acceptance/use coupling, claim transfer and retained obligations. | Resource-scoped authority plus platform requirements. Editing needs physical quiescence; holds may survive public cancellation, run terminal state and restart. | Existing inspect/resolve routes remain owner commands. Add bounded discovery; do not infer stop from a green/closed card. |
| Artifacts and causal context | Runtime creates bounded provenance-bearing manifests; persistence/artifact stores retain exact metadata/bytes; adapters materialize authorized selections. | Current disclosure rules and accepted execution basis; branch isolation and explicit child/public boundary. | Download by owner/exact artifact identity. A connection is not permission to combine data or context. |
| Authority/session binding | Daemon authentication resolves bearer to actor/grant; public `AuthorityRead` supplies durable host and exact grant identity. Runtime/host enforce respective actions. | Grant revisions/revocation affect prospective acts and current disclosure; do not rewrite old accepted authority. | Separate host, endpoint, local label, principal and exact grant. Never union grants across connections. |
| Durable storage and recovery | Persistence traits express owner transactions; redb implementation commits them. No browser storage or generic registry becomes a second authority. | Bounded records, exact replay/conflicts, retained uncertainty; one opening daemon owns the store. | Local exact requests help recover server records, but cannot settle them. |
| HTTP, CLI, browser and evidence drivers | Daemon handlers authenticate/project; Rust client and CLI consume protocol; proposed Svelte consumes the same public contract. Maintained Rust drivers test public behavior. | Transport abort ends observation only. Client cache lifetimes are independent of backend work. | Add CORS in daemon HTTP/config only. Keep workflow semantics out of client helpers and proxies. |

This preserves dependency purpose, not every module boundary. A future managed-discovery change
should touch protocol projection, managed owner pagination/authorization, daemon route and clients;
it should not add a runtime registry. A future paired-branch merge changes blueprint validation,
runtime scheduling and evidence first, then authoring/notation/client rendering. These are different
contracts. The current multi-incoming non-Join refusal in `blueprint` is not remedied by drawing a
merge icon. A pinned child workaround also adds context, authority and identity boundaries; it is
not an equivalent same-scope merge. See [merge evidence](merge-construction-evidence.md).

## Four compound traces

### S21 — nested service, protected resource and changing authority

1. A accepts canonical workflow command `qA` under actor/grant `gA`, creates run `rA`, and reserves
   the task's allowance before a remote attempt. Its peer request names exact B capability,
   generation, operation, inputs, expiry, real run/revision/node/attempt and reservation. B accepts
   request `qB` into serving record `iB`. A lost reply means inspect/replay exact `qB`, not a fresh
   request or fallback host. An admission receipt does not mean physical entry.
2. B's publication record binds `iB` to exact planned internal run `rB`, starting revision,
   service authority and public result contract before create/bind/start. Commit-boundary recovery
   continues that association. The caller can inspect public `iB`; it gains no permission to see
   `rB`. Current publication recovery tests and E03 prove selected lost-boundary behavior.
3. A waiting wrapper releases its execution worker while retaining the accepted method generation
   and resource lifetime protection. The exact accepted internal child receives editing only
   through parent/child claims and quiescence evidence. The wrapper's `NoExternalEntry` permits
   progress; a physically entered child needs actual stop evidence. Unrelated resource work may
   proceed, while conflicting maintenance refuses with the actual blocker. It cannot steal editing.
4. A revoked grant blocks a prospective entry where its relevant authorization is rechecked; it
   does not fabricate cancellation of an effect already entered or invalidate old facts. Service
   continuation uses its accepted association and applicable service authority, not an arbitrary
   extension of the caller's rights. Which current read remains available is checked separately.
5. If the child stops and its durable evidence settles, editing returns and another exact child
   can enter. The managed publication success test observes two such physical entries and no final
   hold leak. If stop proof is lost, cancellation/daemon restart retains suspended parent/entered
   child and their holds; unauthorized return refuses. Authorized inspection and exact-use fencing
   can resolve editing ownership. The old effect remains uncertain unless evidence resolves it.
6. Known usage settles the caller/service reservation once. Unknown usage retains its reservation;
   method return, relabelled requests and worker release cannot reset it. Ordinary descendants
   inherit the controller account; service invocation receives an explicit bounded allowance.

Source owners: `serving/direct.rs`, `serving/worker.rs`, publication continuation and
`adapters/redb-store/src/managed/{uses,execution}.rs`. Discriminating tests are
`published::managed::publication_hands_editing_to_exact_internal_writers_and_releases_every_hold`
and `lost_child_stop_proof_survives_cancel_and_reopen_without_returning_editing` in
`crates/control/tests/control_service/published/managed.rs`, passed in E06. The combined caller revocation,
separate maintenance UI and third-owner browser journey remains an implementation acceptance
scenario; passing its component tests does not prove the whole interaction.

### S22 — same names, different owners, concurrent edits and endpoint replacement

The browser receives authority snapshots for A and B before using their catalogues. Both may
have workflow `release`; stored references are `(host, workflow, revision)` and authorized cache
partitions add actor/exact grant. A draft records its originating partition, exact parent revision,
explicit mutations and independent layout. Two valid concurrent edits from that same immutable
base may both save as sibling revisions. The authoring envelope checks that `expected_revision`
equals the draft's exact base, not a mutable workflow head. Show ancestry and the explicitly
selected revision; offer deliberate comparison/reconstruction/copy without overwriting either
sibling. An actual envelope/base mismatch, invalid ancestry/content or a distinct guarded
run/layout operation can refuse. Do not manufacture a latest-head conflict or silently rebase
effects/pin different children. A current-head compare-and-swap API is not selected.

If A's URL now answers with host C, protocol and authority reads expose the mismatch before any
saved mutation replay. The browser quarantines A's private draft/request records under A's
original binding; it does not send them to C, show cached A results as C's, or discard recovery
evidence. A at two addresses may share a host label but does not merge actor/grant contexts.
Peer-advertised B and directly connected B can be related in presentation only after stable
identity evidence; their usable routes and credentials remain distinct. Each observation feed
retains its own owner/scope/cursor and freshness. There is no total order across A and B.

Grounds: `AuthorityRead`, `SavedRunRequest::submit_saved_run`, `authoring::base/finish`,
`definitions::retain_revision`, `RedbStore::put_revision` and the separate layout owner. Existing
native recovery supports exact identity checks; browser sibling-revision display, actual base conflict,
duplicate connection and replacement-endpoint scenarios are unexecuted acceptance requirements.

### S26 — continuous agent work, repair, bounded reuse and interruption

An agent uses the same authenticated commands to author/propose future work, inspect evidence,
submit a prospective revision and request control. A failed verification remains attached to its
accepted attempt. Repair changes future work under the governing agreement and current authority;
it cannot erase the failed result or broaden protected evaluator bytes/targets. Developing a tool
requires an actual configured capability/managed-resource route and its privileges, not a generic
text node that gains effects. Ordinary descendants share inherited account/constraints; a service
call has the explicitly delegated contract and allowance. Its child association survives a lost
reply and cannot be reconstructed as a new free call.

Waiting for authorized human input is a durable workflow condition. Closing the observing session
does not resolve it. After daemon restart, owner recovery and an authenticated reader recover the
same run and accepted request; unknown external effects remain unknown. Current requirements can
refuse further entry while investigation and authorized resolution remain available. Finite
iterations, deadlines and account bounds need a visible continuation/checkpoint decision instead
of pretending a new label replenishes the same run. An independent direct call is a new bounded
call, not proof of a cross-project lifetime budget. Arbitrary subprocess-internal model use is
not automatically accounted as daemon model calls.

E01 covers prospective repair and replay; E02 retains unknown usage; publication constraints cover
inherited depth/account and reopen. The full research/team coordination/evaluated lesson process
in S28 is not established by these parts. Learning discovery and richer structured authoring are
separate public-contract tasks; do not create a fictional research primitive in the client.

### S27 — client lifetime and accepted work lifetime

Before effectful submission the client must retain or explicitly export a versioned exact recovery
record including original host/actor/grant binding and canonical request, then submit once. A
lost reply leaves a local unresolved submission; current owner lookup supplies authorized evidence
of accepted state or continuing uncertainty. A missing/denied lookup is not proof that no effect
ever occurred. Absence of a local response is not permission for a
new request. Identical replay and changed-byte conflict follow the server contract. Reauthentication
does not silently rewrite the record's original authority; a changed grant needs explicit recovery
handling and current permission for disclosure. Workflow `SavedRunRequest` requires exact
authority equality, and daemon command fingerprints include grant identity/revision/digest. A new
grant may permit inspection of an already known run without permitting replay of its old command.
Direct serving is different: `invoke_client` can recover a retained exact request with current
Inspect permission while comparing against the stored original authority basis. Keep that explicit
operation-specific route; never rebind the private original record or impose workflow-helper
equality as a new universal frontend restriction.

| User event | Client result | Backend result and legal recovery |
| --- | --- | --- |
| Close tab or abort fetch | Observation/request transport ends; retained exact record remains according to explicit local retention policy. | Accepted work continues. Reconnect, authenticate, inspect exact record. |
| Log out | Erase in-memory bearer, abort feeds and clear authorized view caches. Explicitly retained private drafts/recovery records stay locked to their original partition, never shown as a different actor's view. | No implicit cancellation or grant revocation. Reauthentication is required. |
| Forget connection | Remove endpoint metadata/active session; make draft/recovery disposition explicit before deleting the only local record. | Owner and work persist. Exact exported records can recover later at the original owner. |
| Revoke grant | Current reads/commands may refuse; invalidate cached authorized views/cursors when observed. | Future effect entry follows owner rechecks; prior entry/history is retained. Recovery may need a different explicitly authorized operator. |
| Request cancellation | Submit an explicit scoped command and retain its receipt. | Acknowledgment is not proof of physical stop; inspect outcome and retained holds, reconcile when authorized. |

Client state is: disconnected → negotiating → authenticated current observation; unavailable,
protocol mismatch, missing workflow role, changed identity/authority and unresolved submission are
separate states. A failure at A must not freeze B. Catalogue refresh and stream resync replace
authorized views; they never replay mutations. Capability/health feeds have router incarnation and
scope, complete snapshots (including empty), bounded frames and explicit resync. Run feeds retain
their distinct durable cursor contract. Direct invocation observations are not fabricated run SSE.

## Conditional initial browser route

Choose an operator/user-hosted static Svelte client with an approved daemon endpoint set, using
direct bearer-authenticated fetch, strict **opt-in daemon CORS**, and an operator TLS reverse
proxy for remote deployment. Keep the daemon's current
loopback binding rule. This choice is conditional on R-A01's successful browser acceptance: today
the route fails. A same-origin fixed-target proxy is a supported design alternative to test, not
an undeclared mandatory local daemon or central service. SvelteKit/adapter version and frontend
package ownership belong to 06; architecture does not infer SSR or a JavaScript gateway.

E04 observed native authority success and OPTIONS 405 without CORS headers, then all six tested
authenticated cross-origin browser requests failed. The primary [Fetch CORS contract](https://fetch.spec.whatwg.org/#http-cors-protocol)
and [EventSource interface](https://html.spec.whatwg.org/multipage/server-sent-events.html#the-eventsource-interface)
were checked on 2026-10-10. Authorization requires explicit cross-origin permission; native
EventSource offers no arbitrary bearer-header option. Use a bounded fetch stream following the
existing SSE framing/resync contract. Do not put credentials in a URL to work around EventSource.

The proposed transport contract is:

- CORS is disabled by default. Configuration names exact origins, including scheme/host/port;
  reject wildcard, opaque `null`, malformed or credential-bearing values. CORS grants no daemon
  operation. Unauthenticated OPTIONS answers only finite configured methods/headers/origins and
  never dispatches a command; the actual request still authenticates and authorizes normally.
- Allow only headers the public client uses, including Authorization and JSON Content-Type;
  audit Range/request-identity usage rather than permitting arbitrary headers. Expose required
  range/response metadata deliberately. Apply CORS consistently to allowed-origin refusal/error
  responses so a client can distinguish authorized denial from transport failure.
- Fetch uses `credentials: omit` and `redirect: error`. Tokens stay in request headers, absent
  from URLs, analytics and logs. Each connection has its own session-only in-memory bearer;
  reload/reconnect requires provisioning again. No refresh or identity-provider feature is invented.
  Current version/authority discovery is authenticated: trusted origin/TLS and operator endpoint
  configuration govern bearer delivery before the durable host comparison. That comparison guards
  later commands/caches; it cannot promise to hide a bearer from an already trusted origin whose
  daemon was replaced. A stronger bootstrap guarantee needs a separately selected identity design.
- Persist nonsecret endpoint metadata; before the first private draft/effectful request, require
  an explicit personal-profile retained mode or session/export choice. Retained mode uses bounded
  private IndexedDB drafts/exact requests and is unencrypted at rest. No bearer persistence.
  Pending/unknown requests cannot be silently
  evicted; unavailable/full storage refuses new effectful submissions unless a deliberate exact
  export provides recovery. A session-only storage option has an explicit recovery-export cost.
  F2/r2 reserves 16 records/4 MiB of the 128-record/32 MiB total for cancellation/pause/authorized
  resolution; ordinary records use at most 112/28 MiB. If reserved capacity or storage also fails,
  offer explicit exact export or the authorized native-client stop route. Do not hide cancellation,
  send unrecorded effects, evict unknowns or turn the reserve into an unlimited queue.
- Cache partitions include original host and actor/grant. On authority change discard authorized
  view caches and cursors; quarantine private drafts/recovery records under their original binding.
  Browser storage is not secure against same-origin script compromise or an unlocked browser
  profile. Render untrusted outputs as data, use a restrictive frontend policy, bound retention,
  and provide deliberate clearing/export. Same-origin JavaScript encryption with an equally
  accessible key does not remove that residual risk.
- With bearer headers and omitted cookies, no ambient-cookie session is introduced. Any future
  cookie proxy changes CSRF and credential custody requirements and reopens this decision. Static
  delivery has no SSR request context in which sessions can accidentally mix.
- Remote endpoints require a trusted HTTPS origin and an operator-managed proxy to the loopback
  daemon. Verify long responses, streaming flush, finite limits and no redirect credential leak.
  Browser mixed-content and local-network permission behavior remains deployment/browser-specific;
  CLI reachability does not qualify it. Initial evidence must state actual supported browser/version
  and topology rather than claiming general LAN support. The static host owns a CSP `connect-src`
  policy for its approved endpoint set and supplies matching nonsecret deployment metadata to the
  app. An outside-set Add connection action explains the hosting-policy change; the app cannot
  relax response policy at runtime. An arbitrary-endpoint workbench on a generic public origin is
  unqualified. A broader scheme-based CSP is an alternative with different containment, not a
  silent fallback or an already selected product promise.

A same-origin proxy avoids CORS for its served UI but must own a fixed target or narrow configured
target allowlist. An arbitrary URL-forwarding gateway introduces SSRF/reachability and possibly
credential custody; it is not a harmless frontend helper. A broker is justified only by a concrete
unmet reachability/identity outcome and an explicit new operator/credential owner. Native desktop
transport is eventual and independently testable; it does not repair the required first Svelte path.

The first implementation check must serve the actual static build at a different origin, connect
to two isolated daemons, negotiate/read with scoped tokens, submit a controlled exact request,
consume bounded authenticated SSE, force resync/reconnect and recover a lost reply after reload.
Exercise forbidden origin, missing/wrong bearer, changed host/grant, retired generation and a
second working connection while the first fails. Repeat the supported HTTPS-proxy topology. E04
alone cannot satisfy any of these successful-browser claims.

## Source-owner remedy inventory

These are proposed boundaries, not a commitment to every new endpoint. Missing behavior differs
from existing behavior that the interface explains poorly, and from a concern needing a probe.

| ID and classification | Exact current producer/consumer and gap | Proposed remedy and discriminating acceptance |
| --- | --- | --- |
| **R-A01 — missing transport** | `apps/daemon/src/config.rs`, `config/compile.rs`, `http.rs`, `http/response.rs`; plain loopback server, bearer-only routes, no CORS. E04 fails. | Add disabled-by-default exact-origin config and narrowly scoped HTTP preflight/response layer, documented TLS-proxy deployment. Existing auth unchanged. Prove permitted-origin progress, forbidden-origin/credential refusal, bearer fetch SSE/resync and two-owner isolation using actual static build. |
| **R-A02 — missing client integration, existing identity API** | `control-protocol::AuthorityRead`, `control-client::SavedRunRequest`, public protocol negotiation and stream readers already define host/grant binding and recovery. No frontend exists. | Implement one per-connection browser adapter/cache/recovery owner in 06. Use existing identity/protocol reads; do not add a second identity or generic authority oracle. Refuse replacement endpoint replay, preserve exact pending records, prove independent connection progress and reload recovery. |
| **R-A03 — missing public discovery** | Persistence already has `managed_installations(after, limit)` including removed names; managed platform owns approved recipe maps. Public `POST /v1/resources` needs explicit installation/action; no list/recipe discovery. | Add bounded authorized installation pagination and sanitized approved-recipe summaries through managed-owner/adapter ports, protocol and daemon routes. Candidate GET routes `/v1/resources` and `/v1/resource-recipes` require contract review. Never have Svelte read store/config or expose recipe secrets. Bound scanned rows as well as returned rows; support progress past hidden entries without leaking cursor identities, and distinguish an empty permitted page with remaining unknown scope from proven end. Do not demand broad disclosure solely to make narrow discovery usable. Test empty/removed/denied/full pages, restart, exact inspect/prepare progress and no hidden-name leak. |
| **R-A04 — missing qualified relationship projection** | `peer-protocol::ServingInvocationRead` has acceptance/generation/limits but no internal child link. `control-protocol::RunRead.published_source` is reverse-only; method inspect is administrative. Invoke-only public flow already works. | Add a narrow forward relationship read/projection only for callers independently authorized to inspect the internal run/definition; choose owning control/serving join with no new ledger. Candidate invocation relationship endpoint or optional read field needs explicit refusal semantics. Prove invoke-only response remains opaque, gaining internal reads enables real navigation, later revocation removes disclosure, restart retains exact association. Internal IDs/titles and relationship metadata are private too: clear current caches/views and reject late old-session responses after permission loss. No new relationship authority may be inferred from invocation ownership. |
| **R-A05 — uncertain authorization/progress concern** | `apps/daemon/src/host/commands/publications.rs::execute` ListMethods obtains a page then `?`-authorizes every row; narrow scope may fail a whole mixed page. Broad tests do not discriminate. | First run a scoped mixed-publication page probe with known caller grants. If actual intended list contract fails legitimate progress, fix pagination/filtering in the owning query and prove bounded work/non-disclosure; otherwise document the existing broader-list requirement. Do not implement speculative filtering that changes grant semantics. |
| **R-A06 — misleading interaction/documentation** | Local-store subworkflow lookup, `authoring::copy` agreement refusal, remote adapter `accepts_direct_inputs == false`, `refuse_workflow_obligations`; a generic use/move/remote gesture can imply unavailable semantics. | Explicit distinct commits and reasoned eligibility under P1/r2. Correct ADR0041's ambiguous execution-only consumption wording after route reconciliation. Test copy refusal and local pin; mark cross-owner transfer, role change and relay as explicit absent/qualified operations. No new relay follows automatically. |
| **R-A07 — user-decision conditional missing operation** | `ManagedAction` has no detach; Preserve retains data during owned-service removal, not transfer of a running service. | If the user selects release-management, first define new external owner, pending holds/children, evidence, rollback and authority; implement across managed platform/store/control end to end. Otherwise plainly state preserved data versus still-managed service. Do not label Preserve as detach. |

Structured merge/authoring remedies belong to 03/05 and learning-receipt discovery to 06; both
must appear in the shared implementation dependency map without renaming them into these IDs.
Managed discovery is not permission to enumerate private installation secrets. The initial public
contract can expose a sanitized known identifier, lifecycle summary and permitted inspection path;
final field selection must follow current authority masking, error and pagination conventions.

The direct-invocation router offers exact request/invocation lookup, not a server-wide invocation
list. Initially label locally retained history as known on this device; a fresh browser imports
an exact record and recovers through the existing owner. If independent fresh-browser discovery
becomes a required outcome, assign a bounded authorized query explicitly. Do not quietly turn a
local receipt collection into a complete server history or add another inventory without demand.

## Relay demand and ownership investigation

Current direct C→B relay is not established: peer remote adapters demand durable workflow origin
and are excluded from C's direct-input catalogue. A client connected directly to B can invoke B;
a genuine workflow at A can delegate to B. The ADR's broader wording must be reconciled with
those exact routes. This recommendation neither removes a working route nor adds a generic relay.

A meaningful reason to add relay would be a user who can legitimately reach/authenticate C,
cannot directly reach B, and needs bounded ordinary host invocation without introducing a workflow.
Before selection, prove that need against a fixed-target proxy or direct B connection. A relay
would require a real origin identity distinct from invented workflow coordinates, authority
attenuation at C and B, exact request namespace/digest preservation, bounded selected artifact
transfer, cumulative allowances across hops, cancellation/uncertainty ownership, loop/depth limits,
generation pinning, durable lost-reply recovery and current disclosure. It must prevent arbitrary
network proxying/confused-deputy input and survive a disconnected intermediary without duplicate
entry. These are new protocol/owner obligations, not a small `accepts_direct_inputs` override.

## Migration, rollout and reversal conditions

Start with the existing code constants/readers/fixtures, not old ADR version tables. Prefer
additive read projections and transport config without changing durable facts; implementation must
verify that expectation. Coordinate control/peer protocol compatibility, config schema handling,
Rust native consumers and browser negotiation in the owning change. Do not invent a next version
number in this planning note or promise rolling mixed versions before tested negotiation/refusal.

Before a durable migration, inventory exact accepted requests, publications/generations, run and
serving uncertainty, artifacts, service grants and managed intent/holds. Use supported backup and
exclusive store ownership; copying a database does not snapshot external services or prove them
stopped. Active/uncertain obligations cannot be deleted to make an upgrade clean. Preserve the
original recovery owner or provide a verified migration of each obligation. Workflow-role removal
must continue to refuse live obligations. No planned browser deployment changes serving host ID.

Rollback an old binary only when its actual readers accept every written fact and exact replay
still holds. Otherwise retain the compatible owner and restore only through an explicit supported
migration/recovery path; restoring an older store while an external effect survives can repeat
work. Static frontend rollback must read or safely preserve/export newer local draft/recovery
records; it must not erase unresolved submissions on version mismatch. Disabling CORS affects
connectivity, not accepted work or grants. TLS proxy configuration and token references remain
operator-owned deployment data.

Reverse A1 if a tested structural alternative materially improves complete progress/refusal and
recovery, if supported browser deployment makes direct CORS unworkable for the required audience,
or if human use shows P1's common discovery cannot communicate the boundary. Reopen the affected
upstream decision explicitly: browser route changes 06/08 deployment; owner changes affect
03/05/06/08 identity, notation and recovery; a same-scope merge correction affects construction and
scheduling but does not justify replacing serving ownership. Joint review must retain these live
countercases rather than treating this document as an architecture freeze.
