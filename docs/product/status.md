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
  Peer startup refuses a recovery continuation that reports no recovered claims; admission stays
  closed instead of repeatedly requesting an empty page.
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
  descendants. Establishment binds the declared originating run. Final-entry reservations and
  entry intent commit atomically; artifact publication
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

- Context policy session intent is not compared with the model request's session. With
  `StopAtFirstOverflow`, an optional overflow can skip later eligible required-evidence checks.
  Omission-reason precedence can also retain protected reference metadata. The
  [runtime builder](../../crates/runtime/src/context.rs) describes these implementation gaps;
  [the source finding](../development/virtual-office/whiteboard/issues/context-policy-enforcement.md)
  records the follow-up. Existing tests do not establish the missing combined cases.
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

The pre-UI kernel is maintainership-ready. Broad architectural cleanup is frozen under the
[roadmap](roadmap.md); continuous controller activation remains a separate qualification decision.

Implementation commit `0b251625feec6848e650694ff57e2094c88413b5` passes the complete local Windows/MSVC
gate: formatting, all-target/all-feature checking, 682 workspace tests, Clippy, warning-denying
rustdoc, dependency audits, test discovery, and all 21 repository contracts. Five manual longevity
tests are ignored in the ordinary suite and pass separately in release mode. All 21 library public
APIs are inventoried under default and all features, with no default test-helper exports.

The fresh-directory starter, ordinary byte-pinned Windows process, verified artifact download,
exact replay/conflict, settled restart, and failure path pass through actual binaries. Release
headless daemon/CLI evidence and deterministic multiline model evidence also pass. Portable process
fixtures retain explicit Python/Git prerequisites. These software checks do not qualify filesystem
power loss or real coding-agent interoperability.

The ordinary model scenario passes against separately managed LM Studio with
`google/gemma-4-12b-qat`, explicit response/idle bounds, and a 4,096-unit output allowance. Evidence
establishes final text, streamed progress, supplied response identity/usage, selected and omitted
context, linked artifacts, and restart without another attempt. A controlled post-entry close
separately proves retained uncertainty and unsafe-retry refusal. This model-only smoke does not
qualify the combined real-agent/model gate. Endpoint replacement remains an operator profile change
through the [same guide](../guides/local-model-endpoint.md).

A bounded Gemma/Bonsai fork/join experiment completes both author branches and preserves exact
replay/restart. Its Bonsai reviewer reports a length finish at 2,048 output units with no final text;
the stronger useful-review check therefore fails. The deterministic actual-binary counterpart
verifies that both distinct author texts reach the reviewer while private response metadata remains
excluded. Workflow success is not evidence that a model produced a useful review.

Release receipt, peer, controller-lifecycle, controller-admission, historical-frontier longevity,
projection stress, and effect-worker shutdown proofs pass. Hosted Linux
[benchmark and operational evidence](https://github.com/hartolit/milkdrift/actions/runs/34065354712)
passes all twelve scenarios, overload recovery, bounded storage/frontier observations, a real
fresh-cursor stream reconnect, and graceful shutdown. Hosted
[release stress](https://github.com/hartolit/milkdrift/actions/runs/34065354738) also passes at the
same implementation commit. Windows signal refusal is not counted as graceful-shutdown proof.

Hosted [Linux quality](https://github.com/hartolit/milkdrift/actions/runs/34065354710) and
[Windows/macOS/Linux platform checks](https://github.com/hartolit/milkdrift/actions/runs/34065354722)
pass at that commit. Linux quality includes the complete workspace and actual daemon/CLI/model
scenarios; the platform matrix runs complete workspace checking and its declared selected suites.
All seven [mutation groups](https://github.com/hartolit/milkdrift/actions/runs/34065354713) complete
across 15 partitions (656 campaign entries), with 3 exact reviewed classifications, no unclassified
survivors, no timeouts, and inspected compiler diagnostics. Partition selections match their full
groups without gaps or overlaps. Separate controller-origin fault injections are caught; the
[evidence guide](../development/verification-evidence.md) explains constructor exclusions.
Two retention entries whose early fixture timeouts prevented later owner tests were independently
requalified by rerunning the exact mutations through the existing storage contract.

[Verification evidence](../development/verification-evidence.md) owns commands, pinned workflows,
report meaning, and classification rules. Configured workflows are not executed evidence. Raw
outputs, source commit/tree/dirty identity, runtime observations, structural metrics, and API
inventories belong under ignored `target/` paths or CI artifacts, not active documentation.
