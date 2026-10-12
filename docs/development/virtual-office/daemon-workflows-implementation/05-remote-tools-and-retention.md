# 05 — Remote tools, shared connections and rolling retention

## Assignment and prerequisites

Execute this prompt only after the user has authorized this implementation sprint and phases 01–04
are complete. The production Svelte client, early user review and its repairs, evolving task path,
and local method expansion must already work. Read their handoffs and preserve their production
path. This is implementation of the selected design, not another investigation or a new transport.

Read `AGENTS.md`, the canonical product documents in their required order, the relevant
[implementation](../../practices/implementation.md) and
[documentation](../../practices/documentation.md) practices, and the
[workflow](../../workflow.md). Then read the [sprint README](README.md),
[task-workflow decision](../../../decisions/0051-daemon-task-workflows.md),
[code-change map](code-change-map.md), current peer/daemon operation guides and the changed source,
callers, fixtures and tests. Inspect Git before editing and preserve unrelated work and the
pre-reset references. Current source may have moved during earlier phases; follow its real owner.

Finish three connected behaviors across daemon, CLI and Svelte: persistent directional execution-host
connections, truthful remote-operation recovery, and rolling retention with protected operational
state. An authorized fresh client sees the selected daemon's shared environments and retained work.
No client must recreate the topology, know a workflow ID in advance or hold a recovery file.

The full gate and combined acceptance belong to
[07 — final test and repair](07-final-test-and-repair.md). This phase compiles affected callers,
runs its focused checks and fixes every known in-scope defect before handoff. Do not postpone a
failure to phase 07, weaken CI or count a native HTTP test as browser evidence.

## Complete connection administration

Use the existing peer HTTPS transport and expected-host identity check. Operators provide LAN/mesh
connectivity, browser-trusted HTTPS and the permitted listener/proxy deployment. Keep the explicit
loopback development exception. Do not add SSH execution, discovery, a VPN, certificate issuance,
a central registry or a remote workflow owner.

Implement the following through ordinary typed public commands and the existing authority owner.
Connection IDs and generations are durable daemon facts, while connection health and catalogs are
bounded observations. A display name or URL is not host identity.

1. B's administrator creates an inbound relationship for A on B. The request fixes A's expected
   identity, allowed operations/capabilities, resources, limits and expiry. The public B interface
   explains those scopes and issues the credential without granting A any workflow or administrative
   authority on B. Empty permissions offer no tools. The same operation is available in CLI and
   Svelte; it cannot require editing B's database or a hidden test fixture.
2. A's administrator imports B's endpoint, expected identity and relationship credential into A.
   Validate transport and identity before declaring it connected. Persist the connection even when
   its target is temporarily unavailable, with a truthful configured/unreachable state and explicit
   test/reconnect action. Do not let a model-supplied URL replace an administrator's configured
   target. A can use B without B gaining a reciprocal grant to use A.
3. Store secrets through restricted private daemon-owned files and opaque references, using the
   existing secret resolver. Stage file writes, then atomically accept the connection/relationship
   and handle. An interrupted write leaves either an accepted handle or an unreferenced staging
   file that bounded cleanup can remove. Never serialize secret bytes into the workflow journal,
   ordinary command receipts, catalog, logs or read responses. Initial credential delivery uses
   the authorized secret-issuance response; if that response is lost, an explicit rotation produces
   a replacement and invalidates the lost credential instead of making secret reads available.
   Preserve exact replay/conflict of the non-secret acceptance result and report delivery status
   truthfully. Browser tokens remain in memory under the first slice's policy.
4. Implement list/show/add/test/edit/disable/revoke/remove and credential rotation. Mutations carry
   the expected generation and exact idempotency key; stale requests conflict rather than overwrite.
   Bound connection counts, credential files and concurrent tests. Reuse the configured relationship
   count ceiling where applicable and show capacity refusal. Repeated connection tests do not
   create unbounded registries or permit generations.
5. Disable/revoke survives restart and closes new selection and entry under the authority owner.
   Rotation creates a new generation. Drain retained generations only after their accepted
   operations release them. Preserve the old execution association for permitted query/cancel
   without reopening invocation authority. Refuse removal while operations remain unresolved;
   show the reason and permitted inspection/reconciliation actions. Revoke on A cannot claim that
   B stopped an already accepted command.
6. Show redacted offered tools, exact host identity, health/catalog age, scope/expiry and accepted
   operation state through A. Enforce separate inspect, invoke and administer permissions. An
   actor who can inspect A's remote tool does not receive its credential or a direct B login.
   Connecting a client directly to B requires that client's separate B authority. A's imported
   limits and B's offered limits both apply; reconnect cannot silently broaden either.

Replace restart-only configuration of these mutable relationships with one persisted owner and its
startup loading path. Keep bootstrap listener, initial actor and external secret-source configuration
where it belongs. The new configuration reader refuses mutable relationship declarations with
guidance to the public administration path; update fixtures/examples to use that path. Do not retain
a TOML seed/reload mechanism that competes with saved connections. Reuse existing endpoint, scope
and quota validation rather than duplicating it in the GUI.

## Preserve remote effects and independent hosts

Remote tool selection is connection ID plus remote capability ID in A's public/model catalog, not
a display name or fixture-injected registry item. Freeze descriptor and connection generation at
start; refuse stale/revoked offers before entry and refresh without silently selecting another host.
Test duplicate display names across B/C, model and Svelte selection through the same catalog, and
returned artifact digest/access checks before local binding.

Secret import idempotency uses the ADR's stable staged identity and private keyed fingerprint.
Test same key/same bytes after lost response, changed-secret conflict, interrupted staging cleanup
and redacted reads/logs/receipts. Retrying cannot allocate another handle or weaken exact matching.

Exercise a workflow task on A that calls an advertised execution environment on B. B may run in
execution-only mode and must not instantiate A's workflow runtime. It owns the accepted operation,
enforces its offered tool/resource bounds and returns observations/artifacts. A owns task scheduling,
instructions, result bindings and workflow history. Project-specific procedures and environment
explanations remain ordinary Markdown/files written with tools.

Persist the selected host/generation, canonical remote request and operation identity before unsafe
entry. Disconnect after B accepts a command but before A receives completion. A must retain an
unresolved operation, protect its inputs/results and show what is known. Reconnect and restart use
the original identity to query or replay the exact accepted request; they never issue a new unsafe
request because an HTTP response was lost. Known terminal evidence wins over later recovery;
unknown disposition remains uncertain. Do not turn a timeout, worker disappearance, requested
cancellation or relationship revocation into a proven stopped state.

Finish cancellation and repair through the same public path as other tasks. Pre-entry cancellation
can prevent invocation; after entry the UI distinguishes stop-requested, confirmed stopped and
uncertain. A corrective replacement is linked future work. It cannot repeat an ambiguous effect
until host evidence or authorized reconciliation permits the continuation. Resolve pending consumer
bindings explicitly and preserve the earlier attempt's immutable facts.

Preserve direct execution and independent host startup, process lifetime ownership, scoped artifact
transfer, generation draining and Podman managed-resource safeguards. Keep exact prepared-generation
checks, resource accounting, claims and holds, reopen/remove refusals and unresolved resource state.
Do not restore retired publication/evaluated-learning requirements or weaken a retained protection
to make the new workflow work. Any obsolete adapter path discovered within this boundary must be
removed with its consumers and fixtures.

Use temporary local daemon roots and controlled external adapters for repeatable tests. Physical
Podman or other-host qualification may use only an explicitly authorized disposable resource and
existing access; do not stop services, alter host privilege, pull unapproved paid resources or
operate a retained store merely to obtain a pass. When that access is absent, record the exact
missing physical evidence and keep the dependent claim open. Deterministic Podman mechanism tests
remain required and do not prove a physical deployment.

## Implement rolling retention and discovery together

Apply the ADR's defaults and configuration through the existing persistence and maintenance owners:
30 days for completed detail/results/optional context; a 1 GiB retained-history/artifact allowance;
and a separate 256 MiB/100,000-entry compact-receipt allowance, whichever bound is exhausted first.
Use validated configurable values, checked accounting and the existing durable clock boundary.
No second daemon collector or client-side retention engine is needed.

Before collection, compute the transitive protected reference closure for unfinished tasks, pending
bindings, current templates, accepted model-action admissions and method expansions, unresolved local
or remote operations, and exact command recovery. Protection and reference changes share guarded
transactional ownership: a concurrent bind, task start or accepted expansion either obtains a valid
hold or receives a typed expired/unavailable refusal. Cleanup cannot remove data after a reference
has been accepted. A method snapshot remains available while its copied unfinished tasks need it.

Collect only sufficiently old, unprotected daemon-owned content, using bounded pages and restartable
progress. Age alone does not settle unfinished work. Cleanup crashes must leave discoverable,
consistent state and must not resurrect expired content. If protected state fills capacity, refuse
new work/uploads before consuming unreserved bytes and expose the limiting owner/reason. Preserve
ordinary project directories, Markdown and Git; they are not garbage under daemon retention.

Keep canonical requests and compact original command results for exact replay/conflict throughout
the bounded store generation. Large result artifacts stay referenced rather than being copied into
receipts; an old command replays its original result reference even if fetching its unprotected
content now reports expiration. Never forget an accepted key, treat it as new, reset the generation
automatically or keep a hidden permanent full event archive. Bound compact receipts and unresolved
operational records, reserve capacity before acceptance and refuse admission truthfully when full.
Hot-tier compaction alone does not establish rolling storage bounds. Reconcile physical artifact,
record and index accounting so the reported allowance describes what its owner actually retains.

Finish daemon-scoped indexed search by title/description, execution time and retained text result,
with bounded result size and cost. Respect the ADR's default 50/maximum 200 page sizes, stable
snapshot bounds and actor/daemon/query-scoped cursors. Authorize before projecting records, snippets,
counts or expired-content metadata; do not leak hidden matches. A fresh client can find the earlier
research result by title/time and inspect its exact output without a known workflow ID. The timeline
shows parallel dependency lanes, actual start/end times, method origin, corrective links and honest
remote states. If retained detail expired, show that explicitly. Cursor expiration triggers an
authorized fresh snapshot in CLI/Svelte rather than inventing missing events or success.

## Owners and complete adoption

Capacity tests must preserve control of admitted work. Implement the ADR's pre-reserved terminal,
uncertainty, reference-release and target-generation stop/revoke/reconcile slots; ordinary admission
cannot consume them. Fill ordinary receipt capacity with work active, then stop or settle it, revoke
the connection, reconcile and restart. Repeated controls reuse their bounded action identities.
The reserved administrative capacity-increase command remains usable; no unbounded emergency log.
Explain that canonical command text retained for exact recovery can outlive display-history expiry.

Follow the [code-change map](code-change-map.md) and trace callers before editing. The principal
current owners are:

- `apps/daemon/src/host/peers.rs`, `host/peer_store.rs`, `http/peers.rs`, `config/`, authentication and
  authorization: persisted directional administration, registry loading and redacted public views.
  Update `apps/cli/src/command/peer.rs`, `crates/control-protocol`, `crates/control-client` and the
  production `apps/web` environment/settings/operation views in the same changes.
- `crates/authority`, `crates/capability-host/src/serving/`, `registry/`, and
  `adapters/peer-http`: offered authority, identity, credential generations, exact accepted work,
  cancellation, reconnect and artifact transfer. Keep `crates/peer-protocol` changes as narrow as
  the actual wire contract requires; no speculative protocol redesign.
- `crates/persistence`, `adapters/redb-store/src/peer/`, application receipt/artifact/journal owners,
  and `apps/daemon/src/host/maintenance.rs`: durable connection metadata, protected references,
  collection, accounting, restart/refusal and receipt capacity. Private secret-file custody belongs
  to the daemon/local-secret boundary rather than redb storing secret bytes.
- Runtime recovery/attempt and control reconciliation owners: remote attempts retain frozen input,
  generation and request identity; administrative connection changes cannot rewrite accepted work.
- `crates/capability-host/src/managed/`, `crates/persistence/src/managed/` and
  `adapters/managed-linux`: retained independent environment/resource behavior and its consumers.
- Daemon public reads/streams and the phase-01 discovery/index owners: bounded authorized search,
  retention-aware results and cursor resynchronization used equally by agents, CLI and Svelte.

Remove in-memory-only revoke semantics, obsolete symmetric authority requirements, and superseded
connection/retention code when their replacements are adopted. Preserve the useful serving ledger
and its uncertainty invariants. Review API exports, dependencies, wire fixtures, evidence helpers
and maintained examples with each owner change; do not add a generic connection framework.

## Focused evidence and required failure cases

Add proposed modules under the existing production integration targets for connection administration
and retention. Their final test names must be recorded in the handoff; they do not exist merely
because this prompt names the coverage. Test doubles control model/tool responses, clocks and faults,
but the actual daemon performs admission, scheduling, grants, expansion, recovery and collection.

Required cases include:

- Separate A/B admin actors; read-only and invoke-only actors; no automatic B login; secret
  read-back/log/receipt refusal; wrong remote identity, insecure URL, expired grant and unavailable
  catalog; independent inbound/outbound setup without reciprocal authority.
- Connection create/rotate/revoke races, stale generations, unknown response after acceptance,
  credential staging interruption, restart after durable revoke, bounded repeated reconnects,
  refusal to remove an unresolved operation and retained generation query/cancel after rotation.
- Remote accepted-before-disconnect, terminal-before-recovery, entered-without-terminal, cancellation
  before/after entry, revoked permission, denied output transfer and exact replay/conflict. Count
  real adapter entries so a duplicate unsafe command cannot hide behind matching final text.
- Independent execution-only/direct operations and affected Podman resource lifecycle/reopen/refusal
  cases after the connection changes; no workflow runtime dependency on B.
- Old completed result expiration, pending input protection, current-template and accepted-expansion
  protection, unresolved-operation protection, bind-versus-collection and start-versus-collection
  races, interrupted collection/reopen, smaller configured bounds and clock rollback refusal.
- Exact command replay and conflict after detail expiry, explicit expired-content response, receipt
  byte/count exhaustion, protected-capacity exhaustion, incremental cleanup accounting and bounded
  search/streams. Vary content sizes and graph fan-out; do not prove bounds only with tiny fixtures.
- Two authorized scopes with matching search text, another daemon with the same display names,
  altered/expired cursors and revoked stream authority. Hidden results must not appear in snippets,
  counts, progress projections or stale client cache.

The following existing targets are the starting focused checks; update their fixtures to the adopted
task path and record intentional target renames from earlier phases. Run changed package/caller
checks and the relevant targets while developing, then the complete affected target before handoff:

```sh
cargo check -p milkdrift-daemon -p milkdrift-cli -p milkdrift-control-client --all-features
cargo test -p milkdrift-authority --all-features
cargo test -p milkdrift-control-protocol -p milkdrift-control-client --all-features
cargo test -p milkdrift-daemon --test control_plane --all-features
cargo test -p milkdrift-daemon --test two_daemon_peer --all-features
cargo test -p milkdrift-peer-http --test peer_service --all-features
cargo test -p milkdrift-peer-protocol --test protocol --all-features
cargo test -p milkdrift-capability-host --all-features
cargo test -p milkdrift-redb-store --test contracts --all-features
cargo test -p milkdrift-redb-store --test application_state --all-features
cargo test -p milkdrift-managed-linux --test lifecycle --all-features
cargo test -p milkdrift-managed-linux --test linux_mechanism --all-features
```

Preserve/adapt the existing peer cases
`shutdown_and_restart_recover_entered_claim_without_duplicate_adapter_entry`,
`terminal_committed_before_recovery_is_never_replaced_by_uncertainty`,
`uncertain_tombstone_replays_without_becoming_retryable_terminal_evidence`, and
`post_entry_and_post_terminal_cancellation_and_revocation_preserve_truth`. Also retain the
two-repository placement/artifact/restart case. Verify filters discover the intended cases; a
zero-test command is not a pass. Use the adopted runtime/control focused recovery targets when those
owners change. The release-only longevity/full-system lanes remain phase 07's integrated work.

Run the frontend's phase-01 locked install, type/Svelte check, build and browser test commands from
`apps/web/package.json`, recording their exact invocations. Extend its real-browser suite rather
than adding an unrelated mock app. The Rust actual-binary harness starts A and execution-only B
with separate temporary roots, fixture secrets and Rust process helpers; browser tests use their
public endpoints and the supported HTTPS/proxy arrangement where required. Control the remote
response break in the test transport, not by editing stores or faking successful UI state.

In that browser, continue the production research/setup/criticism journey: administer B's offer,
import it on A, invoke a remote task, close the client, interrupt the response, restart/reconnect,
and open a fresh client session. Discover the same workflow, remote uncertainty/settlement, method
origin and earlier result through search. Confirm another client sees A's persisted topology without
receiving B credentials. Exercise retention through a controlled clock and find the truthful expired
result and protected pending input. Repeat remote-tool selection and retained search with the
different project brief used by phases 03/04; no animation-specific scheduler or environment logic.
Record screenshots/traces and exact binary/configuration identities without secrets. These are
plumbing and browser claims, not proof that a real model produced good research.

## Checkpoints, documentation and handoff

Make coherent tested commits as boundaries become usable, for example connection/credential
administration across clients; remote reconnect/recovery and host regressions; retention/accounting
plus search/expiry; and integrated browser corrections/docs. Keep smaller valid checkpoints when
appropriate. No giant final-only commit, automatic squash/rebase, push or unrelated cleanup.

Update implemented facts in product status and architecture, public API/protocol and daemon/peer
operation references, maintained CLI/operator examples and the verification-evidence owner. Explain
credential issuance/rotation, directional setup, remote uncertainty, expiry versus protected state,
receipt capacity and the limit of tested topology. Keep selected design distinct from newly executed
evidence; update ADR consequences only if the implementation establishes a concrete correction.
Record remaining physical-host evidence accurately, without adding a permanent task diary outside
the office. Run the documentation contracts, affected CLI documentation checks and diff checks after
the final edits.

The handoff names the resulting commits, actual commands/counts, browser traces, supported connection
flow, retention capacity behavior, preserved host protections and any precise evidence limitation.
State that the full gate is still pending in phase 07. Leave the production advanced journey ready
for the actual user review in phase 06. Stop there; do not substitute agent approval for that review
or automatically begin the next assignment.
