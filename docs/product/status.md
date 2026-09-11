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
  selected-only materialization. Required-evidence checks continue after selection stops, and
  protected omission identities and sizes are redacted regardless of the reported reason.
  Model session declarations must agree before work is claimed. Retries retain the frozen selection.
- Redb implements journal/index/workspace/account transactions, optional verified snapshots,
  content-addressed artifacts, application receipts/layouts/proposals/audit, peer records and
  tombstones, bounded retention, and resumable administrative integrity scans.
- Local processes support byte-pinned argv profiles, isolated materialization, explicit inputs and
  outputs, bounded streams, cancellation, and platform ownership. Post-spawn reporting and setup
  failures retain child/I/O ownership through termination and joining, including unwinding;
  reporting errors propagate without manufacturing terminal evidence. Model adapters implement
  OpenAI-compatible chat and native Anthropic mappings through bounded HTTP/SSE.
- Prompt sequences compile trusted-process coding, verification, review, and remediation stages
  into ordinary revisions on the same daemon/control path. New revision reasons name the accepted
  import schema; historical reasons and revision identities remain unchanged.
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

- Earlier selection-policy-version-1 manifests remain readable, but omissions retaining ambiguous
  identities or sizes and stopped required evidence cannot authorize reuse. Retry and startup refuse
  those retained records without rewriting their bytes. An unsafe active lease prevents daemon
  startup and HTTP service; no automatic repair is available through the CLI/API. See
  [daemon operations](../operations/daemon.md#startup-and-readiness). The corrected selector emits
  policy version 2;
  [ADR 0031](../decisions/0031-context-enforcement-and-retained-evidence.md) explains this distinction
  from the unchanged manifest schema. Model session agreement is checked before claiming work;
  supported provider mappings still accept only `Fresh`. Process session intent remains
  capability-specific and does not implement continuation.
- Combined real coding-agent/model interoperability is qualified for the exercised local Windows
  Codex/Bonsai configuration. This run does not qualify thinking mode: the operator subsequently
  reported disabling it, and effective server settings were not captured per request.
  Continuous controller activation has separate prerequisites and remains unavailable in production;
  library stop behavior is fail-at-bound, and ambiguous multiple proposer occurrences are refused.
- Trusted processes have daemon-account privileges. No sandbox, network isolation, CPU/memory
  quotas, malicious-descendant containment, universal atomic hashed-handle execution, directory
  artifacts, writable shared mounts, or complete non-Unix process-tree cancellation is claimed.
  Unowned descendants can retain inherited pipes and delay I/O joining during cleanup.
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

The reporting-cleanup implementation passes the complete local Windows/MSVC gate with 713 tests
and all 24 repository contracts; the five manual longevity tests remain ignored in that run.
Focused regressions establish immediate-child termination after initial,
stdout/stderr progress, and heartbeat rejection, including an unread stdin pipe and reporter panic.
Private lifecycle tests observe completion of every started I/O worker and retained cancellation
registration during partial startup and unwinding. Cancellation, timeout, output-limit termination,
and shutdown regressions also pass. The later
[platform run at `005ccb2`](https://github.com/hartolit/milkdrift/actions/runs/34544490423)
passes Ubuntu and Windows, but macOS fails two private lifecycle tests that inspect child absence
immediately after worker gate arrival. The local test correction waits for absence with the gates
held; confirmation on macOS remains pending. That run's Unix owned-descendant reporting cases pass.

The context-policy enforcement and import-label findings are resolved by `a24671a`. Its complete
local Windows/MSVC gate passes with 725 tests and all 24 repository contracts; five manual longevity
tests remain ignored. Production regressions cover stopped required evidence, serialized protected
omissions, legacy retry/reopen refusal, model
session agreement for inline/artifact requests and category-free historical snapshots, and exact
import revision identity. A bounded runtime/host/HTTP matrix proves Fresh completion and no request
for contradictory or unsupported continuation declarations. All 42 default/all-feature library API
inventories are reviewed; the sole added export shares the model document byte ceiling with runtime.
Focused revalidation passes 28 context/session/import tests, five affected doctests, formatting,
warning-denying rustdoc, and documentation contracts. The full gate is reused for unchanged
executable code; subsequent changes clarify explanations and remove resolved office topics.
These checks do not add real-provider, process-session, or cross-platform qualification.

Independently accepted combined real external evidence qualifies clean candidate
`8c6cdb9137bc8f688fd6cc800d3adf416d4f4625` on Windows x86_64/MSVC with Rust 1.95.0.
Byte-pinned `codex-cli 0.153.4` used LM Studio's `prism-ml/bonsai-27b:2`; the direct model task used
`prism-ml/bonsai-27b`. Both scenarios qualify: fresh coding-agent work, separate verification and
prospective remediation, selected causal context, final model text and usage, linked artifacts,
and settled-boundary restarts without duplicate entry. The candidate passed the complete local
gate with 709 tests, 24 doctests, and all 24 repository contracts; five manual longevity cases
remained ignored. This establishes no model-quality, thinking-mode, peer, new-platform,
graceful-signal, or filesystem power-loss claim. The
[retained evidence](../development/verification-evidence.md#actual-binary-scenarios) identifies the
accepted report, independent inspection, and private session. Controller activation remains separate.

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
