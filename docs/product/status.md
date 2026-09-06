# Status

This document owns current implementation, limitations, exact versions, and qualified evidence.
[Architecture](../architecture.md) owns semantics and source ownership; Git and CI retain chronology.

## Implemented now

- The headless Rust workspace provides immutable blueprints/mutations; task, branch, fork/join,
  reducer, bounded repeat, wait/signal/timer, pinned subworkflow, and terminal definitions; durable
  commands, scheduling, recovery, prospective reconciliation, and scoped workspace/artifact state.
- The daemon compiles strict TOML into owner-specific plans, recovers with admission closed, and
  serves authenticated commands, reads, artifacts, and resumable feeds through one bounded owner.
  Process/model/control/peer adapters use exact generations, final authority checks, fixed workers,
  and incremental durable reporting. Capability health is refreshed before observations become
  stale. Pages and streams share authenticated cursor-scope construction. Boundary-clock rollback
  fails closed across restart; fresh clock sampling is serialized with watermark transactions.
- The CLI covers blueprint/sequence authoring, run/proposal/controller/peer/layout control,
  retained-work resolution, bounded inspection, verified create-new downloads, wait/follow deadlines,
  and stable machine output. [Production examples](../../examples/operator/README.md) provide
  fresh-directory setup and ordinary process/model workflows.
- Causal context uses bounded historical discovery, explicit branch/join/subworkflow visibility,
  exact provenance, authority/sensitivity checks, deterministic budgets and omissions, and
  selected-only materialization. Retries retain the frozen selection.
- Redb implements journal/index/workspace/account transactions, optional verified snapshots,
  content-addressed artifacts, application receipts/layouts/proposals/audit, peer records and
  tombstones, bounded retention, and resumable administrative integrity scans.
- Local processes support byte-pinned argv profiles, isolated materialization, explicit inputs and
  outputs, bounded streams, cancellation, and platform ownership. Model adapters implement
  OpenAI-compatible chat and native Anthropic mappings through bounded HTTP/SSE.
- Prompt sequences compile trusted-process coding, verification, review, and remediation stages
  into ordinary revisions on the same daemon/control path.
- Controller libraries implement durable policy assessment and cumulative accounts across runs and
  descendants. Final-entry reservations and entry intent commit atomically; artifact publication
  and logical-byte charges also commit atomically. Account revisions retain replayable predecessor
  evidence. **The production daemon leaves the controller lifecycle uninstalled** pending a current
  qualifying real external-evidence run.

Current exact versions follow. Owning constants, strict readers, and golden tests determine these
values; repository contracts check the version cells against source.

| Contract or durable family | Current version | Read behavior |
| --- | --- | --- |
| Capability descriptor/events/cancellation / resolved snapshot | 1 / 2 | Snapshot v1 retains its original digest and conservative missing-category meaning. |
| Invocation request | 2 | Context-free v1 migrates unambiguously. |
| Blueprint revision and mutation | 2 | v1 refused. |
| Context manifest | 2 | v1 refused; model envelope remains independent. |
| Model document / task / response / endpoint profile | 1 / 1 / 1 / 1 | Exact supported contracts. |
| Proposal / workflow-control command / risk policy / controller policy | 1 / 1 / 1 / 1 | Exact supported contracts. |
| Prompt-sequence import | 2 | v1 refused. |
| Run command / run event | 1 / 3 | Exact event v1/v2 remains readable. |
| Authority grant / authorization decision | 4 / 2 | Earlier grants refused. |
| Authorized-command wrapper / command result | 1 / 2 | Result v1 reads only closed internal records. |
| Projection snapshot envelope / runtime payload | 2 / 4 | Old/invalid optional checkpoints replay from journal. |
| Administrative integrity cursor | 2 | Exact supported cursor. |
| Peer hot record / compact tombstone | 3 / 1 | Hot v2 upgraded on next append. |
| Redb internal document format / physical schema | 14 / 11 | Older/future stores refused; no migration. |
| Application command receipt / layout record | 1 / 1 | Exact supported contracts. |
| Local-process profile / host materialization | 2 / 1 | Process v1 refused. |
| External control / authenticated cursor | 2.3 / 2 | Earlier major/cursor forms refused. |
| Peer protocol and catalog messages | 1.2 | Earlier minors refused. |
| Daemon configuration | 9 | TOML; JSON and earlier versions refused. |
| Layout document / CLI JSON output | 1 / 2 | CLI schema 1 refused. |

## Limitations now

- Combined real coding-agent/model interoperability remains unqualified: no operator-supplied
  byte-pinned real coding-agent profile is available. A separately managed supported model has
  passed the ordinary model scenario, but a clean strict combined report is still required.
  Deterministic helpers and mock endpoints do not qualify. Continuous controllers therefore remain unavailable in production;
  library stop behavior is fail-at-bound, and ambiguous multiple proposer occurrences are refused.
- Trusted processes have daemon-account privileges. No sandbox, network isolation, CPU/memory
  quotas, malicious-descendant containment, universal atomic hashed-handle execution, directory
  artifacts, writable shared mounts, or complete non-Unix process-tree cancellation is claimed.
- Peers require operator connectivity; the daemon listener is loopback-only. There is no discovery,
  NAT traversal, coordinator, automatic CA/internal mTLS mapper, consensus, shared database,
  model synchronization, or automatic transfer of every artifact. Grants/profiles/relationships
  generally require validated restart; no dynamic local reload exists.
- Models have no provider discovery, tokenizer/pricing service, generic file parts, managed
  sessions, or OpenAI Responses mapping. Cancellation cannot prove remote termination. Post-entry
  timeout, response loss, and malformed/truncated streams preserve uncertainty rather than successful
  partial artifacts or automatic unsafe retry. Sequence stages are process-only; checkpoint
  capabilities and automatic distributed dogfood are absent.
- There is no public local artifact upload, global event firehose, configuration/audit/shutdown
  route, general plugin framework, context search service, or optimized lifetime attempt index.
  Historical attempt reads use bounded memory but may scan substantial journal history.
- Task requirements cannot express locality or peer selectors. Revision admission consequently
  refuses grants narrowed in those dimensions; the [authority guide](../operations/authority.md)
  explains supported exact capability/profile/trust-zone constraints.
- Active state grows with legitimate live obligations, not just elapsed history. Cold receipts and
  peer tombstones grow for the store generation. No storage migration, online destructive rotation,
  export/delete operation, automatic proposal-index rebuild, whole-database authenticity, rollback
  protection, or filesystem power-loss qualification is claimed.
- No UI is implemented or authorized. Actual graceful OS-signal evidence is qualified on the
  hosted Linux runner; forced Windows child termination does not qualify that platform claim.

## Current validation/evidence snapshot

Local Windows/MSVC formatting, all-target/all-feature checking, Clippy, warning-denying rustdoc,
dependency audits, test discovery, and 21 repository contracts pass. CLI examples parse, and actual
executables validate bare/relative/absolute configuration paths. The fresh-directory starter,
ordinary byte-pinned Windows process, verified artifact download, settled restart/replay, and
ordinary model example against a controlled loopback endpoint pass, as does the headless daemon/CLI
scenario. Adapter, authority, runtime, context, artifact, and recovery
contracts provide additional software evidence, not real-provider or power-loss qualification.

The Windows workspace suite passes 675 tests with five release-only longevity tests ignored and
run separately. Portable process fixtures use explicit Python/Git prerequisites. The 14 control-plane
tests, three bounded timeline-observer regressions, and peer transport/resource authorization
regression pass. All-library public API inventories pass under default and all features, with no
default test-helper exports.

Release actual-binary headless, deterministic model, and separately managed real-model smoke lanes
pass. The real endpoint was temporary LM Studio with `prism-ml/bonsai-27b`, thinking disabled and
explicit response/idle/output bounds. Evidence establishes final text, streamed progress, response
identity, supplied usage, selected/omitted context, artifact linkage, and duplicate-free restart.
The controlled post-entry close separately establishes retained uncertainty and unsafe-retry
refusal. This model-only smoke does not qualify the combined real-agent/model gate. Server
replacement remains an operator profile change through the [same guide](../guides/local-model-endpoint.md).

Release receipt, peer, controller-lifecycle, controller-admission, historical-frontier longevity,
projection stress, and effect-worker shutdown proofs pass. Hosted Linux
[benchmark and operational evidence](https://github.com/hartolit/milkdrift/actions/runs/34021380874)
passes all twelve scenarios, overload recovery, bounded storage/frontier observations, stream
reconnection, and graceful shutdown. Hosted [release stress](https://github.com/hartolit/milkdrift/actions/runs/34021380881)
also passes. Both use commit `2599839`, whose production source is unchanged in this checkout.

Hosted [Linux quality](https://github.com/hartolit/milkdrift/actions/runs/34021964548) and
[Windows/macOS/Linux platform checks](https://github.com/hartolit/milkdrift/actions/runs/34021964535)
pass at `878a3e3`; Linux runs the full workspace suite and actual daemon/CLI/model scenarios, while
the platform matrix runs its declared selected suites. Six mutation groups pass with reviewed
diagnostics and no timeouts. Complete peer mutation qualification, including the strengthened
authority, catalog-filtering, and disconnected-health assertions, remains pending. Independent readiness and the
architecture freeze remain open until that check passes.

[Verification evidence](../development/verification-evidence.md) owns commands, pinned workflows,
report meaning, and classification rules. Configured workflows are not executed evidence. Raw
outputs, source commit/tree/dirty identity, runtime observations, structural metrics, and API
inventories belong under ignored `target/` paths or CI artifacts, not active documentation.
