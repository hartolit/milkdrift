# 04 — Compare architecture, ownership and the real browser path

**Owner:** distributed execution/methods investigator, authority/privacy critic and frontend transport reviewer. **Output:** reviewed ownership/lifetime model and browser connection decision. **Prerequisites:** 01 actual paths/browser observations; 02 working goals; 03 product meanings/alternatives. **Stop:** reasoned architecture proposals and public-operation remedies. Joint review with 05/06 is required before architectural choices are endorsed.

Read the current packet and dossiers, [independent-host decision](../../../decisions/0038-independent-host-execution.md), [published-method decision](../../../decisions/0041-published-method-invocation.md), [published-method guide](../../../guides/published-methods.md), [authority guide](../../../operations/authority.md), [control API](../../../reference/control-api.md), and the [implementation](../../practices/implementation.md)/[documentation](../../practices/documentation.md) practices. Trace current owners and tests; an older ADR's version table is not the current wire contract.

## Compare complete architecture, not only the remote-call subsystem

For each consequential product choice in 03, compare the current owner structure, a smaller complete correction and a structural alternative where warranted. Examine workspace/crate/module responsibility, dependency direction, naming and documentation ownership as consequences of the chosen behavior. A prettier directory tree is not a remedy for duplicated authority; fewer structs are not proof of greater expression.

Trace operation construction, validation, scheduling, context, resource/account lifetime, persistence, external adapters and client reads together. Identify existing strengths and exact producers/consumers to keep or replace. Prefer neither backend preservation nor rewrite by default. Measure maintenance through a concrete future change: which facts and consumers must change together, and why?

Work iteratively with 05 and 06 on the same important user actions. Their diagram, API, browser and interaction constraints can revise this model; there is no architecture freeze before human behavior is examined. If the conflict changes a goal or product meaning, reopen its 02/03 decision and list dependents. Do not conceal it by adding another configurable mode or permanent compatibility layer.

Architecture proposals are authored judgments linked to goal/product decisions, not established facts. A source observation about current behavior and a recommendation about future behavior need different labels and readiness consequences.

## Separate the questions before deciding which concepts to combine

For each current and proposed operation establish: who owns the definition; who owns the running process/history; which host performs an effect; which principal can request it; which rights govern internals and results; which resource generation is used; and what outlives the call. These are questions, not necessarily separate classes or UI pages.

Distinguish UI connections from backend peer relationships. A frontend may know A and C without A being authorized or network-connected to execute on C. A peer supplying operations is not automatically a source of editable workflows. A single machine may perform several roles. A disconnected workflow owner may still have active work elsewhere.

Compare ordinary direct invocation, workflow delegation to a remote host, pinned reuse, publication with a constrained service identity, and any proposed unification. The user may see one useful call abstraction while inspectable contracts remain distinct. Conversely, adding “remote” to a task cannot silently create trust, copy private inputs or acquire a service grant.

Reconsider workflow-owner migration or a distributed history only where a concrete outcome demands it. Describe actual new recovery, coordination and permission obligations. Do not reject it merely because the current design has one owner, and do not add it merely to make a diagram look seamless.

## Follow complete lifetimes, not only RPC routes

Use S07–S13, S20–S22 and S26–S27. At each boundary specify request identity, acknowledgment, external entry, known outcome, authoritative record and legal next action. Consider failure before proven acceptance, lost acceptance reply, possible execution, lost result, uncertain cancellation, revoked grant, retired method and changed target.

Document what can safely retry/replay and what must be inspected or explicitly resolved. Source evidence about a disconnected operation is not a reason to automatically reassign it to another host. A method's internal run, public invocation, verified output and persistent deployed service have separate completion conditions even when shown in one view.

Compose worker availability, editing access, lifetime holds, account reservations and method ancestry. A waiting caller must not block its authorized child; unrelated conflicting writers must not gain access; accepted uncertain work must not be treated as stopped. Require an actual route for legitimate progress and investigation, not permanent protective deadlock.

Check cumulative allowances and propagation of governing restrictions through ordinary children, service calls, revisions and remote delegation. A new request label must not reset inherited obligations. Service execution may use explicitly delegated authority without granting the caller raw internal privileges; test confused-deputy inputs, target paths and callbacks.

## Model an independent client with several owners

Define fully qualified references and state for hosts, sessions, workflows/revisions, invocations, artifacts, capabilities and observation feeds. Distinguish stable host identity from an address, local connection label and actor/grant context. Test same names on different owners, the same owner at two addresses, duplicate views via peer/direct connections, and an address reused by a different owner.

Specify zero-connection onboarding, partial connection failures, protocol mismatch, unavailable workflow role, stale catalogue, replaced generation, grant change, logout and deletion of a saved connection. One failed daemon must not freeze unrelated sessions. A merged overview has no implied global clock, permission union or global transaction.

Cached content is an authorized observation, not a durable truth owned by the frontend. Define what survives reload, what is cleared on an identity/authority change and how private drafts are handled. Never reveal an invoke-only method's internal graph to provide an attractive expand animation.

Use the current stream contract deliberately: [ADR 0050](../../../decisions/0050-live-capability-snapshots.md) and [control API](../../../reference/control-api.md) describe complete capability snapshots and process-local resynchronization. Recheck its actual consumer behavior. Do not recreate permission filtering or cursor semantics in Svelte.

## Resolve browser and deployment feasibility now

Use 01 observations plus current primary platform documentation. Compare viable direct browser connections, operator-controlled same-origin proxying, a narrowly scoped broker where justified and eventual native desktop transport. Svelte remains first; Tauri is not an assumed cure for browser limitations. No hidden mandatory local daemon or central cloud account is introduced by convenience.

Choose a supported initial deployment path and explicitly identify broader options needing later work. Cover:

- TLS, browser origin/private-network restrictions, endpoint/host identity and redirect credential handling;
- cross-origin authorization/preflight and authenticated streaming; native EventSource header limitations versus an appropriately bounded fetch stream;
- token provisioning/storage, refresh/rotation, scope per connection, URL/log/referrer leakage and logout;
- CSRF and cookie mode where applicable; XSS/untrusted output; SSR isolation if a server-rendered frontend is chosen;
- proxy/broker credential custody, target allowlisting/SSRF, network reachability and service ownership;
- exact command recovery across reload without automatic mutation retry, independent request abort and backend cancellation;
- deployment/build operation, environment configuration and a reproducible browser-to-daemon check.

Do not forbid or require a storage mechanism by slogan. State the threat model, safeguards, residual risks and user consequence. Do not claim support for a network arrangement merely because a Rust CLI reached it. Missing browser proof remains an owned prerequisite in the implementation program.

Useful primary entry points include [Fetch](https://fetch.spec.whatwg.org/) and [EventSource](https://html.spec.whatwg.org/multipage/server-sent-events.html). Pin the versions/dates actually used for decisions; do not substitute general familiarity for checked behavior.

## Decide, then carry the implications forward

Write `working/architecture-and-federation.md`: whole-system responsibility/dependency proposal; owner/authority/lifetime matrix; compared alternatives; successful and interrupted compound traces; per-connection state model; transport decision and evidence; source-owned API gaps; loss-and-gain account; and whiteboard decisions/blockers.

Cross-review with outcome/semantics/interface reviewers while 05/06 investigate the same operations. Link upstream goal/product decisions and downstream notation, public-contract and migration decisions. Do not adopt pools, lanes or collapsed nodes as ownership or permission rules by visual analogy. A new model must remain expressive for agent-created work, not make every cross-machine action an administrative ceremony. But simplification may not remove the actual owner who enforces an effect.

Commit coherent relationship and transport decisions with documentation checks. **Gate:** no hidden single-daemon assumption, false method bridge, permission union, duplicate execution owner or unresolved lifetime prerequisite is embedded in the preferred design. Conflicting earlier alternatives are revised, not left as parallel truths. Initial 04 completion permits downstream investigation, not a finally approved architecture; joint feedback and premise validity are checked in 07.
