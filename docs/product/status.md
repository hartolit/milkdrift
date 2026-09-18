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
- Independent hosts expose execution-only and workflow-enabled roles. Both admit authenticated
  direct process and fresh-model calls with explicit inputs, bounded public upload/download,
  durable replay and honest host-invocation artifacts. Execution-only composition constructs no
  runtime, control service or workflow workers. Origin workflows can use the same serving
  operations remotely with staged inputs and their existing controller reservation.
- The CLI covers blueprint/sequence authoring, run/proposal/controller/peer/layout control,
  retained-work resolution, bounded inspection, verified create-new downloads, wait/follow deadlines,
  and stable machine output. [Production examples](../../examples/operator/README.md) provide
  fresh-directory setup and ordinary process/model workflows.
- Causal context uses bounded historical discovery, explicit branch/join/subworkflow visibility,
  exact provenance, authority/sensitivity checks, deterministic budgets and omissions, and
  selected-only materialization. Required-evidence checks continue after selection stops, and
  protected omission identities and sizes are redacted regardless of the reported reason.
  Model session declarations must agree before work is claimed. Retries retain the frozen selection.
- Redb implements journal/index/workspace/account transactions, optional verified snapshots,
  content-addressed artifacts, application receipts/layouts/proposals/audit, peer records and
  tombstones, bounded retention, and resumable administrative integrity scans.
- Explicit daemon `storage-admin` operations inspect blocked generations through a private
  database copy under the source lock, without runtime recovery or source-byte modification.
  Bounded diagnostics redact retained context. Offline backup/verify/restore preserve database,
  artifacts and execution materializations; restored copies carry an execution guard. The
  [storage operations guide](../operations/daemon.md#offline-storage-administration) owns permissions,
  limits and the separate operator decision required before activation.
- Local processes support byte-pinned argv profiles, isolated materialization, explicit inputs and
  outputs, bounded streams, cancellation, and platform ownership. Post-spawn reporting and setup
  failures retain child/I/O ownership through termination and joining, including unwinding;
  reporting errors propagate without manufacturing terminal evidence. Model adapters implement
  OpenAI-compatible chat and native Anthropic mappings through bounded HTTP/SSE.
  Model preparation freezes the exact encoded request before durable entry intent and rechecks
  authority afterward. Proven local refusals end as rejected attempts without a provider request
  or controller reservation. Complete-response reporting loss remains distinguishable from
  preparation refusal while retaining conservative uncertainty when no terminal is durable.
- Prompt sequences compile trusted-process coding, verification, review, and remediation stages
  into ordinary revisions on the same daemon/control path. Separate control-capability acceptance
  tasks check verifier checkpoint reports and usable reviews before branches permit continuation.
  Invocation completion, accepted result, and workflow terminal remain distinct in inspection.
  [Result acceptance](../guides/result-acceptance.md) explains the supported purpose contracts and
  their limits. Historical reasons and revision identities remain unchanged.
- Controller libraries implement durable policy assessment and cumulative accounts across runs and
  descendants. Establishment binds the declared originating run. Final-entry reservations and
  entry intent commit atomically; artifact publication
  and logical-byte charges also commit atomically. Account revisions retain replayable predecessor
  evidence. The daemon composition installs the single control-owned lifecycle before recovery
  when `controller_activation = "enabled"` is explicitly configured. Startup remains disabled
  by default. The integrated review accepts the finite activation prerequisites within the
  operating scope recorded below. Supported text profiles now freeze operator billing, byte-BPE
  input bounds and the selected total-output control with the prepared request. Explicitly unbilled
  completions settle tokens without a monetary report; text tariffs retain calculated and reported
  cost separately. The shared account accepts explicitly declared logical model tokens; unspecified
  units and unsupported bounds refuse entry. Conflicting charges retain unresolved raw evidence.
  Run/controller reads expose
  the same canonical cumulative account; ordinary runs report
  inactive accounting. [Daemon operation](../operations/daemon.md#controller-activation) explains
  configuration and recovery consequences.

Current exact versions follow. Owning constants, strict readers, and golden tests determine these
values; repository contracts check the version cells against source.

| Contract or durable family | Current version | Read behavior |
| --- | --- | --- |
| Capability descriptor/events/cancellation / resolved snapshot | 1 / 3 | Only snapshot v3 is supported; exact typed placement is required. |
| Invocation request | 2 | Context-free v1 migrates unambiguously. |
| Blueprint revision and mutation | 2 | v1 refused. |
| Context manifest | 2 | v1 refused; model envelope remains independent. |
| Model document / task / response / endpoint profile | 1 / 1 / 1 / 2 | Endpoint v1 refused; explicit billing and counting required. |
| Explicit continuation companion | 1 | Exact artifacts and journal anchors; unknown/future documents refused. |
| Proposal / workflow-control command / risk policy / controller policy | 1 / 1 / 1 / 2 | Controller policy v1 refused; currency is explicit or absent. |
| Prompt-sequence import | 3 | v1/v2 refused; existing blueprint history unchanged. |
| Result acceptance contract / decision | 1 / 1 | Explicit purpose; no implicit policy on generic model tasks. |
| Run command / run event | 1 / 3 | Supported event v1/v2 variants remain readable; obsolete nested snapshots are refused. |
| Authority grant / authorization decision | 4 / 2 | Earlier grants refused. |
| Authorized-command wrapper / command result | 1 / 2 | Result v1 reads only closed internal records. |
| Projection snapshot envelope / runtime payload | 2 / 4 | Old/invalid optional checkpoints replay from journal. |
| Administrative integrity cursor | 2 | Exact supported cursor. |
| Peer hot record / compact tombstone | 4 / 2 | Exact current caller/origin meaning; older records refuse. |
| Redb internal document format / physical schema | 17 / 12 | Older/future stores refused; no migration. |
| Application command receipt / layout record | 1 / 1 | Exact supported contracts. |
| Local-process profile / host materialization | 2 / 1 | Process v1 refused. |
| External control / authenticated cursor | 2.8 / 2 | Only the exact current protocol and cursor forms are accepted. |
| Peer protocol and catalog messages | 1.4 | Earlier minors refused. |
| Daemon configuration | 10 | TOML; JSON and earlier versions refused. |
| Layout document / CLI JSON output | 1 / 2 | CLI schema 1 refused. |

## Limitations now

- Execution-only startup omits runtime, control and workflow workers and can serve incoming peers.
  Authenticated clients can discover, prepare, submit, inspect, cancel and follow direct process
  and fresh-model invocations, and upload/download bounded artifacts through the public API/CLI.
  Direct selection freezes explicit inputs without workflow coordinates; direct continuation refuses.
  Local and serving work share prepared execution while retaining
  their distinct durable owners. Serving artifacts use host-invocation accounting; imports retain
  authenticated foreign provenance. Role removal refuses active workflow obligations and preserves
  closed history for offline inspection. Managed deployment and real two-machine qualification
  remain separate from the deterministic loopback binary evidence.
- Managed installations, persistent resource inventories and generation holds, worker isolation,
  protected adaptive agreements, service-execution delegation, published invocation/run linkage,
  and evaluated method learning are not implemented. Existing prospective reconciliation and result
  acceptance do not establish those guarantees. The current risk classifier requires approval for
  node replacement and other elevated changes; it has no protected editable-scope policy.
  [Architecture](../architecture.md) and ADRs 0039–0041 describe these accepted additions, not supported APIs.
- Earlier selection-policy-version-1 manifests remain readable, but omissions retaining ambiguous
  identities or sizes and stopped required evidence cannot authorize reuse. Retry and startup refuse
  those retained records without rewriting their bytes. An unsafe active lease prevents daemon
  ordinary startup. Explicit offline inspection supports diagnosis and preservation. Authenticated
  `--recovery` controls permit reviewed prospective safe-restart proposals with execution disabled;
  normal restart validates the repaired generation before replacement work can run. Missing/corrupt
  evidence and unsafe effects still require their existing refusal/resolution paths. See
  [daemon operations](../operations/daemon.md#startup-and-readiness). The corrected selector emits
  policy version 2;
  [ADR 0031](../decisions/0031-context-enforcement-and-retained-evidence.md) explains this distinction
  from the unchanged manifest schema. Model session agreement is checked before claiming work;
  both provider mappings support `Fresh` and bounded exact-reference `ExplicitContinuation`.
  Continuation is restricted to authorized causal text history and complete tool exchanges in the
  same run and visible scope lineage. Provider-managed sessions and non-Fresh process tasks remain
  refused. See [the continuation guide](../guides/model-continuation.md).
- Combined real coding-agent/model interoperability is qualified for the exercised local Windows
  Codex/Bonsai configuration. This run does not qualify thinking mode: the operator subsequently
  reported disabling it, and effective server settings were not captured per request.
  Explicit controller activation is qualified separately below. Stop behavior is fail-at-bound,
  and ambiguous multiple proposer occurrences are refused.
- Trusted processes have daemon-account privileges. No sandbox, network isolation, CPU/memory
  quotas, malicious-descendant containment, universal atomic hashed-handle execution, directory
  artifacts, writable shared mounts, or complete non-Unix process-tree cancellation is claimed.
  Unowned descendants can retain inherited pipes, but local nonblocking I/O now interrupts at
  the cleanup deadline and joins its workers. Missing EOF remains uncertain. Current platform
  execution evidence is recorded below.
- Peers require operator connectivity; the daemon listener is loopback-only. There is no discovery,
  NAT traversal, coordinator, automatic CA/internal mTLS mapper, consensus, shared database,
  model synchronization, or automatic transfer of every artifact. Grants/profiles/relationships
  generally require validated restart; no dynamic local reload exists.
- Models have no provider discovery, tokenizer/pricing service, generic file parts, managed
  sessions, or OpenAI Responses mapping. Reasoning controls have only the limited mappings in the
  [adapter feature matrix](../../adapters/model-provider/README.md#choose-features-the-endpoint-actually-supports);
  there is no explicit thinking on/off task control. Cancellation cannot prove remote termination. Post-entry
  timeout, response loss, and malformed/truncated streams preserve uncertainty rather than successful
  partial artifacts or automatic unsafe retry. Sequence stages are process-only; checkpoint
  capabilities and automatic distributed dogfood are absent.
- There is no global event firehose, configuration/audit/shutdown
  route, general plugin framework, context search service, or optimized lifetime attempt index.
  Retained attempts use verified occurrence anchors; retired attempts use bounded journal passes.
  Missing optional checkpoints can still require full projection replay.
- Task placement supports exact peer sets and localities through admission, frozen selection, and
  entry. It provides no discovery, tag selector, shared checkout, or cluster scheduling. Remote output
  observations use authorized core artifact transfer; exact artifact inputs and manifests are
  staged before remote submission. Workspace-value references must first become explicit portable
  inputs; remote adapters refuse them. The
  [peer guide](../operations/peers.md#pin-tasks-to-approved-hosts) explains the two-host workflow.
- Active state grows with legitimate live obligations, not just elapsed history. Cold receipts and
  peer tombstones grow for the store generation. No storage migration, online destructive rotation,
  export/delete operation, automatic proposal-index rebuild, whole-database authenticity, rollback
  protection, or filesystem power-loss qualification is claimed.
- No UI is implemented or authorized. Actual graceful OS-signal evidence is qualified on the
  hosted Linux runner; forced Windows child termination does not qualify that platform claim.

## Current validation/evidence snapshot

### Independent host execution

Assignment 01 is reviewed and accepted on Windows x86_64/MSVC with
Rust 1.95.0. The local gate after the process-shutdown correction passes: 833 workspace tests,
24 doctests, all 24 repository contracts,
formatting, all-target/all-feature checking, warning-denying Clippy/rustdoc, dependency audits and
test discovery. Five manual tests remain ignored. Commands and results are under
`target/ci-repair/`, with the independent-host binary evidence under `target/adaptive-hosts/review/`.
The [assignment handoff](../development/virtual-office/adaptive-hosts/handoffs/01.md) records the
resulting commits, corrected operator grant, verification and initial timeout failures.
Default/all-feature API inventories for fourteen affected libraries were reviewed against their
actual consumers; fixture entry, conformance and unjournaled clock helpers remain outside the
ordinary host API.

Hosted platform run 35317306020 passed Linux but failed macOS process shutdown and Windows fixture
startup. Shutdown now leaves signalling to the execution monitor; the cleanup fixture separates
bounded startup from its three-second shutdown assertion. The delayed-start regression reproduces
the old Windows failure and passes after correction. The corrected macOS path and hosted Windows
timing still require a fresh CI run; local checks do not qualify those hosted results.

The actual daemon/CLI independent-host scenario verifies direct process/fresh-model output,
public input upload and artifact download, replay/restart, real remote workflow origins and
controller settlement. A lost model response retains the exact outstanding allowance after
restart without another external entry. The [evidence guide](../development/verification-evidence.md#independent-host-execution)
owns reproduction and measured idle-role costs. This deterministic same-machine evidence does
not qualify live models, non-loopback deployment, Linux resources or the physical UM790 scenario.

### Retained operational qualification

The integrated operational review is based on `f80cce80725777cc364b71db9ce81e2baff22a85`
plus the reviewed operational changes on Windows x86_64/MSVC, Rust 1.95.0. Raw commands,
source/binary identities, reports and logs are retained under `target/acceptance-08/` and
`target/review-commit/`. The [verification guide](../development/verification-evidence.md)
owns reproduction and evidence scope.
These retained results remain scoped to their own source and environment; the
[roadmap](roadmap.md) owns the remaining finite implementation work.

### Integrated operating decision

Explicit controller activation is accepted for the implemented bounded contract. Ordinary builds
install the existing lifecycle only with `controller_activation = "enabled"`; default startup
remains disabled. No live daemon or model-server configuration was changed. The accepted scope
combines current deterministic refusal/recovery evidence with the retained authorized real local
controller loop. It is not a general provider, model-quality, capacity or power-loss qualification.

The actual daemon/CLI loop retains failed work and rejected verification, selects causal review
evidence, submits an ordinary proposal, requires a separate actor's approval, applies a prospective
revision, and accepts verified repair. Two model calls share one publicly inspected cumulative
account; a third model request is refused before transmission. A new child, revision, retry or
restart cannot reset that account. Per-request permission remains separate from cumulative allowance.
Disabled recovery, revoked approval, concurrent admission, unknown metering, entered-process crash,
exact archived replay and compaction retain their existing refusal/accounting assertions.

The real-loop evidence is `target/review-03a-real/report.json`, accepted against the corrective
implementation committed as `d537059`; the reviewed account, lifecycle and metering owners are
unchanged by later operational work. It used byte-pinned Codex CLI `0.154.0-alpha.6.2` and LM Studio
`prism-ml/bonsai-27b:2` on the authorized local endpoint. Two direct calls settled 6661 input / 102
output tokens, four process entries and 45487 logical artifact bytes, zero provider spend, absent
currency and no outstanding reservations. Accepted repair precedes the deliberate third-call
refusal; the final failed workflow is truthful stop evidence. The report retains exact source,
binaries, profiles, observed server defaults, operator declarations and unknowns. Pre-run inspection
of thinking enabled is not per-request attestation or a controlled thinking comparison. Agent-internal
calls remain outside the direct-model meter. The integrated review reused this real controller report.

### Retained integrated checks

The acceptance checks exercise ordinary operator use, Fresh beside ExplicitContinuation, accepted
result gates and model preparation refusal through actual binaries. Production-host tests cover two
exact approved peers, verified returned artifacts, reconnect/reopen without duplicate entry, blocked
read-only inspection, guarded backup/restore, prospective recovery, and inherited-pipe shutdown.
The latter preserves uncertainty and unsafe-retry refusal. These are local loopback/software cases.

Historical-query measurements at 128, 1024 and 4096 settled occurrences found an unnecessary
lifetime execution map and repeated attempt scans. The daemon now uses existing verified occurrence
anchors and retains one owning execution. On the same reopened stores, near-start/end reads at the
largest size fell from about 1.32 seconds to 32–35 milliseconds, with identical public results.
Broad browsing still reads every requested page. Bounded causal discovery selected the same early
artifact at all sizes. No new index, cache, public Rust item or serialized version was introduced;
see the [measurement](../development/verification-evidence.md#historical-query-cost) and its limits.
Historical reads also recognize recovery and reconciliation remediation, preserving each
occurrence's creation revision across later pins in both anchored and full-history reconstruction.
Windows fixture corrections explicitly restore blocking accepted TCP sockets and allow end-to-end
peer scheduling variance while preserving process deadlines and every placement/entry assertion.

At that integrated review, the complete local gate passed: 802 workspace tests, 24 doctests, all 24 repository contracts,
formatting, all-target/all-feature checking, warning-denying Clippy/rustdoc, dependency audits and
test discovery. All three actual-binary scenarios passed in default builds. The remediation-history
regression fails before correction and passes afterward. Historical-owner and omitted-activation
fault evidence remains in `target/acceptance-08/final/`; current commands/results are in
`target/review-commit/`. Those results belong to that review's checkout. Five manual longevity tests remain separate from ordinary discovery.
Regenerated default/all-feature daemon API inventories match the retained review with no exported
changes.
The evidence guide maps applicable retained longevity results and the finite controller checklist.

### External and platform scope

The earlier combined external session at clean candidate `8c6cdb9137bc8f688fd6cc800d3adf416d4f4625`
qualifies its Windows Codex CLI `0.153.4` / LM Studio Bonsai configuration, selected context, linked
artifacts and settled restarts. It does not qualify thinking mode, peers, graceful signals or power
loss. That review's ordinary smokes for `prism-ml/bonsai-27b` and `prism-ml/bonsai-27b:2` pass through
the default daemon/CLI binaries with a requested 4,096-unit allowance. Both produce final text and
usage, preserve selected/omitted context and reopen without another attempt. Reports are under
`target/review-commit/bonsai-1/` and `bonsai-2/`. Server settings were unchanged and effective
thinking settings remain unknown. These runs do not exercise live continuation or qualify the
combined external-agent boundary. Earlier deadline/empty-review results remain negative evidence in the
[retained scenario records](../development/verification-evidence.md#actual-binary-scenarios).

The review-recorded [quality run](https://github.com/hartolit/milkdrift/actions/runs/34595978663)
and [platform run](https://github.com/hartolit/milkdrift/actions/runs/34595978543) at
`aa77c483626b100e0f550c63648875ba28182238` are confirmed successful prior evidence, not newly run
checks. The later [process platform run](https://github.com/hartolit/milkdrift/actions/runs/35023792743)
at `515337f` passes on Ubuntu 24.04, Windows 2025 and macOS 15. Its exact scope is complete workspace
checking, selected domain/protocol/client/shared-host/process suites, and the daemon inherited-pipe
shutdown/restart case. This closes the old pending macOS note for those cases, not a complete
Windows/macOS workspace test gate or physical multi-machine peer qualification.

Retained hosted Linux [benchmark/operational evidence](https://github.com/hartolit/milkdrift/actions/runs/34065354712)
covers twelve scenarios, overload recovery, bounded frontier/storage observations, fresh-cursor
stream reconnect and graceful shutdown. [Release stress](https://github.com/hartolit/milkdrift/actions/runs/34065354738)
passes at the same recorded implementation. Windows forced-child termination is not graceful-signal
proof. The [mutation campaign](https://github.com/hartolit/milkdrift/actions/runs/34065354713) covers
seven groups / 15 partitions; the evidence guide preserves reviewed classifications and independent
retention requalification. These reports remain scoped to their own commits and environments.

Configured CI lanes are not executed evidence. None of these checks establish sandbox strength,
arbitrary provider interoperability, escaped-descendant containment or filesystem power-loss behavior.
