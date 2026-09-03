# Status

This document owns current implementation facts, current limitations, and the latest evidence
snapshot. Git, CI, releases, and external audits own chronology.

## Implemented now

- Milkdrift is a Rust 1.95.0, edition-2024 workspace of twenty-two safe-Rust packages. Immutable
  blueprints, append-only execution facts, scoped authority, prospective reconciliation, bounded
  operations, and external capabilities are separate owners.
- The semantic core implements immutable revisions and mutation batches; typed task, branch,
  fork/join, reducer, repeat, wait/signal/timer, subworkflow, and terminal definitions; deterministic
  validation and fingerprints; scoped workspace values/artifacts/budgets; durable run commands and
  event projection; scheduling, execution, recovery, reconciliation, and structured concurrency.
- The runtime executor port has one external-work operation path. Caller-owned workers report each
  observation through the incremental durable reporter; there is no synchronous report-batch
  compatibility path or second validation owner. Production capability-host execution and
  deterministic test executors use that same interface.
- `milkdrift-capability-host` owns the common adapter contract and the sole live registration,
  exact-generation permit, drain, cancellation-routing, and panic-containment path. Authority,
  start, drain, and shutdown hooks are explicit for every implementation. Failed start cannot
  publish a generation; health must preserve the supplied boundary time; cancellation
  acknowledgements must preserve exact invocation/sequence correlation; and shutdown attempts
  every registered adapter even when one fails. Its non-default `test-support` surface provides one
  reusable factory-driven suite run by the local-process, model-endpoint, remote-peer, and
  workflow-control adapters, with declared differences for start replay, stateless health, and
  unknown cancellation.
- One exact immutable grant basis governs run entry, local commands, information-bearing reads,
  pages, streams, artifacts, layouts, capability/provider views, controller actions, and peer
  operations. Humans, services, AIs, and peers use the same authority evaluator and command path.
  Capability and artifact selectors are explicit `Any`, bounded nonempty `Only`, or whole-scope
  `DenyAll`; layout authority is deny-all or shared revisions only. Empty collections never mean
  wildcard, and private actor-owned layouts are not advertised as executable daemon state.
- `milkdrift-daemon` compiles bounded duplicate-safe TOML schema 9 into immutable storage,
  authentication, runtime, adapter, peer, and shutdown plans; the superseded JSON/global-document
  path is refused. It is the single bounded synchronous owner of redb, runtime, control, adapters,
  peers, receipts, layouts, and proposal discovery. A dedicated owner thread sits behind a bounded
  channel. Typed one-shot closure messages remove the former parallel operation/result enums and
  make mismatched HTTP response variants unrepresentable; an owned queue guard and one coherent
  versioned health projection make overload, startup recovery, readiness, periodic maintenance,
  health streaming, and ordered shutdown explicit. The peer service consumes its existing narrow
  execution and artifact ports through daemon-private owner adapters: owner-thread lifecycle work
  executes inline, while durable calls originating in peer HTTP and fixed workers enter that same
  bounded queue and retain typed overload. Weak service-facing store handles prevent router
  lifetimes from extending redb ownership. Peer HTTP admits only a bounded number of synchronous
  service calls off Tokio reactor tasks, and shutdown keeps the owner servicing final peer writes
  while fixed peer workers join. One daemon-owned fallible clock feeds runtime, peer, control,
  artifact, and stream boundaries through the owner queue and redb physical-schema-9 durable
  high-water evidence. Failure or rollback refuses operations and is visible in health/logs. The
  peer execution service owns a closed post-entry classification for adapter failure, missing
  terminal evidence, adapter panic, and interrupted service work; fixed workers retain and retry
  only the exact release-or-uncertainty transition. Clock/store failure, shutdown, replay, and
  restart do not re-enter the adapter. The owner continues serving final worker clock/persistence
  calls while peer and effect workers drain, preventing shutdown from waiting on work that only the
  owner can complete. The CLI is a storage-free client of `milkdrift-control-client`.
  Daemon command adaptation is partitioned into private definition, run, controller, proposal, and
  control-result owners behind one exhaustive protocol route. Attempt inspection separately owns
  current projection lookup, bounded historical folding, authority filtering, and context/provenance
  attachment without changing the public attempt meaning.
- External control protocol 2.2 provides bounded duplicate-safe DTOs, exact negotiation,
  authenticated cursor schema 2, idempotent commands, revisions/diffs, runs/nodes/attempts,
  timelines, proposals, capabilities/providers, authority, peers, artifacts, layouts, health, and
  resumable SSE. Protocol-1 clients are refused. Internal events, redb rows, snapshots, provider
  payloads, process handles, and framework types are not wire contracts.
- Peer protocol 1.2 is transport-neutral. The HTTP adapter owns relationship authorization,
  rotating bearer authentication, fixed endpoints, bounded quotas/workers, catalog generations,
  remote ordinary capability registrations, transactional acceptance/claim/entry, append-only
  observations, cancellation, restart recovery, tombstone replay, and ordinary core artifact
  transfer. It introduces no second workflow truth.
- Redb physical schema 11 and internal document format 14 own transactional journal, history chain,
  indexes, snapshots, workspace accounting, artifacts, application receipts/layouts/proposals,
  security audit, and peer execution/retention. Application receipts move independently from hot
  to cold while preserving exact replay; peer terminal detail compacts independently to replay and
  conflict tombstones. Older/future stores are refused because no migration is claimed.
  Administrative integrity scanning uses one private typed phase driver, one read transaction, and
  one cursor/refusal policy across revision, event, artifact, and index checks.
- Causal context is discovered from a bounded durable tail plus compact projection anchors at the
  frozen journal head. Historical revisions, branch/join/subworkflow visibility, sensitivity,
  authority, selection budgets, omissions, and exact provenance are explicit. Only selected
  content is materialized. Required denied, missing, corrupt, unsupported, or over-budget evidence
  fails before dispatch; retries rebind the prior frozen selection.
  Private discovery state owns projection, journal, explicit-source, and exposure phases; private
  selection state owns final deterministic ordering, omissions, authority, sensitivity, and budgets.
- Models remain external provider capabilities. Provider-neutral task/response and context
  contracts are separate from bounded OpenAI-compatible and native Anthropic mappings. Exact
  profile/model/usage metadata and committed artifacts are recorded without logging prompts,
  responses, bearer values, or resolved secrets. One local-secret adapter resolves only configured
  bounded environment or restricted-file references for authentication, process/model adapters,
  and peer credentials. Local processes use byte-pinned safe-argv profile schema 2, explicit
  inputs/outputs, bounded streams, cancellation, and process ownership. Their native canonical
  roots convert to one pure authority representation supporting Unix absolute roots and ordinary
  Windows drive-absolute roots with exact component containment; ambiguous Windows path families
  fail closed.
- Prompt-sequence schema 2 imports bounded JSON or Markdown, compiles to ordinary blueprints, and
  uses the normal daemon/control/CLI path for validation, execution, verification, review,
  approval, prospective remediation, restart recovery, and historical inspection. Its headless
  dogfood fixture uses fresh byte-pinned processes and a persistent temporary Git repository.
- The control/runtime libraries implement a bounded controller lifecycle using ordinary repeat,
  proposal, reconciliation, authority, and event contracts. One durable account owns controller
  cost, unit, artifact, process-entry, and model-entry ceilings across the controller run and its
  descendants. The final adapter-entry event and account admission commit atomically after one
  exact generation is prepared but before adapter code; artifact metadata and its logical-byte
  charge also commit atomically. Immutable per-account revision evidence replays every total and
  reservation change from its predecessor and exact transition or publication source. Unknown
  bounds, missing terminal usage, uncertainty, currency mismatch, and adapter-contract violations
  fail closed. The production daemon still leaves this lifecycle uninstalled because no current
  qualifying real external-evidence run is available.
- The public Rust surface follows the current policy in
  [`../reference/public-api-policy.md`](../reference/public-api-policy.md). Canonical identities come
  from their semantic owner; deterministic runtime/secret implementations require non-default
  `test-support` features; storage fault hooks require redb `test-admin`; daemon routing and read
  projections remain internal. Runtime scheduling and caller-owned effect execution are separate
  canonical operations with no blocking compatibility driver. Cross-domain UTF-8 truncation and
  canonical `b3_` lexical validation have one implementation in `milkdrift-contracts`, while
  semantic digest types and errors remain domain-owned. There are no UI packages or UI dependencies.
  Repository contracts require exact reviewed exceptions with rationales and bounded ceilings when
  production Rust sources cross 1,000 lines; all Rust sources remain below the 2,000-line backstop,
  and test/evidence sources cannot weaken the production policy.

Current exact versions are:

| Contract or durable family | Version and read behavior |
| --- | --- |
| Capability descriptor/events/cancellation / resolved snapshot | 1 / 2; legacy snapshot v1 reads retain their original digest and conservative missing-category meaning |
| Invocation request | 2; context-free v1 reads migrate unambiguously |
| Blueprint revision and mutation | 2; v1 refused |
| Context manifest | 2; v1 refused |
| Provider-neutral model task/response and endpoint profile | 1 |
| Proposal, workflow-control command/risk policy, and controller policy | 1 |
| Prompt-sequence import | 2; v1 refused |
| Run command / run event | 1 / 3; exact legacy event v1/v2 remains readable |
| Authority grant / authorization decision | 4 / 2; earlier grant forms refused |
| Authorized-command wrapper / command result | 1 / 2; result v1 reads only closed internal records |
| Projection snapshot envelope / runtime payload | 2 / 4; old optional payloads replay from journal |
| Administrative integrity cursor | 2 |
| Peer hot record / compact tombstone | 3 / 1; hot v2 reads are upgraded on the next append |
| Redb internal document format / physical schema | 14 / 11 |
| Application command receipt / layout record | 1 / 1 |
| Local-process profile / host materialization | 2 / 1; process v1 refused |
| External control / authenticated cursor | 2.2 / 2; legacy forms refused |
| Peer protocol and catalog messages | 1.2; earlier minors refused |
| Daemon configuration | 9 TOML; JSON and earlier versions refused |
| Layout document / CLI JSON output | 1 / 1 |

## Limitations now

- External interoperability is not yet proven. This checkout has no operator-supplied real coding
  agent profile and no reachable real supported model endpoint/profile with secret mapping. The
  hermetic external-evidence mode validates the harness but is explicitly non-qualifying. Real
  closure requires a byte-pinned real agent, a real supported endpoint that returns response
  identity and usage, private credential sources, and a clean strict-mode run.
- No hosted workflow has run this final Pass 1 worktree. At source commit `8b7b2e2`, the 2026-09-03
  hosted Linux quality gate and the Ubuntu/macOS platform jobs passed, while the Windows platform
  job failed thirteen local-process tests at the former Unix-only filesystem-authority constructor.
  The local correction has not yet received hosted Windows evidence. Mutation, benchmark, and
  operational workflow executions for this worktree are likewise not evidenced here; local Linux
  or cross-target checks cannot substitute for those runs.
- The local-process adapter provides mediation and ownership, not a sandbox. Trusted processes run
  with the daemon account's privileges. Network isolation, CPU/memory quotas, malicious descendant
  containment, atomic hashed-handle execution on every OS, directory artifacts, writable shared
  mounts, and complete non-Unix process-tree cancellation are not claimed.
- Peer support has no discovery, NAT traversal, coordinator, automatic CA, internal mTLS mapper,
  consensus, shared database/workflow truth, model synchronization, or automatic transfer of every
  runtime artifact. The daemon listener is loopback-only; nonlocal deployment needs operator-owned
  private connectivity or HTTPS termination. Relationship/grant/profile changes generally require
  validated restart.
- There is no dynamic local adapter/profile/grant reload, public local artifact upload API, global
  event firehose, public configuration/audit/shutdown route, generalized plugin framework,
  optimized lifetime attempt index, or context search service. Historical attempt reads trade
  bounded memory for journal scan time.
- Continuous controllers are not production-supported by the daemon until an operator-supplied,
  current qualifying real external-evidence run closes the activation gate. The final-entry ledger
  has passed the local hostile, mutation, longevity, and independent-review lanes. The library
  lifecycle's only stop behavior remains immutable fail-at-bound. Multiple active
  controller occurrences deliberately prevent ambiguous proposer attribution. Prompt sequences
  currently use trusted-host process stages; model-backed sequence stages, checkpoint
  capabilities, and automatic distributed dogfood are not implemented.
- The provider adapter does not implement provider discovery, tokenization, pricing, generic file
  parts, managed sessions, or the separate OpenAI Responses API. Cancellation cannot prove remote
  termination, and malformed/truncated streams do not produce a successful partial artifact.
- Active projection memory is bounded by legitimate live state, not a universal constant. Cold
  application receipts and peer tombstones preserve replay for one store generation and grow until
  explicit offline rotation. No storage migration, online destructive rotation, export/delete
  operation, automatic proposal-index rebuild, whole-database authenticity, or rollback protection
  is claimed.
- No browser client, desktop application, or other UI is implemented or currently authorized.

## Current validation/evidence snapshot

- Current local Linux worktree snapshot (2026-09-03): formatting, all-target/all-feature checking, the complete
  workspace test and doctest suite, Clippy with warnings denied, rustdoc with warnings denied,
  `cargo deny check`, `cargo machete`, duplicate dependency inspection, and test inventory all pass.
  Five explicitly manual longevity/storage-bound tests remain ignored in the ordinary suite.
- All five manual longevity lanes pass separately in release mode: 10,001 receipt commits across
  hot/cold turnover and restart, two-daemon peer retention/restart, controller checkpoint/restart,
  controller reservation/artifact turnover and restart, and the 2,049-occurrence runtime frontier.
  The seven pinned current-source mutation shards enumerate 642 mutants after the controller shard
  was extended across final-entry admission, account, and persistence owners. The complete current
  campaign catches 606; 35 are compiler-unviable, and one pre-existing runtime reconciliation
  mutant retains its exact reviewed unreachable-contract classification. There are no unclassified
  survivors or timeouts.
- The simplified all-feature `cargo-public-api` inventory is retained under `target/public-api` for
  per-package review. This boundary intentionally adds the narrow admission-envelope,
  prepared-entry, and durable controller-account contracts shared by their real owners. The daemon
  still exposes its effective configuration as an opaque compiled plan rather than validated raw
  fields. Lint allowances remain individually justified at their use sites.
- The maintained evidence suites cover immutable/schema readers, validating constructors, hostile
  bounds, exact idempotency/conflict, crash/reopen and deterministic fault boundaries, projection
  replay, reconciliation, causal context, process/model adapters, controller lifecycle, application
  and peer retention, artifact integrity/ranges, authentication/cursor revocation, daemon overload,
  shutdown, and a loopback two-daemon remote execution.
- `milkdrift-evidence` owns repeatable storage/projection/context/artifact/daemon measurements,
  the Cargo-native mutation shard/classification runner, and operational reports under
  `target/evidence`. The current dirty-tree 256-operation run accounts for all requests under
  overload, reopens bounded receipt and peer state, reconnects the stream, and shuts down without
  task growth; it records Git commit/tree/dirty state plus `rustc -vV` and is not release
  qualification. The hermetic external-evidence report remains explicitly non-qualifying for real
  interoperability.
- Raw API inventories are generated under `target/public-api`; generated public-API reports and
  pass histories are not source documentation.
- `cargo machete` reports no unused dependency. `cargo deny` accepts the maintained transitive
  `syn` 2/3 split; duplicate-tree inspection also reports the maintained transitive `getrandom`
  0.3/0.4 split. No duplicate Milkdrift package or second HTTP/TLS/async stack is present.
