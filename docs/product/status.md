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
- Managed installation commands share one semantic owner through the authenticated API/client/CLI
  and `milkdrift.resources` capability. Approved recipes, exact generation holds, transferable editing
  claims, pending changes and command receipts survive restart in redb. The production Linux adapter
  prepares workload-independent working storage, temporary workers, and optional owned Quadlet llama-server or
  attached endpoint. It requires explicit resource protection and refuses unsupported prerequisites.
  Deterministic lifecycle/ledger/conformance/backup tests are distinct from the gated real-Podman
  lane. Desktop and UM790 Podman 6.1.2 are exercised, including owned CPU model lifecycle and an
  orderly host reboot with completed-call replay. Managed GPU and pressure qualification remain pending; see
  [managed operations](../operations/managed-linux.md).
- Workflow-enabled hosts publish exact governed methods through ordinary capability discovery and
  invocation. Local attempts and direct/peer serving records retain one recoverable internal run,
  configured service authority, bounded continuation and cumulative internal allowances. Waiting
  publications release execution workers and transfer managed editing only through their exact
  accepted association. Invoke-only callers can read their declared public results without access
  to internal runs or administrative capabilities. Retirement preserves accepted calls; ancestry,
  caller revocation, cancellation and deadlines constrain future internal entry. See
  [published methods](../guides/published-methods.md).
  Public preparation resolves the selected revision's agreement and configured service grant facts
  into a reviewable publication document. Contracts, grant selection and limits remain explicit;
  publishing is separate. Text-brief preparation uses the existing authorized artifact upload.
- Selected knowledge and finite learning commands compose the existing artifact, context, proposal,
  runtime, account, verifier and publication owners. Declarations fix independent evaluation slots
  before model entry. Comparisons retain eligible, rejected and inconclusive outcomes; manual or
  separately preauthorized promotion requires current publication authority. The native Slotbook
  driver exercises held-out comparisons, independent loan/class products and staged tool recipes.
  Named CLI commands select artifacts by identity, submit actual candidates, compare, inspect and
  request promotion through the public learning operations. Inspection retains reasons and missing
  measurements. Source selection and current read authority survive exact receipt replay/restart.
  [Learning methods](../guides/learning-methods.md) explains frozen source versions and operator use.
- The CLI covers blueprint/sequence authoring, run/proposal/controller/peer/layout control,
  retained-work resolution, bounded inspection, verified create-new downloads, wait/follow deadlines,
  and stable machine output. [Production examples](../../examples/operator/README.md) provide
  fresh-directory setup and ordinary process/model workflows.
- Ordinary `workflow` commands create independent model workflows, edit prompts from files/stdin,
  connect named inputs and selected earlier results, choose an accepted final output, and save or
  reopen exact revisions through public daemon authoring. Draft files retain only base references
  and pending blueprint mutations, with guarded atomic replacement. Editing a saved version creates
  a child without changing old definitions or runs. The [operator recipe](../../examples/operator/README.md#author-a-model-workflow)
  explains the supported model editor and its explicit refusal of richer definitions.
  Saved-version discovery exposes declared inputs and outputs. Independent copies preserve the graph
  and immutable source provenance under a different workflow identity, without copying execution
  state or authority. Identity-bound governing agreements refuse independent copying; exact-version
  reuse remains available. Concurrent controlled runs retain separate briefs, outputs and pins when
  the copy is edited and saved.
- Ordinary run starts accept named immutable artifacts through existing upload and workspace
  admission. The CLI can upload a separate text brief for each run of one saved revision, retain a
  private exact request before submission, and reconnect under the original host/caller/grant.
  Selected brief and prior-step bytes reach model requests through frozen context manifests.
  Bounded optional waiting ends local observation without cancelling daemon work. Controlled
  endpoint and actual CLI/daemon tests cover isolated inputs and lost-reply recovery without
  repeating provider requests; integrated sprint acceptance remains open.
- The public run-result view shows a bounded current frontier, separate invocation/acceptance/
  workflow outcomes, authorized output previews and current permitted-action hints. The CLI
  retrieves final artifacts with range, size and digest verification and escapes terminal controls.
  Reconnecting observation deduplicates positions and refreshes an authorized current view after
  resync. At an editor workflow's final failed-model review hold, a paused run can prepare an
  ordinary approval-required repair proposal. Its new step receives the selected failed response
  and original run inputs, retains the failed check, and must pass a fresh completeness check.
  Controlled CLI/daemon tests cover restart, unchanged completed history, stale/unauthorized
  refusal and terminal-run refusal. Richer definitions and repeated automatic repair are outside
  this convenience; protected agreements retain their existing admission checks.
- Independent clients can author/save, upload an input, start, recover the exact request, download
  results and copy through ordinary public JSON. A focused actual-daemon test uses no CLI or private
  domain builders and retains the same two external fixture calls across restart/replay. Direct and
  published-call preparation now uses the serving owner for selection, profile, idempotency and
  deadline, with unchanged acceptance/replay owners. These focused checks do not establish browser
  authentication, CORS, packaging, real-model quality or full sprint acceptance.
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
| Blueprint revision and mutation | 3 | v1/v2 refused; agreement absence is explicit. |
| Context manifest | 2 | v1 refused; model envelope remains independent. |
| Model document / task / response / endpoint profile | 1 / 1 / 1 / 2 | Endpoint v1 refused; explicit billing and counting required. |
| Explicit continuation companion | 1 | Exact artifacts and journal anchors; unknown/future documents refused. |
| Proposal / workflow-control command / risk policy / controller policy | 1 / 1 / 1 / 2 | Controller policy v1 refused; currency is explicit or absent. |
| Prompt-sequence import | 3 | v1/v2 refused; existing blueprint history unchanged. |
| Result acceptance contract / decision | 1 / 1 | Explicit purpose; no implicit policy on generic model tasks. |
| Run command / run event | 1 / 5 | Supported event v1/v2/v3/v4 variants remain readable; obsolete nested snapshots are refused. |
| Authority grant / authorization decision | 4 / 2 | Earlier grants refused. |
| Authorized-command wrapper / command result | 1 / 2 | Result v1 reads only closed internal records. |
| Projection snapshot envelope / runtime payload | 2 / 6 | Old/invalid optional checkpoints replay from journal. |
| Administrative integrity cursor | 3 | Exact supported cursor. |
| Peer hot record / compact tombstone | 5 / 3 | Exact current caller/origin meaning; older records refuse. |
| Redb internal document format / physical schema | 20 / 16 | Older/future stores refused; no migration. |
| Application command receipt / layout record | 1 / 1 | Exact supported contracts. |
| Local-process profile / host materialization | 2 / 1 | Process v1 refused. |
| External control / authenticated cursor | 2.16 / 2 | Only the exact current protocol and cursor forms are accepted. |
| Peer protocol and catalog messages | 1.5 | Earlier minors refused. |
| Daemon configuration | 13 | TOML; JSON and earlier versions refused. |
| Managed resource request / inventory | 3 | Exact schema, bounded typed recipe references, preserved receipts and guarded transitions. |
| Published method / invocation association | 1 / 1 | Exact schema and generation; ancestry preserves accepted depth ceilings. |
| Learning declaration / comparison | 1 / 1 | Exact selected evidence, frozen evaluation slots and separate promotion authority. |
| Layout document / CLI JSON output | 1 / 2 | CLI schema 1 refused. |

## Limitations now

- Ordinary supplied inputs require artifact-reference v1 interface fields; files use the bounded
  upload route. Published methods retain their separate choice/artifact contracts. Public authoring
  returns validated revisions with server-derived identities. The model editor is restricted to
  fresh text steps, explicit direct inputs and completeness gates; richer definitions refuse
  editing. `construct_blueprint` accepts explicit existing mutations for advanced clients.
  Offline `blueprint create`/`govern` still support governed-method bootstrap before daemon setup.
- Execution-only startup omits runtime, control and workflow workers and can serve incoming peers.
  Authenticated clients can discover, prepare, submit, inspect, cancel and follow direct process
  and fresh-model invocations, and upload/download bounded artifacts through the public API/CLI.
  Direct selection freezes explicit inputs without workflow coordinates; direct continuation refuses.
  Local and serving work share prepared execution while retaining
  their distinct durable owners. Serving artifacts use host-invocation accounting; imports retain
  authenticated foreign provenance. Role removal refuses active workflow obligations and preserves
  closed history for offline inspection. Managed deployment and real two-machine qualification
  remain separate from the deterministic loopback binary evidence.
- Governed methods preserve immutable enclosing definitions while permitting bounded ordinary task
  edits under exact capability envelopes. Accepted agreement identity and cumulative adoption counts
  survive replay; child pins stay protected. The managed owner evaluates immutable candidate artifacts
  with an operator-pinned verifier and retains failed/unknown evidence. Publication binds exact bytes,
  configuration, policy, target generation and fresh authority. Raw start/update cannot substitute a
  candidate. The finite [Slotbook example](../../examples/adaptive-slotbook/README.md) is the supported
  authoring and inspection path. It is not a general application correctness or security guarantee.
  Catalog publication, service delegation and finite evaluated reuse are implemented.
  Public inputs are finite reviewed choices or authorized exact artifacts; results are declared
  accepted terminal fields. Per-call allowances and service grant/host ceilings apply, with no
  service-wide lifetime spending account. Process-internal network/model calls are not direct-model
  accounting. Native seeded protected deployment has run on the UM790, and completed calls replay
  after an orderly host reboot. An explicitly assisted source correction also passed the unchanged
  verifier and protected publication on the UM790; unaided local-model source development remains
  unqualified.
- Learning comparisons are bounded to declared slots and 4,096 events per internal run. Unread
  descendants, unavailable usage, missing public completion or missing verifier evidence prevent
  eligibility. The declaration fixes the input and output fields; the returned product must be the
  artifact accepted by the final verifier. The maintained
  application and repair remain seeded native fixtures. A valid real Ornith method proposal was
  evaluated on Drifty; its candidate runs lacked comparable verifier evidence, so the comparison
  remained inconclusive and the baseline was retained. Earlier malformed candidates and uncertain
  calls remain recorded. No useful real-model improvement or universal superiority is claimed. The
  [evidence guide](../development/verification-evidence.md#selected-learning-and-product-variants)
  records the distinct evidence lanes.
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
- No GUI is implemented. Actual graceful OS-signal evidence is qualified on the
  hosted Linux runner; forced Windows child termination does not qualify that platform claim.

## Current validation/evidence snapshot

The [strict static gate](../development/workflow.md#strict-static-gate) now enforces shared compiler
and Clippy policy, parsed repository boundaries, independent consumer probes, dependency hygiene,
workflow checks and bounded secret scanning. The complete selected static matrix passes on the
pinned Linux target, including default product binaries, all workspace targets/features and
warning-denying documentation. [05b's handoff](../development/virtual-office/client-ready-workflows/handoffs/05b.md)
records the exact checkpoint, checker negatives and focused regressions for cleanup uncertainty,
bounded access and output failures. Final combined runtime acceptance remains assigned to 06;
the static gate and earlier runtime evidence below do not establish that result.

### Active capacity and boundary validation

Publication turnover now passes six generations through a two-slot registry across complete store
reopen, preserving accepted calls and historical replay. Queued serving requests retain their exact
adapter before child creation; durable retirement refuses new acceptance from an old catalog.
Managed installation and evaluation turnover passes six cycles with one active slot, while uncertain
removal and incomplete evaluation retain their capacity. Successful verifier observations reclaim
scratch copies after confirmed container absence; retained failures have explicit byte/directory
admission bounds and an operator cleanup procedure. The
[evidence guide](../development/verification-evidence.md#active-capacity-and-publication-retirement)
identifies the tests and the physical qualification limits.

The linked mutation run's 32 survivors are resolved by 27 caught branches, four deleted duplicate
checks and one exact reviewed classification. Scoped campaigns have passing baselines and no
timeouts, including the additional disconnect-health assertion. These results qualify local
software behavior. Hosted and physical results remain scoped to their recorded executions.
Format-15 stores remain unsupported by physical format 16, as specified in
[ADR 0043](../decisions/0043-active-capacity-and-retained-history.md).

Software verification covers 988 unit/integration tests, including all 24 repository contracts,
and 24 doctests. Eight opt-in tests remain ignored. Formatting, all-target/all-feature checking,
warning-denying Clippy/rustdoc, dependency audits and test discovery pass. Default/all-feature API
inventories for the seven affected libraries retain test-only facilities behind their existing
features; the new capacity and recovery ports have named workspace consumers. Raw evidence remains
under `target/validation-repair/` and `target/public-api/`.

### Integrated acceptance

The fresh deterministic learning lane completed four held-out pairs with one baseline repair and
zero candidate repairs each, promoted generation 2, and produced separately verified camera-loan
and yoga-class variants. Only the loan product was deployed. A staged tool update preserved its
source application's bytes; knowledge supersession preserved prior selections. Cleanup removed
all 23 study installations and replayed with the reviewed identity/preservation checks. Native
inspection found all 47 retained study volumes and matching source/guidance hashes afterward.
The unrelated service kept its original container identity/start time, with no study containers left.

Fresh desktop actual-binary evidence covers independent roles, public files, direct and delegated
work, retained unknown usage, and the one-worker peer Slotbook method through repair, protected
deployment, daemon restart, exact replay, verification renewal and preservation-aware removal.
The model in these lanes is deterministic; both peers run on the desktop. Native BusyBox worker
enforcement and the protected verifier's timeout/reopen/candidate-integrity lane also pass afresh.
An authorized recovery now preserves a published wrapper's proof of no external entry. Its
regressions cover lost child stop evidence, complete store reopen, fencing, exact return/replay,
unrelated-authority refusal, cancellation and terminal hold release. Deliberate removal of the
proof guard makes the continuation test fail. Linux idle-role memory/thread observations and
reproduction commands belong to the [evidence guide](../development/verification-evidence.md).

The [integrated evidence](../development/verification-evidence.md#integrated-host-acceptance)
retains exact inputs, review scope and verification. Physical
desktop-to-UM790 direct process/model calls and workflows now pass using the same execution-only
serving capabilities. Both daemons restart with exact replay, unchanged outputs and no additional
process or native-model entry. The native Slotbook qualification also passes on the UM790.
Rootless prerequisites are installed under a dedicated account, and a setup reboot restored its
matching kernel/TUN module. A later authorized reboot automatically restarted the owned CPU model
service before Milkdrift; reopening the daemons replayed completed direct and workflow operations
with exact retained results and no new external entry. This tests idle completed-work recovery,
not interruption during an active call or power loss.

A real Ornith proposal was obtained from explicitly seeded source evidence. Its four candidate
evaluations did not produce comparable verifier results, so the outcome is inconclusive and the
candidate was not promoted. The retained baseline produced two independently verified variants;
only the selected loan variant was deployed. The owned CPU model passed direct inference, exact
reapply, automatic reboot recovery, drained configuration replacement and preservation-aware removal.
Old generation acceptances replayed, and the managed Rust source, binary and marker survived both
replacement and removal. Bounded real-source studies have not qualified a Slotbook application.
The maintained driver can alternate the desktop Ornith 9B and UM790 Ornith 35B endpoints, submit
their exact source through ordinary authorized proposals, compile it, and return selected compiler
evidence and the current source to the next call. Both servers expose 32,768-token contexts; the
current study allows 16,384 output tokens per call. Its eight responses produced seven failed builds
and one refusal because the rationale exceeded its declared byte bound. No candidate reached the
fixed verifier or publication. Earlier response-loss attempts and unknown usage remain uncertain;
they were not replayed. Study installations were removed with source, artifacts and data preserved.
The operator authorized an assisted correction to test the workflow independently of the local
models' ability to implement the whole application. That separately attributed source passed all
six unchanged checks on Drifty and reached protected publication. The served application remained
healthy after the workflow finished. Reopening the daemon and replaying its exact start command
preserved the complete terminal run, attempt identities, target version, accepted evaluation and
container ID/start time. Removal used the recorded preservation policy; all four selected volumes
remain. This is successful assisted workflow acceptance, with no claim that the local models
completed the implementation themselves. The original failed and uncertain records are unchanged.
Authenticated forwarding carries native model requests; the non-loopback plaintext restriction
remains in force. Native Vulkan use does not qualify the managed Vulkan boundary.

The closeout review's local gate passed 973 unit/integration tests, 24 doctests and all 24 repository
contracts, with warning-denying Clippy/rustdoc, dependency checks and test discovery. Eight opt-in
tests remain ignored. It used the default unoptimized debug profile and includes the CI fixture
corrections, assisted-input path and source replay checks. Cached model responses must match their
immutable artifact references; reconstructed authoring documents refuse changed retained proposals.
Source reads enforce byte bounds and regular files, and the writer refuses symlink replacements.
CLI reconstruction reproduced a retained model workflow's canonical document.

The native verifier lifecycle and actual-binary independent-host scenario retain their separately
reviewed evidence. Serving archival regressions reject output disclosure without authority and
retain authorized bytes across archival; restoring the prior behavior makes all four checks fail.
Default/all-feature serving and persistence API inventories are unchanged. These are local software
checks; retained hosted and physical results remain scoped to their recorded executions. The
[evidence guide](../development/verification-evidence.md#integrated-host-acceptance) distinguishes
those evidence classes and assisted source provenance.

### Evaluated method reuse

The 05 review passes the full gate with 943 unit/integration tests, 24 doctests and all 24 repository
contracts. Eight environment-specific tests remain ignored; the native verifier lifecycle test
passed separately. The changed default/all-feature APIs were reviewed. The same local optimization
1/debug-information 0 profile retains debug assertions. A real host/runtime regression reproduces
and fixes startup denial when a narrowly granted unnamed worker is temporarily unhealthy, while
proving execution still waits for health. Fresh actual-binary fixture qualification completed all
four held-out pairs, promoted generation 2, and produced two independently verified variants using
the candidate; only the loan product was selected for deployment. Staged tool activation preserved
application bytes and explicit knowledge updates preserved prior selections. The
[evidence guide](../development/verification-evidence.md#selected-learning-and-product-variants)
records the physical fixture evidence separately from the real-model and host results above.

### Protected adaptive method verification

The Rust implementation follow-up passes the full gate with 923 workspace tests, 24 doctests and all
24 repository contracts. Eight environment-specific tests remain ignored by that gate; the native
protected-verifier lifecycle test also passed separately. The changed managed-linux, evidence and
example default/all-feature API inventories were reviewed. Earlier deliberate removals of
protected-graph validation, failed-check enforcement and candidate-file validation made their refusal
tests fail. The build uses optimization level 1 with debug
assertions enabled; no unoptimized full-gate result is claimed.

The finite [Slotbook qualification](../../examples/adaptive-slotbook/README.md) uses immutable Rust
executable artifacts and the operator-owned native six-check verifier. Protected recipe schema 2
pins that verifier's executable digest and runs the candidate directly. Actual daemon/CLI execution retained
initial authentication/persistence failures, rejected an uploaded forged positive report, automatically
adopted a scoped prospective repair, completed all six new checks and served exactly the checked bytes.
Direct lifecycle updates cannot remove protection. Reopen preserved the agreement, adoption count,
private verification and target generation. A new authorized request renewed verification of the same
bytes without overwriting earlier evidence. A separately authorized direct publication reached
generation 3; exact replay after
another reopen preserved its generation, version, container ID and start time. Both test installations
were removed through their owner, retaining declared data volumes. The
[published-workflow evidence](../development/verification-evidence.md#published-workflow-capabilities)
records exact candidate identities and evidence paths. The native verifier test also covers detached-container cleanup after
completion, timeout and platform reopen, plus changed candidate-file refusal before activation.
Protected preparation shares the Podman version, controller delegation, subordinate-ID and lingering
checks used by other managed services. These fixtures do not represent observed model choices;
the separate live/assisted results are stated above. Interrupted verification retains unknown evidence and refuses
publication. This desktop qualification does not establish second-host operation,
power-loss persistence or general application correctness/security.

### Managed Linux resource qualification

Managed resources and their configuration corrections have desktop Arch Linux qualification with
Podman 6.1.2, systemd 261.3 and kernel 7.2.6. The adapter accepts 5.4..6.x and checks actual
controller delegation, private UID/GID mappings and effective definitions. Recipe schema 2 and
mechanism v3 separate worker/service budgets and make the owned model alias, endpoint bounds and
service deadlines configurable. Platform preparation no longer depends on Slotbook content or a
Git/Rust/C tool suite. The maintained image initializes Slotbook through an ordinary worker command.

Real BusyBox and owned CPU-model tests cover simultaneous independent kernel limits, denied
manager/rootfs access, retained files, reapply/reopen and preservation-aware removal. Systemd
restarts a deliberately killed owned service while every Milkdrift owner is absent. A failed
supervisor start preserves a foreign same-name container through exact container-ID cleanup.
Preparation accounts for the saved current service's resident memory and refuses byte limits that
would be rounded by the host. An unsuccessful authorized preview returns `unprepared` with useful
diagnostics and no effects.

Fresh daemon/CLI runs cover Slotbook initialization and Rust compilation, unrelated text analysis,
attached and differently named owned inference, busy-removal refusal, exact replay after restart,
retained edits, headroom diagnosis, incompatible apply refusal, explicit update and safe removal.
The owned model alias changes through update without Rust changes. A second model input and larger
operating choices have deterministic configuration coverage; a second GGUF was not physically
loaded in the corrective run.

The full local gate passes with 880 workspace tests, 24 doctests, all 24 repository contracts,
warning-denying Clippy/rustdoc, dependency audits, formatting, checks and discovery. Seven manual
tests remain ignored by that gate; both physical Linux cases were executed separately. Eight
default/all-feature API inventories were reviewed. That qualification used optimization level 1
with debug assertions enabled; the later integrated gate above uses the default unoptimized profile.

Actual model calls use explicit per-request `reasoning_effort: "none"`. Earlier desktop `ornith-9b`
and Drifty `ornith` daemon/CLI smoke evidence remains valid for those recorded inputs; Drifty uses
an authenticated SSH tunnel to its NetBird listener. Default `reasoning_content` streams still
refuse and retain uncertainty. Neither the corrected owned CPU run nor the earlier failed Gemma
smoke qualifies default reasoning-mode interoperability. Native server settings were unchanged.

The [managed evidence](../development/verification-evidence.md#managed-linux-installations) binds
images, model inputs, commands, full-gate/API results and profile settings. Corrective evidence lives under
`target/adaptive-hosts/post-02/`; predecessor results remain under
`target/adaptive-hosts/managed-02-qualification/` and `target/adaptive-hosts/managed-02-review/`.
Power loss, managed Vulkan and UM790 pressure remain in the
[hardware qualification issue](../development/virtual-office/whiteboard/issues/managed-linux-hardware-qualification.md).
The later orderly UM790 reboot establishes the narrow completed-work recovery described above;
the passing desktop and CPU tests do not establish the remaining hardware claims.
[Managed operations](../operations/managed-linux.md#platform-qualification) owns the runnable lanes.

### Independent host execution

The independent-host implementation is reviewed and accepted on Windows x86_64/MSVC with
Rust 1.95.0. The local gate after the process-shutdown correction passes: 833 workspace tests,
24 doctests, all 24 repository contracts,
formatting, all-target/all-feature checking, warning-denying Clippy/rustdoc, dependency audits and
test discovery. Five manual tests remain ignored. Commands and results are under
`target/ci-repair/`, with the independent-host binary evidence under `target/adaptive-hosts/review/`.
The [independent-host evidence](../development/verification-evidence.md#independent-host-execution)
records the corrected operator route and qualification limits.
Default/all-feature API inventories for fourteen affected libraries were reviewed against their
actual consumers; fixture entry, conformance and unjournaled clock helpers remain outside the
ordinary host API.

Hosted platform run 35317306020 passed Linux but failed macOS process shutdown and Windows fixture
startup. Shutdown now leaves signalling to the execution monitor; the cleanup fixture separates
bounded startup from its three-second shutdown assertion. The delayed-start regression reproduces
the old Windows failure and passes after correction. The later hosted
[platform run](https://github.com/hartolit/milkdrift/actions/runs/36319050580) at `c322cf4` passes
the configured cross-platform suites. This is selected platform coverage, not a complete
Windows/macOS workspace gate.

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
