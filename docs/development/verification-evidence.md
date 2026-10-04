# Verification and operational evidence

Use this guide to choose a reproducible check and understand what its observations support.
The ordinary [full gate](workflow.md#full-local-gate) checks executable changes, at the point
required by the [verification policy](workflow.md#choose-verification-for-the-change); the additional
lanes below exercise application use, mutation sensitivity, sustained load, and external resources.
The [evidence package guide](../../tools/evidence/README.md) compares the tools and their entry points.
[Status](../product/status.md#current-validationevidence-snapshot) owns the latest executed state.

## Independent workflow client

The daemon's maintained [JSON client test](../../apps/daemon/tests/control_plane/independent_client.rs)
launches the real binary and uses ordinary HTTP payloads to author/save a two-step model workflow,
upload its brief, start, retrieve the result, restart/replay and copy the definition. All semantic
identities and results come from public responses. It uses no CLI, private definition builder,
in-process runtime or database access. The external fixture counts model requests, and exact replay
after restart leaves that count at two. Changed requests conflict and missing inputs refuse.

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon -p milkdrift-cli --bin milkdrift \
  -p milkdrift-local-process --bin milkdrift-process-test-helper
cargo test -p milkdrift-daemon --test control_plane independent_client::
cargo test -p milkdrift-daemon --test control_plane direct_preparation::
cargo test -p milkdrift-daemon --test control_plane inputs_cli::
```

The preparation cases exercise the serving-owned request builder for idempotent, read-only and
non-idempotent operations, with host, capability, operation, input and limit refusals before
acceptance. The CLI cases separately interrupt an actual start reply and recover saved input
references. These are focused software checks, not the complete operator journey or browser
authentication/CORS, desktop packaging, real-model quality or physical durability qualification.

## Active capacity and publication retirement

Public reuse and request preparation have focused daemon/CLI checks alongside the core publication
and learning suites. Build the actual CLI before tests that launch it:

```sh
cargo build -p milkdrift-cli
cargo test -p milkdrift-daemon --test control_plane reuse::
cargo test -p milkdrift-daemon --test control_plane published_inputs::
cargo test -p milkdrift-daemon --test control_plane resources::configured_resource_owner_retains_exact_failed_platform_intent_across_restart
cargo test -p milkdrift-control --test learning --all-features
```

Reuse exercises concurrent distinct briefs, immutable accepted pins and independent copied edits.
The published-input fixture uses a controlled model with explicit billing/token bounds and a finite
account; it asserts uploaded text reaches the model and only the declared result is returned.
The Linux resource test retains a deliberately unavailable setup without starting a container, then
checks authorized learning-source selection, missing-page refusal, exact replay and restart.
Comparison tests distinguish eligibility, rejection and missing evidence; they do not establish
that a real model learned an improvement. Publication retirement tests below also exercise the
public preparation operation and refuse promotion without an eligible comparison.

The ordinary control and managed lifecycle suites exercise six completed cycles with deliberately
small operating limits: one installation, one pending evaluation, four active stored publications,
and two registered generations per capability. Full store reopen preserves historical receipts,
definitions and exact request replay. Incomplete evaluation intent and uncertain removal still
consume capacity; successful removal, completed verification and retired publication free their
respective slots. Missing count anchors or index rows refuse reopening, and integrity checks compare
the derived indexes with their authoritative records.

Publication tests invoke the production retirement maintenance operation. They retain a pending
child across retirement/reopen, then observe the adapter disappear after settlement. A separate
queued-serving case accepts a request before any child exists, retires its generation, refuses a
new request using the old catalog, and restores the queued request's exact adapter on reopen.
Terminal evidence then permits reclamation. The daemon's public invocation test also covers an
immediate promotion/retirement while the accepted call proceeds to a successful result.

```sh
cargo test -p milkdrift-control --test control_service --all-features published::
cargo test -p milkdrift-managed-linux --test lifecycle --all-features
cargo test -p milkdrift-redb-store --test contracts --all-features capacity::
cargo test -p milkdrift-daemon --test control_plane --all-features published::
```

Linux scratch unit tests exercise completed-copy turnover, exact byte/directory boundaries and
symlink refusal. They establish filesystem accounting and cleanup behavior; the physical verifier
fixture remains an opt-in test. See the [managed guide](../operations/managed-linux.md) for retained
scratch cleanup and [ADR 0043](../decisions/0043-active-capacity-and-retained-history.md) for the
format-16 storage boundary. These software regressions do not renew prior hardware qualification.

Integrated verification at `b902a28` uses Rust 1.95.0, the ordinary debug profile and loopback
listener permission. The final workspace run passes 964 unit/integration tests outside the repository
contract target and 24 doctests, leaving eight opt-in tests ignored. Its status-wording contract
failure was corrected in prose; all 24 repository contracts then passed separately. Build,
format/check, warning-denying Clippy/rustdoc, deny/machete/duplicate audits and test discovery pass.
`target/validation-repair/gate-test-final.log` preserves the full run, and
`gate-contracts-final.log` preserves the corrected contract result. Default/all-feature API
inventories for persistence, capability-host, control, redb-store, managed-linux, peer-http and daemon
are under `target/public-api/`. No test-owned daemon or process helper remained after completion.

## Independent host execution

Build the daemon, CLI and `headless-cli-evidence`, then run:

```sh
cargo run -p milkdrift-evidence --bin headless-cli-evidence -- \
  --independent-host-only --daemon PATH_TO_DAEMON --cli PATH_TO_CLI
```

This uses actual daemon/CLI binaries, the production process adapter and a deterministic external
model HTTP endpoint. Public uploads and `invocation prepare/submit` produce useful outputs on an
execution-only host. Hot replay and complete teardown/restart preserve one entry, direct producers
and absence of workflow records. A second daemon runs workflows against the same serving operations;
the fixture asserts staged artifact/manifest inputs, real origin coordinates, imported useful outputs,
originating controller reservation/settlement and no serving-side synthetic runs. A third model
request reaches the external endpoint, which drops the reply. The origin retains an uncertain
attempt and the exact outstanding token/artifact allowance across restart; the external counter
remains three model requests in total (one direct, two remote). An uncertain outcome is not terminal
settlement and cannot release its reservation. The process marker records two useful entries, one
direct and one remote. It uses an explicit loopback development exception and does not
establish non-loopback TLS deployment, a physical second machine, or real model quality.

To exercise hosts already installed by an operator, use `--installed-hosts` with a private JSON
manifest and a fresh `--installed-output` directory. The driver uses the CLI only; it does not
launch, reconfigure or stop these hosts. Each endpoint must be loopback HTTP, including the local
end of an authenticated SSH forward for a remote machine. First connect the peer with
`milkdrift peer connect HOST` and read `milkdrift capability list` on the coordinator. Copy the
exact coordinator capability IDs into the manifest; the driver also checks their peer identity
before dispatch. Example shape for the exercised topology:

```json
{
  "schema_version": 1,
  "namespace": "acceptance-unique-1",
  "serving": {
    "endpoint": "http://127.0.0.1:19790/",
    "token_file": "/private/serving-operator.token",
    "host": "um790-execution"
  },
  "coordinator": {
    "endpoint": "http://127.0.0.1:19791/",
    "token_file": "/private/coordinator-operator.token",
    "host": "desktop-coordinator"
  },
  "process_capability": "independent-process",
  "model_capability": "operator-model",
  "coordinator_process_capability": "peer:06bd6f8e9186:independent-process",
  "coordinator_model_capability": "peer:06bd6f8e9186:operator-model"
}
```

The serving process must accept the `source` artifact and return its exact bytes as `stdout`.
The byte-pinned evidence executable's `--fixture-independent MARKER_FILE` mode supplies this
process and appends an independent entry marker. The model must accept fresh text requests with
streaming, a 64-unit output maximum and the explicit `reasoning_effort: none` extension. Its profile
needs truthful finite admission terms and enough artifact allowance for inputs plus the prepared
response bound. Use the normal [peer](../operations/peers.md) and
[provider](../guides/local-model-endpoint.md) setup paths for capabilities and permissions.

```sh
cargo run -p milkdrift-evidence --bin headless-cli-evidence -- \
  --daemon PATH_TO_DAEMON --cli PATH_TO_CLI \
  --installed-hosts /private/hosts.json --installed-output /private/evidence-new
```

The driver retains exact requests, paged observations and downloaded bytes, requires successful
terminal outcomes, and compares direct versus workflow origins. Model text must be nonempty UTF-8;
this check establishes useful transport, not application quality. A closed uncertain invocation
stops qualification with its evidence preserved. Observation history is bounded at 4096 events.
After restarting the same daemons with their preserved stores, run the same command with
`--installed-replay`. It checks the original execution/attempt identities and bytes, including
archived direct outputs. Independently record physical host identities, process marker deltas and
model server entries before/after replay; the driver cannot infer those from HTTP success.

The harness prints idle process measurements before either role begins its workflow scenario.
On Windows x64 build 26200 with debug binaries, one 2026-09-18 observation measured execution-only
at 31 threads, 30,457,856 working-set bytes and 6,369,280 private bytes; workflow-enabled measured
35 threads, 31,608,832 working-set bytes and 6,275,072 private bytes. Both had the configured serving
workers and ran on the same machine. These are process snapshots with OS/runtime overhead,
not a portable performance promise or a live-inference memory measurement.

On Linux the harness reads the daemon's `/proc/PID/status` and records resident memory (`VmRSS`),
resident high-water mark (`VmHWM`), virtual size (`VmSize`), thread count, architecture and kernel.
An Arch x86_64/kernel 7.2.6 observation with Rust 1.95.0, development optimization 1 and debug
information 0 measured 40 threads and 35,840 kB resident for execution-only, and 44 threads and
35,408 kB resident for workflow-enabled. Debug assertions remained enabled. Virtual sizes were
2,157,392 kB and 2,165,384 kB respectively; virtual address space is not physical memory consumption.
These idle same-machine snapshots precede requests and include runtime/OS overhead. They do not
measure private allocation, model memory or a throughput guarantee. Raw observations are in
`target/adaptive-hosts/06/independent-measured.log`.

Focused serving tests cover prepared refusal/panic/revocation/expiry, caller separation, archival,
request-bound input staging and cumulative quota replay. Redb artifact tests reopen after interrupted
client uploads, reclaim only that owner through bounded pages, preserve workflow/peer publication
ownership, and verify imported controller charges across replay/reopen. Daemon role tests assert
closed-history preservation and active-obligation refusal. The full gate remains necessary for
changes to these shared boundaries.

Peer adapter regressions also cover continuous output and delayed artifact metadata/cleanup while
the origin retains its lease. Renewal occurs between bounded requests and refusal stops further
work. A request or transport outage that itself exceeds the lease still requires uncertainty
handling; the adapter does not extend an expired lease or replay an external effect to hide it.

The prepared-allowance negative tests were also run with their admission guard deliberately
disabled. Both detected one forbidden external entry instead of zero; both passed after restoring
the guard. The targeted commands and failure assertions are retained under
`target/adaptive-hosts/prepared-allowance-mutant.log` and `prepared-allowance-restored.log`.

## Governed Slotbook method and publication

Run the [maintained CLI qualification](../../examples/adaptive-slotbook/README.md) on a provisioned
rootless Linux host. It freezes the HTTP/API representation, uses explicitly seeded fixture failures,
submits a structured prospective repair, and obtains fresh trusted observations before deploying
immutable executable bytes. Its files preserve the original failure, proposal identity, accepted agreement,
new candidate and target generation. A fresh direct evaluation/publication and request replay after
reopen separately exercise the managed owner outside a workflow. Container identity/start time and
mounted bytes support the replay and exact-candidate observations. This does not qualify arbitrary
application behavior, model reasoning, power loss, hostile host administration or another machine.

For actual generated source, use the separate [`slotbook-evidence develop` route](../../examples/adaptive-slotbook/README.md#develop-source-with-a-real-model).
It selects the fixed public source brief, obtains at most three actual structured proposals,
submits them through a scoped advisor credential and compiles returned Rust in an isolated worker.
Completed compiler/verifier evidence feeds bounded repair requests. The immutable candidate must
pass the same six checks before protected publication. A passing initial source candidate requires
a separate labelled seeded-repair run; seeded repair cannot establish a model's prior planning
failure. Explicit resume inputs and ownership-checked preserving removal retain the full study.
The separately declared learning comparison keeps its original held-out inputs and criterion.

`cargo test -p milkdrift-blueprint --test agreements` covers indirect dependency/interface/terminal
and scope edits. `cargo test -p milkdrift-control --test control_service agreements::` covers ordinary
automatic proposals, raw adoption refusal, inherited child pins and cumulative limits after reopen.
`cargo test -p milkdrift-managed-linux --test lifecycle protected::` supplies real redb transactions
and deterministic external-effect counters. It checks private failed/unknown evidence, artifact read
authority, concurrent revocation during preparation, policy changes at final entry, expiration,
wrong generation, exact bytes, lost effect responses and before/after evaluation-commit failures.
Those deterministic tests establish owner behavior without making an OS-isolation claim.

The opt-in `protected_verification` target exercises actual detached test containers and filesystem
state. Set `MILKDRIFT_PROTECTED_TEST_IMAGE` to an approved preloaded image providing `/bin/sleep`, and
optionally `MILKDRIFT_LINUX_EVIDENCE_PARENT` to the directory for retained evidence, then run:

```sh
cargo test -p milkdrift-managed-linux --test protected_verification -- --ignored --nocapture
```

It checks fresh evaluations of unchanged bytes, cleanup after a verifier that omits its own cleanup,
cleanup after timeout, fencing a retained container on platform reopen, and candidate-file drift
refusal before configuration or startup. The CLI qualification separately proves that new evaluation
requests retain the prior evidence and can publish, including with a registry-qualified image digest.

Three deliberate guard removals in a disposable worktree made the graph-protection, failed-evidence
publication and candidate-file entry tests fail at their refusal assertions. Original code remained
unchanged. Commands, mutated diffs and failures are retained under
`target/adaptive-hosts/03-review/mutation-*`; successful owner suites are in the same directory's
final gate. The handoff records the exact execution profile
and physical observations so later publication/reuse work can preserve their finite provenance.

## Local process reporting cleanup

Run `cargo test -p milkdrift-local-process --all-features` for adapter conformance and lifecycle
regressions. The [reporting cases](../../adapters/local-process/tests/process_execution/reporting_cleanup.rs)
reject initial, stdout/stderr progress, and heartbeat reports while a real helper keeps its pipes
open. They observe child exit, propagated post-entry errors, and absence of a fabricated terminal
report. Deadline failures invoke fixture cleanup. Reporter panic, cancellation, timeout, output-limit
termination, and shutdown use the same child observation.

The [private ownership tests](../../adapters/local-process/src/process/lifecycle/tests.rs) hold stdin,
stdout, and stderr workers at completion while separately waiting for child absence. A worker
reaching its gate does not prove the cleanup thread has already reaped the child. The tests
establish that cleanup joins each started worker
before removing registration, including partial startup and unwinding, and that a duplicate
registration leaves the original cancellation control intact. Windows execution qualifies the
immediate child; the Unix-only reporting cases additionally check owned descendants when run there.
See [status](../product/status.md) for executed evidence and platform limits.

The inherited-pipe fixture creates a holder outside the owned group and synchronizes readiness
through its PID record. Tests own both explicit release and fallback termination. They require
invocation completion while the holder is still alive, so child termination alone cannot satisfy
the regression. Normal-output tests compare every final byte. Startup-failure injection and a
stopped Unix child exercise partial ownership and escalation when TERM cannot complete cleanup.
The shutdown regression pauses the monitor at its initial report, requests cancellation or shutdown,
then requires a fresh child-written marker before releasing the monitor. This detects competing
signal delivery even when an exited child remains visible as a zombie. After release, it requires
child absence, cancelled terminal evidence and no retained adapter load.

`cargo test -p milkdrift-daemon --test control_plane --all-features process_cleanup::` drives the
same byte-pinned helper through drain/cancel/retain shutdown, reopens the store, checks the durable
uncertainty explanation and refuses unsafe retry without another entry. Build the helper first.
This fixture deliberately delays holder startup beyond 800 ms. Readiness has a separate bounded
allowance; drain/retain request parent exit explicitly after shutdown is requested, while cancel
must terminate the parent itself. The daemon has ten seconds for durable shutdown, with a separate
fifteen-second harness deadline. Cancel must return the daemon's retained-uncertainty error; a harness
timeout cannot satisfy the test. Live external-holder checks remain independent of startup speed.
The platform workflow also runs these daemon cases and the capability-host lifecycle suite.
The per-platform uploaded logs, not cross-compilation, establish which OS paths were executed.

## Constrained peer placement

Constrained peer placement is exercised by
`cargo test -p milkdrift-daemon --test two_daemon_peer --all-features` using the maintained
`examples/operator/peer-placement.json`. Three production daemon host instances communicate through
real HTTP listeners on local TCP.
An origin with a local look-alike connects to two serving peers, each with its own temporary store,
credential and repository directory. Exact peer requirements admit a narrowed
grant, enter each approved process once, return verified core artifacts, and retain the same
requirement/peer/catalog provenance through reconnect and reopening all stores. The fixture uses
the production daemon host and configuration reader plus the built process helper. This is a
repeatable test topology for the implemented execution path. It does not establish physical
multi-machine deployment qualification.

The fixture gives each delegated call thirty seconds and observes completion for forty seconds
per call, including eighty seconds for the two sequential placement calls. A ninety-second catalog
covers setup and both calls. Turnover waits for
a fresh catalog when less than one full call allowance remains. These test budgets permit the
configured work to finish on unoptimized runners; they are not latency guarantees.
The local published-method fixture likewise observes its thirty-second method allowance plus ten
seconds for terminal reporting. It polls for actual archival after the retention horizon and accepts
an exact replay whether its record is still hot or already archived. A 25% CPU-quota run reproduces
the previous ten-second observer failure and passes with these observation changes; the method's
execution deadline and required successful terminal stay unchanged.

Control-plane fixtures also distinguish execution budgets from consumer observation. The configured
process keeps its ten-second execution limit and is observed for twenty seconds. Positive stream
observations allow thirty seconds, including draining to the journal head; the 600 ms negative
idle check and fifteen-second shutdown bound remain unchanged. Stage-labelled errors identify which
observation failed. Local pressure runs reproduce both earlier observation failures and pass with
these corrections: the operations group at 25% CPU quota and the stream case at 10%.

The headless remediation fixture assigns its four process profiles thirty seconds each, including
synchronous progress reporting and pipe completion. Its observers cover five sequential calls before
approval and three after remediation, with ten seconds of orchestration/reporting allowance per
call. Each run read is bounded by that phase's monotonic deadline. Unexpected terminal failures still
fail immediately with attempt evidence. Adapter timeout and cleanup tests keep their independent
limits; these fixture allowances establish no product latency promise.

Capability/authority/blueprint/sequence tests cover strict placement sets, immutable revision identity
and obsolete snapshot refusal. Runtime and capability-host suites exercise final-entry revocation,
deterministic
selection, local look-alikes, conflicting exact requirements, empty sets, stale health, removed
generations and catalog-update races with entry counters. Peer protocol/service tests retain exact
acceptance/replay/conflict, observation paging and expiry behavior. Artifact tests check the entered
execution's shared input/output allowance, exact publication replay at its bound, empty/corrupt
outputs, metadata ownership, and revocation before further bytes. Remote-adapter tests advance a
controlled clock across multiple HTTP chunks to prove lease renewal through publication. Refused
renewal and shutdown stop further downloads and release incomplete core staging. Run these through
the ordinary full gate; the operator/model/controller binary lanes below check the affected common
composition.

## Historical query cost

The integrated review measures the public daemon/client paths with an ignored reproduction under
`target/acceptance-08/query/`. It creates the ordinary process → signal wait → process workflow,
then appends 128, 1024 or 4096 settled terminal occurrences using atomic journal contracts. The
semantic frontier remains fixed: one live wait before release, four current nodes and no live
obligations after completion. Seeded checkpoint envelopes are verified by the production reader;
the daemon runs the actual processes and selects the early output as the late task's context.

The Windows build 26200 x86_64/MSVC Rust 1.95.0 debug comparison runs on an i9-13905H (20 logical
processors) with about 32 GiB RAM and uses source base `f80cce8` plus the acceptance diff.
Both binaries read the same reopened completed stores; every public attempt field must equal
the baseline result. Temporary counters were removed after building private measurement binaries.
They count decoded journal rows/pages and returned stored-event payload bytes, not physical disk
I/O, total allocation, TLS cost or operating-system cache misses. Three attempt samples use median
elapsed time; broad browsing and node/denial reads are individual observations.

| Settled occurrences | Attempt rows / pages / payload bytes before, per read | Near-start / near-end before | Near-start / near-end after | Execution entries retained before → after |
| --- | --- | --- | --- | --- |
| 128 | 427 / 2 / 161804 | 61 / 60 ms | 34 / 35 ms | 131 → 1 |
| 1024 | 3116 / 13 / 1013719 | 344 / 344 ms | 35 / 36 ms | 1027 → 1 |
| 4096 | 12331 / 49 / 3946426 | 1324 / 1327 ms | 32 / 35 ms | 4099 → 1 |

After correction each retained attempt reads one page of 15–17 events, about 17.6–18.8 KB. The
existing occurrence's creation/terminal anchors replace the whole-history reconstruction, and the
current-attempt path no longer scans again for peer identity already frozen in its snapshot. A
retired occurrence without an anchor uses two page-bounded passes and retains one owning execution.
No new index or cache is justified by this workload. Original revision, retry timing, exact prefix,
missing/discontinuous page refusal, whole-result equality and restart checks protect the change.
Historical reconstruction also recognizes recovery and reconciliation remediation creation. The
regression checks both full-history and anchored reads after later revision pins; reconciliation
work keeps its authorizing plan's target revision rather than the prior or latest run revision.
Runtime's existing rejected/missing optional-checkpoint tests remain applicable without modification.

Broad timeline browsing still returns 427 / 3116 / 12331 rows in 2 / 13 / 49 pages, with a peak page
of 256 events and 0.16 / 1.01 / 3.95 MB of event payload. Its post-change elapsed times are 92 / 619 /
2393 ms. Known execution reads require no event rows on these checkpointed stores (22–74 ms in the
post-change observations). A credential scoped to another run is refused before journal decoding:
zero rows and payload bytes at every size (9–27 ms). Existing context tests additionally verify
protected-artifact omission/redaction and post-claim permission refusal.

Causal discovery before late-task entry takes 49 / 59 / 60 ms. It reads four exact anchors and up
to two bounded tail pages: 415 / 516 / 516 rows and 145176 / 164185 / 165133 payload bytes. Its tail
limit is 512 records; maximum retained execution entries are 131 / 171 / 171, with one candidate,
at most one attempt, one revision-distance entry and one indexed source sequence. It selects one
authorized early artifact in all three histories. This is bounded by the configured discovery
budget, not constant memory for every possible task policy.

The separate missing-checkpoint probe shows why these figures are not universal latency promises:
at 1024 occurrences a near-start read takes about 2.2 seconds and a full browse about 24.8 seconds;
at 4096, a subsequent CLI command exceeds its harness deadline while replaying the uncapped tail.
This seeded probe deliberately omits normal checkpoint persistence. Keep snapshots available and
distinguish projection rebuild from exact historical lookup. Reconsider a rebuildable lookup index
only if measured retired-occurrence access remains an operator problem after using existing anchors
and cursors. Journal authority, exact replay and authority filtering must remain unchanged.

`query-summary.json`, `query-snapshot-before/`, `query-snapshot-after/`, the earlier cold probe,
instrumentation scripts and build logs retain the reproduction and raw samples under
`target/acceptance-08/`. These are machine-specific observations, not throughput guarantees.

## Controller activation acceptance

Explicit `enabled` activation is accepted by the integrated review. Default startup remains
disabled. The finite prerequisite list is closed by these owners and scoped observations:

| Prerequisite | Evidence and acceptance meaning |
| --- | --- |
| Concurrent final entry | Persistence/redb account contracts and control admission tests require exact revisions and reserve the final allowance atomically. The actual-binary race retains two process reservations and refuses another entry. |
| Crash/reopen and unknown usage | Runtime/host/model effect-stage and account suites retain uncertain effects and reservations through preparation, entry and reporting boundaries. The controller process-kill case reopens after lease expiry and again without resetting its account. |
| Artifact accounting and compaction | Atomic publication/charge, replay/abort/restart and exact-bound artifact tests pass; the binary lane archives receipts, retains linked acceptance/proposal artifacts and verifies unchanged public accounting after cold replay. |
| Approval and reconciliation | The installed binary loop refuses revoked approval, reopens at the approval hold, adopts an ordinary prospective revision and preserves failed history and exact command replay. |
| Mutation sensitivity | Retained accounting/model faults from `review-03a` still apply to unchanged owners. The acceptance diff additionally targets historical owner selection and omitted installation for explicit enablement. Raw fault outcomes remain under `target/acceptance-08/final/`. |
| Longevity | Release `revision_and_lifecycle::release_controller_longevity_stops_once_across_checkpoints_and_restart` and `admission::release_controller_admission_longevity_turns_over_reservations_artifacts_and_restart` exercise the unchanged lifecycle/account owners. Retained executed results are `target/review-03a-extra-results.json`. |
| Operational and full gate | Current Windows full gate and default-build operator/model/controller binaries cover the combined diff. Retained hosted benchmark/stress scope is recorded in status; it does not become new platform evidence. |
| Real external loop | `target/review-03a-real/report.json` records the authorized Windows Codex/Bonsai loop, separate approval, accepted repair, two settled direct-model calls and pre-transmission refusal of a third. Account/lifecycle/metering owners are unchanged; current fixtures cover later operational integrations. No additional cloud call is required. |

This decision changes source support for explicit configuration. It neither deploys an installation
nor qualifies arbitrary providers, managed sessions, thinking controls, physical peers or power loss.
The [status owner](../product/status.md#current-validationevidence-snapshot) records the accepted
configuration, source identities, current results and supported limits.

## Actual-binary scenarios

The deterministic `local-model-evidence` lane also exercises explicit model continuation. It
inspects a canonical predecessor, submits and adopts an ordinary prospective proposal, restarts
before release, and captures the continued request beside a Fresh request. Inspection exposes the
exact companion and remains unchanged after a second restart. A separate case gives a complete text
response to a tool-output acceptance contract: continuation must refuse that rejected answer before
scheduling or contacting its endpoint. Both cases use the actual daemon and CLI, not a provider
session service. Run the lane with the `evidence-process-helper` binary, as specified below and in CI.

`cargo test -p milkdrift-model-provider --test mock_endpoints --all-features` exercises both wire
mappings through runtime and the capability host. Continuation cases cover same-run source linkage,
sibling scope and other-actor references, missing/corrupt/unsupported evidence, retained-policy
refusal, bounds, current grant revocation, three-invocation chains, complete/incomplete tool pairs,
excluded traces in saved request messages, source corruption or read revocation after claim, later artifact publication,
and store reopen after selection. Model contract tests pin the companion/request fixtures and reject
cycles, depth overflow, unknown fields and instruction-role injection. Parser tests reject unsupported
response roles and content; sequence tests reject non-Fresh process imports. These mock lanes make no
real provider-session interoperability claim. The stream-cancellation unit regression forces a
cancellation during the body read before EOF, a transport error or malformed content returns;
the HTTP fixture holds its connection until cancellation has been acknowledged. Both preserve
uncertainty without claiming remote termination.

Offline storage composition is exercised by
`cargo test -p milkdrift-daemon --test storage_admin --all-features` and the structured-runtime
`context_enforcement::offline_binary_inspects_blocked_legacy_context_without_disclosing_or_rewriting_it`
test. Build the daemon first as described in the [workflow](workflow.md). These tests use temporary
current-format stores and private scratch: writer refusal, create-new backup/restore, source-byte
identity, clone execution refusal, and normal startup refusal beside redacted unsafe-context
inspection. Redb offline/artifact/account tests and peer retention tests establish exact cold
receipt and tombstone replay/conflict, unfinished publication offsets and outstanding-account
preservation. They do not establish filesystem power-loss or hostile OS-actor protection.

The structured-runtime `context_enforcement::recovery_binary` scenario starts that actual daemon
with `--recovery` over the same legacy fixture and configured retained grant. It submits, approves
and applies a prospective repair through the real control client, checks withheld context and
execution readiness, and reopens the daemon to prove exact receipt replay/conflict. Normal runtime
startup then completes the repaired run with one fresh invocation and unchanged prior evidence.
`context_enforcement::recovery_controls` covers both legacy failure classes and permanent runtime
effect/scheduler refusal. Control-service replay tests retain the recorded policy across a switch
to normal startup; daemon host tests verify shutdown without workers. These prove supported
safe-restart behavior, not general corruption repair or external-effect settlement.

The integrated review retains gate and deterministic binary-scenario logs under `target/review-main`
and default/all-feature API inventories under `target/public-api/review-main`. Its real-endpoint
model smokes use `target/review-main/profiles/bonsai-1.json` and `bonsai-2.json`; each saved session
and failure log records the 150-second harness deadline without terminal evidence. These failures
do not qualify the live endpoint or prove that generation stopped when the harness exited.

Build and run the headless operator scenario:

```sh
cargo build -p milkdrift-daemon --bin milkdrift-daemon \
  -p milkdrift-cli --bin milkdrift \
  -p milkdrift-evidence --bin headless-cli-evidence
target/debug/headless-cli-evidence \
  --daemon target/debug/milkdrift-daemon \
  --cli target/debug/milkdrift --examples examples/operator
```

On Windows append `.exe` to executable paths. The scenario uses maintained operator files,
temporary redb/artifact roots, private bearer files, ephemeral loopback HTTP, deterministic
byte-pinned processes, and a controlled model endpoint. It checks starter setup, import,
replay/conflict, inspection, pause/signal/resume, guarded proposal adoption, artifact download,
abrupt-restart uncertainty resolution, durable reads, stable failure exits, and model provenance.
Every CLI action is a real process; no CLI receives the database path.

The operator model check allows thirty seconds for CLI negotiation, model output publication and
the separate result-acceptance task. The earlier five-second observer expired in hosted run
36692081893 even though its timeline subsequently recorded both tasks succeeding, with completion
about 6.5 seconds after run creation. A controlled actual-binary regression holds the model response
for eleven seconds before releasing it, challenging both that observer and the former ten-second
subprocess watchdog. This is a software completion check, not a model latency promise.

The shared application harness gives ordinary CLI commands eight seconds of work and two seconds
to exit. Explicit `--timeout-secs` replaces the work allowance; the watchdog still leaves two seconds
for the CLI's own bounded result. Nested readiness, run and controller-child probes share their
enclosing monotonic deadline, including the last partial probe. Killing a child does not cancel
submitted workflow work. Failure collection has a separate six-second allowance for selected public
run, timeline, health and optional capability reads. It retains the original error if these reads
fail, bounds output, and omits credentials, input/context contents and arbitrary adapter messages.
Owned children are killed and reaped on early exits, with a separate bounded cleanup attempt.

Independent hosting uses the daemon's ordinary thirty-second lease. The five-second fixture lease
belongs to the operator crash/recovery case and is no longer copied into the peer-transfer scenario.
Run 36362687103 retained an uncertain remote model attempt after that short lease expired during
output reporting; extra observer time cannot resolve that uncertainty. Production lease enforcement,
accepted-work protection and the explicit lost-response assertions remain unchanged.

After the application build succeeds, quality CI runs all three scenarios even if an earlier one
fails. Cancellation or a failed/skipped build prevents new scenario entry. Each scenario retains its
own failing exit, so a later success cannot make the gate green.

The installed controller scenario uses the same daemon composition with explicit `enabled`
activation. Build the daemon with ordinary default features or `--all-features`, then
run `headless-cli-evidence` with the same `--daemon` and `--cli` paths plus
`--controller-qualification`. Optional `--controller-output NEW_DIRECTORY` retains the private
fixture configuration, pinned process profiles, exact proposals, accounting reads, and reports.
CI runs this deterministic scenario after building the real applications.

It composes ordinary process, verification, model review, acceptance, proposal, approval, reconciliation, repeat,
and signal nodes. Failed work remains rejected; an ordinary workflow-control invocation submits
the artifact-linked repair, a distinct authorized actor approves it, the child adopts a new
revision prospectively, and independent verification accepts the repair. Two model calls complete;
the next model task is refused by their cumulative allowance. Separate concurrent workers compete for two process entries; both active
reservations and the denied third attempt are inspected. Process/model entry counts are charged
at admission, while unit/cost/artifact remainders stay reserved until their owning evidence settles.

The scenario checks disabled start, explicit enablement, disabled recovery of an active
account, revoked approval, exact command replay after receipt archival, retained proposal/acceptance
artifacts after compaction, settled restart, and crash/reopen of an entered process with a retained
artifact obligation. These are process-kill boundaries, not power-loss or escaped-descendant proof.
Revoked approval is exercised by disabling the approver and restarting. The serving owner records
the effective revocation in its immutable admission generation; re-enablement uses a fresh grant
revision and revocation generation, as described in [changing authority](../operations/authority.md#changing-authority).
Both disabled and enabled modes refuse an unmarked subworkflow placed before the marked
repeat: its parent remains created and no child is admitted outside the cumulative account.
The process fixture emits bounded entry progress, lives at most ten seconds, and has a thirty-second
adapter deadline; concurrent verification uses a sixty-second execution lease.
The separate crash case uses a thirty-second lease and reopens after expiry; an unexpired lease
does not yet establish the recovery classification. Its non-idempotent process effect remains
uncertain with the same reservation through another reopen.

Its counting model fixture proves zero endpoint calls on unknown hard input-unit bounds. Up to
three explicitly supplied `--controller-model-profile PATH` options exercise the same negative
path against operator-selected profiles. Registration and local preparation must succeed before
the exact admission refusal is accepted as evidence. A refusal is not a qualifying model/controller
loop. Requested output units, endpoint/profile provenance, unknown effective server settings, and
the unchanged model-admission count are retained separately. Activation is an explicit operator
configuration described in [daemon operation](../operations/daemon.md#controller-activation).

The same scenario's `--controller-review-profile PATH` selects an explicitly approved loopback
model for two connected review stages. Real mode also requires `--controller-server-facts PATH`,
with bounded inspected settings and operator declarations matching that model and endpoint.
`--controller-agent-profile PATH` supplies a byte-pinned
local Codex/LM Studio launcher and creates a fresh disposable repository. The first agent writes
the deliberately incorrect 41; a separate JSON checker requires 42. A stopped or malformed model
decision prevents proposal submission. A supported decision is evidence for the ordinary proposal,
separate approval and future-only repair; the second checker and acceptance must pass. Restart
holds and final accounting prove that completed calls are not repeated. The two-call model
allowance then refuses a third task before transmission, retaining successful repair evidence;
the workflow stops failed on that deliberate refusal. Omitting the real-resource options runs
the same deterministic fixture lane. Process-ceiling evidence runs separately.

The corrective contract reserves the complete prepared body under the supported byte-BPE/template
contract. Unbilled final tokens settle without an invented monetary report. Billed fixtures cover
exact currency, cached/input/output rates and conservative rounding; unknown charge remains a
different refusal. Missing/contradictory final usage, over-envelope tokens, reporting failure,
revoked preparation and lost-stream restart are covered by adapter/account/host contracts. The
[local guide](../guides/local-model-endpoint.md#controlled-local-text-requests) owns reproduction,
server-contract obligations and the distinction between direct model calls and agent launches.

Current review evidence is retained under `target/review-03a-*`: gate/focused logs,
installed reports, selected profiles and server facts, source patches, and binary identities.
The successful real loop is `target/review-03a-real/report.json`: two direct model reviews
settle 6661 input and 102 output tokens, four process entries and 45487 logical artifact bytes.
Provider spend is zero, currency is absent, reservations are empty, and the repair is accepted.
An additional model task is rejected with `Limit { dimension: "model_admissions" }`, without
uncertainty or another admission. The workflow stops failed on that deliberate refusal; its
completed controller status has no `reached_bound` value. Recorded restart, replay, compaction,
concurrent entry and entered-process crash assertions pass. This single bounded run takes 477
seconds; the repair attempt succeeds, with no retry or allowance increase. The original corrective
reports remain under `target/corrective-03a-*` as evidence for their own source and binaries.

The reviewed deterministic controller report is `target/review-03a-controller/report.json`; the
ordinary operator log is `target/review-03a-operator.log` and the ordinary deterministic model
report is `target/review-03a-local-deterministic/report.json`. The scenario explicitly places the
third task after the completed-review restart hold. Earlier corrective fixture assertion failures
remain in their original evidence directories rather than being replaced by the passing runs.
The reviewed full gate passes 750 tests, 24 doctests and all 24 repository contracts, with five
manual tests ignored in the ordinary gate. Both required release controller longevity lanes pass
again. Four review faults in streamed usage overwrite, nullable detail parsing, raw usage retention
and permission-cost rounding are caught by assertions and restored. The model suite passes again.
Prior unspecified-unit, conflicting-charge and zero-spend-repeat faults remain applicable to those
unchanged owners in `target/corrective-03a-mutation-results.json`.
Exact commands and results are in `target/review-03a-final-gate-results.json`,
`review-03a-binary-results.json`, `review-03a-mutation-results.json`,
`review-03a-permission-mutation.json` and `review-03a-extra-results.json`. The final gate's initial
Clippy diagnostic identifies a redundant default initializer; after its removal, formatting,
full workspace checking/Clippy and the model suite pass. Eight default/all-feature inventories for
capability, model-provider, persistence and control under `target/public-api/review-03a/` match the
corrective inventories; the review fixes add no public surface.

Read-only configuration and bounded template/tokenization observations are retained in
`target/corrective-03a-server/observed.json` and `probes.json`. The one-token HTTP probe reports one
completion token; the oversized prompt is refused at the inspected 8192-token context. The SDK
process hit a Windows shutdown assertion after saving the probe results; these are retained
request observations, not a clean SDK lifecycle test. Before the reviewed real run, a fresh read-only
inspection confirmed the same accounting-relevant settings; its observations, comparison and copied
facts are under `target/review-03a-server/`. That inspection performs no loading or generation and
does not repeat the prior probes. No running server was reconfigured.
Earlier `target/model-budgets-*` reports remain evidence for their own source and settings, including
a stopped review and a 300-second agent timeout. Windows process termination/reopen does not
qualify power-loss durability or escaped descendants. Cloud testing was not authorized or performed.

The separate [local-model lane](../guides/local-model-endpoint.md#run-the-maintained-daemoncli-lane)
checks wait/restart/release, selected/omitted context, streaming/usage/artifact provenance,
nonduplication, post-entry response loss, unsafe-retry refusal, and explicit retain. Its deterministic
mode is non-qualifying. Real mode requires an explicit separately managed loopback profile and
never falls back to a mock. Both modes also run an isolated local-preparation refusal through
actual daemon/client binaries. A request-body limit rejects the attempt before entry intent;
the counting listener observes zero connections, and restart preserves that rejected attempt
without uncertainty or another request.

The deterministic lane also exercises production result acceptance with actual daemon/client
binaries: an empty exhausted model response retains a successful invocation, acceptance rejects
it, and a dependent endpoint stays unentered until an authorized repair produces usable review.
It restarts before evaluation, after rejection, and after acceptance before dependent dispatch.
Retained decisions, exact release-command replay, and endpoint entry counters check that restart
does not replace history or repeat work. The control tests cover structured-only and tool-only
contracts, whitespace, malformed decisions, checkpoint mismatch, justified no-change, unavailable
evidence, and failed publication. These mechanical checks do not qualify semantic review quality.

The September 12 acceptance change in the checkout based on `48fc896` passes the Windows/MSVC full
gate and both maintained actual-binary lanes. Its final deterministic report is
`target/result-acceptance-model-06/report.json`; the operator log is
`target/result-acceptance-final-headless.log`. Full-gate results and test counts are retained in
`target/result-acceptance-final-gate.json` and `target/result-acceptance-final-test-counts.json`,
with the corresponding `target/result-acceptance-final-*.log` files. The dependency audit logs are
`target/result-acceptance-deny.log`, `target/result-acceptance-machete.log`, and
`target/result-acceptance-duplicates.log`; affected public API inventories are under
`target/public-api/result-acceptance/`. The source and final binary hashes are recorded in
`target/result-acceptance-verification.json`.

Real model observations made during this change are retained separately. The first Bonsai alias
passes in `target/result-acceptance-real-bonsai-a/report.json`. The second alias times out at the
declared 180-second bound; `target/result-acceptance-real-bonsai-b.log` and that directory's
`recovered-run.json` and `recovered-attempt.json` retain the single uncertain attempt. Both requests
declare 4,096 output units and leave effective server thinking settings unknown. These earlier
observations cover the unchanged model/request path only; the final deterministic report covers
the finished acceptance and publication implementation. They are model-only observations, not a
new combined external-agent qualification or evidence of remote cancellation.

The model-preparation change based on `bf8cbd5` retains its actual-binary deterministic report at
`target/model-stages-deterministic-02/report.json` and its operator log at
`target/model-stages-headless.log`. The report covers zero-request refusal and restart, the existing
lost-response/unsafe-retry boundary, and result acceptance before dependent entry. The full suite
passes 727 workspace tests and 24 doctests; five manual longevity tests remain ignored. Gate results
are in `target/model-stages-gate.json` with corresponding `target/model-stages-*.log` files and
`target/model-stages-test-counts.json`. The initial duplicate-worker regression and its successful
correction are retained in `target/model-stages-initial-tests.log` and
`target/model-stages-duplicate-worker.log`. API inventories are under
`target/public-api/model-stages/`; `target/model-stages-verification.json` records source and binary
hashes. Preserved evidence binaries are under `target/model-stages-binaries/`.

Its real model smoke passes for `prism-ml/bonsai-27b` in
`target/model-stages-real-bonsai-a/report.json`. The separate `prism-ml/bonsai-27b:2` request reaches
the 180-second limit without a terminal. `target/model-stages-real-bonsai-b.log` and that output
directory's `recovered-run.json`, `recovered-attempt.json`, and `recovery-summary.json` retain two
reopens with one uncertain attempt and no replacement attempt. Both requests declare 4,096 output
units; effective thinking settings remain unknown. Only the first real lane completes, and neither
model-only observation qualifies the combined external-agent boundary or remote cancellation.

The reviewed September 12 controller integration based on `741b230` passes the Windows/MSVC full gate
(737 workspace tests, 24 doctests, all 24 repository contracts) and both required release controller
longevity lanes. Five manual tests remain ignored in the ordinary gate. Its rebuilt installed
scenario passes in `target/controller-review-scenario/report.json`; the ordinary operator log is
`target/controller-review-operator.log` and the deterministic model report is
`target/controller-review-model/report.json`. The installed report covers accepted prospective
repair followed by the exact cumulative stop, concurrent descendant admission, revoked approval,
cold replay, charged-artifact preservation, and an entered non-idempotent process remaining
uncertain through two reopens, plus the unaccounted-child refusal in both activation modes. The
counting model fixture proves conservative unknown-bound refusal. The prior supplied Bonsai profile
refusals remain scoped to `target/controller-scenario-final/`; review used deterministic fixtures
only. No qualifying real controller loop or thinking-mode claim is made. Production activation
stays refused.

`target/controller-review-verification.json` records the reviewed source hashes, base commit,
executable hashes, and checks. Exact binaries are preserved under `target/controller-review-binaries/`.
Gate results and logs are `target/controller-review-final-gate-results.json` and
`target/controller-review-final-gate-*.log`. Release logs are
`target/controller-review-lifecycle-longevity.log` and `target/controller-review-admission-longevity.log`.
Default/all-feature API inventories are under `target/public-api/controller/`; review introduces
no further exports. The verification command sequence is retained in
`target/controller-review-final.ps1` and `target/controller-review-final-gate.ps1`.
These local process-kill observations do not qualify power loss,
another platform, or an independently reviewed production operating decision.

[Strict external evidence](../guides/external-evidence.md) requires both a real byte-pinned coding
agent and a supported real model endpoint with operator-owned resources. Only its complete
validated report can qualify that interoperability boundary. Hermetic external mode tests the
harness. It cannot authorize production controller activation.

The accepted local combined report and its supporting evidence are retained under ignored
`target/external-interop-20260910/`. `acceptance.json` identifies clean candidate
`8c6cdb9137bc8f688fd6cc800d3adf416d4f4625`, its binary/build/run provenance, and the private session.
The report is `real-bonsai-03/report.json`, with BLAKE3 digest
`b3_182a6602a7276cee5e6daac4ae842fcc3612d0dd1891315392e08e89bd11a1bf`.
`review/accepted-handoff.md` retains the independent acceptance; `review/verification.json` and
`review/contract-validation.log` retain the artifact, journal, production-reader, and redaction
inspection. The exact binaries, full-gate logs, session, failed attempts, and private resource
profiles remain available at the paths in that record and handoff. Preserve this evidence before
clearing build output; Git retains source history, not these private files. Accepted coverage and
its limits live in [status](../product/status.md#current-validationevidence-snapshot).

The local Bonsai evidence does not qualify thinking mode. The operator subsequently reported
disabling thinking after the initial attempt; the retained endpoint profile does not bind that
server setting to each request. The truncated 64-unit probe and successful 4,096-unit probe
therefore do not isolate the effect of the larger budget. See the
[configuration evidence finding](virtual-office/whiteboard/issues/external-model-configuration-provenance.md).

The development-only evidence package shares one child lifecycle owner for deadlines, bounded
captured output, readiness, restart, CLI JSON decoding, and cleanup. Production configuration and
composition remain in the actual daemon. Build the daemon before isolated evidence-package tests.
Readiness probes have a two-second bound; subsequent scenario calls retain the control client's
ordinary thirty-second request timeout. The workflow observation deadline is a separate limit.
The successful external fixture explicitly selects a two-minute workflow observer: each wait covers
several sequential processes, acceptance steps and executable identity checks. Individual process
limits remain unchanged, and the stalled-read regression checks that observation stays bounded.
External fixture children have a five-minute outer bound covering their sequence of workflow waits,
restarts and compiler invocations; that bound does not replace any individual operation deadline.

## Mutation

The Cargo runner and [.cargo/mutants.toml](../../.cargo/mutants.toml) pin mutation tooling. Install
the tool outside workspace dependencies, then list or run a semantic shard:

```sh
cargo install cargo-mutants --version 27.1.0 --locked
cargo mutation-evidence authority --list
cargo mutation-evidence authority
cargo mutation-evidence retention
cargo mutation-evidence runtime
cargo mutation-evidence uncertainty
cargo mutation-evidence controller
cargo mutation-evidence context
cargo mutation-evidence peer
```

`CARGO_MUTANTS_JOBS` selects the campaign concurrency (default 2), and
`CARGO_MUTANTS_BUILD_TIMEOUT` sets a positive build deadline in seconds (default 180). A slow host
may require serial builds and a larger measured deadline. Preserve failed campaign artifacts and
rerun after infrastructure failures; a timed-out build or a file-lock failure is not evidence that
a mutant was caught. Finish workspace builds before running a campaign, particularly on Windows
where the live runner executable cannot be replaced.
On a host dominated by debug-symbol linking, Cargo's `CARGO_PROFILE_DEV_DEBUG=0` and
`CARGO_PROFILE_TEST_DEBUG=0` retain test assertions while omitting debug symbols. Record these
settings and `CARGO_BUILD_JOBS` alongside the campaign; they do not classify failed builds.
Hosted campaigns are manual: select one semantic shard and one partition per dispatch. Each run
uses one Linux runner, one mutation worker and a 60-minute job limit, with debug symbols omitted
and assertions retained. There is no automatic full-campaign matrix. Partitioning divides the work
but repeats baseline builds, so adding runners can increase total runner-minutes even when it
reduces elapsed time. Choose a partition small enough to finish within the job allowance; a timeout
leaves incomplete evidence, not a successful campaign. For example, a manual `peer` dispatch with
partition `0/8`, or local `cargo mutation-evidence peer --partition 0/8`, runs the first partition.
The pinned tool owns partition parsing and selection; qualification requires the union of every
partition to match the unpartitioned mutant list exactly. Partitioning preserves the selected
tests and enforces each mutation's separate deadlines.
Hosted builds allow 600 seconds and use two Cargo compiler jobs. The single mutation worker
keeps application tests from competing with another mutation's compilation. The peer shard's test
selection includes the redb owner's contracts as well as peer, daemon, process, and evidence tests.
The peer group excludes the three process/model-only external fixture scenarios, which configure
no peers and previously produced unrelated HTTP timeouts before peer assertions ran. They remain
in the full workspace gate and the authority/receipt groups. Peer service, daemon-peer, storage
corruption, and operational peer tests remain selected.
The runner passes each shard's complete package selection to Cargo for both the baseline and every
mutation. The pinned tool's `--test-package` option alone changes only mutation scenarios, leaving
the baseline dependent on which source packages happen to occur in a partition. A baseline must
exercise the same suite to detect missing prerequisites before they can make a mutation look caught.
Shards that run the retained-context binary tests include the daemon and shared process-helper
packages. Cargo builds their executables inside each isolated checkout, including any mutation;
prebuilt binaries from the caller's workspace cannot supply that evidence.
Shards selecting the application evidence package also build the CLI, which its source-authoring
checks invoke. Daemon composition fixtures allow ten seconds for ordinary processes and declare
matching invocation budgets; adapter-specific timeout tests retain their own exact bounds.
The runner prints each completed mutation, including caught and unbuildable cases, so CI logs
show progress during long campaigns instead of remaining silent until the final summary.
Set `CARGO_MUTANTS_MINIMUM_TEST_TIMEOUT` from a measured full selected-suite run when the host needs
more time; an inadequate automatic deadline must be rerun, not classified as a caught mutant.
Hosted shards allow at least 600 seconds for that suite.
Mutation checkouts retain Git metadata because application-evidence tests require exact source
provenance; a missing repository must never make a mutation look caught.

Scope covers authority conjunctions, application/peer idempotency and retention, runtime optimistic
replay/recovery/reconciliation, controller accounting, context budgets, peer lifecycle, and exact
catalog renewal with bounded registration retirement. It
excludes generated fixtures and unrelated constructors. A failing unmutated baseline cannot
qualify a campaign.
Controller transaction revalidation is included. Independent contract tests cover originating-run
binding, action bounds, exact revision guards, and canonical fingerprints. Cargo-mutants 27.1 skips
constructors named `new`; constructor fault injection must be recorded separately from its campaigns.

Retain each `mutants.out` directory with exact source identity, logs, and `outcomes.json`.
Inspect the actual failing test: an unrelated fixture timeout cannot qualify a caught mutant.
Cargo may stop at an early failing target before later owner tests run. Recheck each affected exact
mutation through its relevant owner contracts, using `--cargo-test-arg=--no-fail-fast` when needed,
and retain both the original failure and the recheck evidence.
Unclassified survivors and timeouts fail. Fix missing assertions or record an exact reviewed entry
in [.cargo/mutation-classifications.json](../../.cargo/mutation-classifications.json). Accepted
classifications are only equivalent behavior, unreachable under a validated public contract, or
mutation-tool limitation. The runner rejects duplicate identities and validates its classification
policy. When source moves, review the affected contract again and refresh its exact mutant identity
from the current list; a stale line number must not silently match another mutation.
A healthy benchmark cannot justify a survivor. Historical counts do not qualify new source.

The runtime failure-drain regression rejects prospective revision adoption while entered work is
still draining toward an already chosen failure terminal. The peer storage regression inserts
valid document envelopes with mismatched published source, capability or inputs, or inconsistent
observation/output counts. Both transaction readers must reject those corrupt records. These checks
exercise reachable recovery and corruption boundaries; producer validation does not make their
reader guards redundant.

The September 28 repair review reconciled all 32 survivors reported by
[run 36323713449](https://github.com/hartolit/milkdrift/actions/runs/36323713449): 27 are caught by
focused owner tests, four duplicated pre-write checks were deleted in favor of the transactional
record validator, and one nested-account predicate is classified under its validated transition
contract. Process-only and model-only unknown usage, serialized reopen and later measured settlement
exercise that contract. The defensive predicate remains in production.

The scoped campaigns use cargo-mutants 27.1.0 with passing baselines and no timeouts: record readers
catch 18 mutations, entry/accounting boundaries 12, client grant binding two, the controller assessor
16, and peer registry health/authority fields ten. These include additional operators and struct
field deletions emitted outside the requested regex. The latter exposed a missing disconnect-health
assertion, which is now caught by the full peer library suite. The campaigns qualify these exact
branches, not every hosted partition. Raw outcomes and failing owner assertions are retained under
`target/validation-repair/mutations-*`; the boundary/grant/controller campaigns use `0489101`, and
the final health campaign uses `b902a28`. No hosted rerun or physical product exercise is included.

Serving archival regressions use real adapter output and archive the execution at each observed
authorization boundary after workers join. Authorized direct output reads retain exact bytes;
inspect-only clients and peers without artifact metadata rights cannot receive archived output
references. Artifact lookup also survives retirement of its hot index, while zero and unrelated
sequences still refuse. Restoring the earlier hot-only lookups and authorization checks makes all
four regressions fail at these boundaries. The tests are in `peer_service::direct::archival` and
the peer-service artifact-transfer suite; raw weakening output is retained under
`target/adaptive-hosts/06/completion/ci/archival-weakening.log`.

The September 12 controller integration based on `741b230` has a focused changed-line campaign,
not a new full controller-shard qualification. Across 46 exact mutants, 36 are caught, five are
classified under validated contract invariants, and five are ill-typed `Default` substitutions.
`target/controller-mutation-summary.json` maps every selected identity to its retained outcomes
and classification; `target/controller-mutation-failure-review.json` identifies the failing tests.
The main all-feature run selects control, daemon, persistence, and runtime libraries plus
`control_service`, `structured_runtime`, and `durable_runtime`. Default-feature configuration and
exact runtime follow-ups close feature-dependent and child-pin coverage. An initial entry-test
timeout is preserved; after adding a bounded wait, that exact mutant is caught, not classified
from its timeout. A diagnostic run that also selected unchanged struct fields is excluded from
the changed-line result and retained separately. No unresolved selected mutant remains.

Commands and lists are retained in `target/controller-mutation.ps1`,
`target/controller-default-mutation.ps1`, `target/controller-mutation-exact-closeout.ps1`, and
`target/controller-mutants-selected.json`. Runs use cargo-mutants 27.1.0, one mutation worker,
two compiler jobs, and zero dev/test debug symbols with assertions retained. The main build/test
deadlines are 600/120 seconds; exact runtime follow-ups use 600/60 seconds. Raw outcomes remain
under the corresponding `target/controller-mutation-*/mutants.out/` directories.

Review's additional child-creation regression observes one unaccounted adapter entry on the
original implementation (`target/controller-review-child-baseline.log`). Inverting the new exact
controller-marker comparison is independently caught in `target/controller-review-child-mutant.log`.
The original source was restored before final verification. The prior 46-mutant results apply to
their unchanged production functions; this additional fault is separate from that campaign.

## Benchmarks and operations

Divan is pinned in the evidence [manifest](../../tools/evidence/Cargo.toml). The process fixture
emits 256 KiB to each of stdout and stderr with no shell, network, credentials, time, or randomness.
Model measurements feed fixed OpenAI-compatible and Anthropic SSE through production parsers.
The non-default provider `operational-evidence` feature exposes only that network-free driver.

```sh
cargo test -p milkdrift-evidence --test operational_contracts --all-features
cargo build --release -p milkdrift-evidence \
  --bin evidence-process-helper --bin operational-evidence \
  -p milkdrift-daemon --bin milkdrift-daemon
MILKDRIFT_EVIDENCE_PROCESS_HELPER="$PWD/target/release/evidence-process-helper" \
  cargo bench -p milkdrift-evidence --bench core_paths -- --test
```

Capture distributions and operational reports on Unix:

```sh
mkdir -p target/evidence
MILKDRIFT_EVIDENCE_PROCESS_HELPER="$PWD/target/release/evidence-process-helper" \
  cargo bench -p milkdrift-evidence --bench core_paths 2>&1 | tee target/evidence/benchmarks.txt
MILKDRIFT_EVIDENCE_PROCESS_HELPER="$PWD/target/release/evidence-process-helper" \
  target/release/operational-evidence --operations 256 --output target/evidence
cargo test --release -p milkdrift-capability-host --test effect_worker \
  bounded_queues_backpressure_and_forced_shutdown_preserves_unresolved_truth \
  -- --exact --nocapture
```

Benchmarks cover journal transactions/batches, projection rebuild/checkpoint tail, receipt turnover,
peer hot/compact replay, context discovery/selection/materialization, artifact publication/ranges,
process/model streams, and authenticated daemon round trips. Their fixtures and dimensions live in
[core_paths.rs](../../tools/evidence/benches/core_paths.rs); this guide is not a generated inventory.

The operational runner asserts a bounded frontier over 10,000 events, cold-receipt replay after
reopen, and sustained receipt/peer turnover. It distinguishes logical document bytes from physical
redb-directory allocation. Daemon phases measure sequential/concurrent load, bounded-queue overload,
slow SSE consumption, authenticated reconnect, post-overload recovery, and public Ctrl-C shutdown.
Recovery and reconnect use separate bounded phases; reconnect requires a fresh health observation
with a changed cursor. Retryable stream errors alone never establish reconnection.
The signal lane requires Unix; child thread counts use Linux `/proc` when available. A separate
blocking-adapter regression checks fixed worker backpressure and unresolved forced-shutdown truth.

Reports are `operational-evidence.json` and `scenario-summary.csv`, with scenario identity,
operation/byte counts, checksums, storage/reopen facts, latency, overload, stream/shutdown outcomes,
platform, Git commit/tree/dirty state, and `rustc -vV`. Synthetic selection and in-memory projection
serialization are distinguished from durable discovery and snapshot recovery. Credentials,
prompts, provider payloads, artifact bytes, and environment values are excluded.

## CI and qualification

| Workflow | Trigger and configured job allowance | Configured evidence |
| --- | --- | --- |
| [quality](../../.github/workflows/quality.yml) | PRs and pushes to `main`; one Linux job, 45 minutes. | Full Rust gate, actual CLI/daemon independent-host and operator scenarios, deterministic local model, and installed controller qualification/refusal scenario. |
| [platform](../../.github/workflows/platform.yml) | Manual; select one Ubuntu, Windows or macOS runner, 60 minutes. | Workspace checks and selected domain/protocol/client/process tests. |
| [mutation](../../.github/workflows/mutation.yml) | Manual; select one shard/partition, one Linux job, 60 minutes. | Selected mutation outcomes and logs; complete coverage requires every partition. |
| [benchmarks](../../.github/workflows/benchmarks.yml) | Manual; one Linux job, 60 minutes. | Smoke/full distributions, operational reports, worker saturation. |
| [stress](../../.github/workflows/stress.yml) | Manual; one Linux job, 60 minutes. | Receipt, peer, controller lifecycle/admission, and runtime frontier longevity. |

Actions use immutable SHAs, jobs have timeouts/read permissions/concurrency limits, and platform
logs remain available on failure. Workflow definitions establish configured lanes, not successful
execution. Closure requires successful runs on their declared hosts for the source being qualified;
a local or cross-target check cannot substitute.
Only quality runs automatically. A feature-branch push does not also launch a duplicate quality
job for its PR. The four additional evidence workflows have no push, PR or scheduled trigger;
their results must be requested when the change needs that qualification. This keeps an ordinary
PR update to one runner instead of automatically launching every platform and mutation partition.
The ordinary gate remains complete; a green quality run does not claim the additional lanes ran.

Use GitHub's **Run workflow** control, or an explicit dispatch such as
`gh workflow run mutation.yml --ref main -f shard=peer -f partition=0/8`, when that evidence is
needed. A workflow disabled in repository settings must first be enabled, after confirming the
default branch contains these manual-only triggers. A newer run supersedes an older run for the
same workflow and ref; platform runs are also separated by the selected OS. Retain partial logs
after cancellation, and do not count a cancelled or timed-out partition as complete coverage.

Deterministic fault/reopen, clock rollback, reservation/artifact, conformance, and corruption tests
prove software invariants, not filesystem power-loss behavior, sandbox strength, provider service
levels, or production traffic capacity. OS time remains trusted during daemon downtime. Physical
store size is a trend observation rather than a portable quota; benchmarks are neither universal
throughput guarantees nor allocator/network/TLS profiles. Keep raw reports, API inventories, and
timings outside source control. Report unavailable credentials, profiles, tools, or runners explicitly.

## Managed Linux installations

Run `cargo test -p milkdrift-managed-linux --all-features` and the redb `managed::` contract cases
for deterministic intent/action/result faults, exact receipt replay, guarded child transfer,
physical-evidence refusal, cancellation and restored-store fencing. Recovery permission tests use a
replacement with different resource requirements; transaction tests independently enforce originating
scope when a command key already exists. Concurrent-command regression checks prevent duplicate
platform drivers; service-start checks refuse foreign ownership before invoking the supervisor,
platform names and labels separate identical requests under different manager roots, and
prerequisite tests require enough UID/GID ranges for a worker beside its owned model. The lifecycle and attached-model
adapters use common conformance with real serving acceptance/artifact storage, prepared-envelope
allowance checks and a bounded model HTTP fixture. These establish contracts, not Linux containment.
Inspection regressions refuse zero or mismatched CPU quota/period values, including arithmetic
overflow, and disabled or contradictory `no-new-privileges` settings. Workers, models and protected
applications share that enforcement reader.

The opt-in actual Podman/Quadlet lane and its exact environment variables are in
[managed operations](../operations/managed-linux.md#platform-qualification). It includes common
worker conformance through the production resource/adapter path. Normal CI does not silently skip
missing prerequisites within that lane: an explicitly invoked lane fails without its recipe,
rootless engine or user session. Qualification must retain the actual image/model digests, Podman
and systemd versions, observed kernel limits/devices/backend, and preservation/removal outcomes.
A daemon reopen is not a host reboot; GPU configuration/log evidence does not alone qualify useful
GPU inference or shared-memory pressure on the UM790.

The 2026-09-22 review's full gate and API results are retained under
`target/adaptive-hosts/managed-02-review/`; it used optimization 1 with debug assertions and retained
the initial physical gaps. The follow-up evidence under
`target/adaptive-hosts/managed-02-qualification/` closes desktop Podman 6.1.2 worker and owned-CPU
lifecycle qualification, actual kernel limits, supervisor recovery without Milkdrift, ID-bound
cleanup under a real foreign-container collision, and the complete daemon/CLI path. It includes
concurrent model progress, busy-removal refusal, replay/restart and preserved data. The physical
lane retains its store on failure. Later idle-reboot evidence is stated in
[integrated host acceptance](#integrated-host-acceptance); managed Vulkan and pressure remain in the
[hardware issue](virtual-office/whiteboard/issues/managed-linux-hardware-qualification.md). The actual daemon/CLI
Gemma smoke is negative evidence: one entered attempt remains uncertain, without another entry,
when the strict response reader encounters LM Studio's unmapped `reasoning_content` deltas.
`gemma-final.log` and its retained session store preserve the result; `gemma-diagnostic-sse.txt`
records a separate bounded diagnostic request. No provider mapping or server setting was changed.

The follow-up desktop Ornith and Drifty smoke reports use an explicit per-request
`reasoning_effort: "none"`; their passing text streams do not qualify the default reasoning format.
Drifty uses an authenticated SSH forward to its NetBird-bound listener. The managed desktop model
call additionally passes finite-accounting admission and terminal settlement. Unknown optional
reasoning fields remain refused, and no remote-termination or inference-throughput claim is made.


Post-02 evidence for recipe schema 2 and mechanism v3 is retained under
`target/adaptive-hosts/post-02/`. It repeats the physical worker/owned-CPU and collision lanes with
independent container limits, a configurable alias and a BusyBox worker that has no Git/Rust/C
suite. The maintained Slotbook image binds its initial brief; actual daemon/CLI execution checks
its ordinary initializer and preservation of edited files. Both product paths exercise exact
replay after restart, headroom diagnosis, incompatible apply refusal, explicit update and safe
removal. Short varied deadline/cancellation and combined-output tests complement the existing
retained-uncertainty and lifecycle fault cases. Alternate model inputs and larger configuration
values have deterministic reader/generation coverage; only the recorded Ornith weights have new
owned physical evidence. The retained `target/adaptive-hosts/post-02/` manifests and reports identify
commands, inputs, final gate and API review. This supersedes recipe-1 evidence for changed
configuration consumers and leaves the existing hardware issue open.

## Published workflow capabilities

The focused suites are `milkdrift-control --test control_service published::`,
`milkdrift-daemon --test control_plane published::`, and
`milkdrift-daemon --test two_daemon_peer published::`, with `cargo test` and `--all-features`.
They exercise durable create/bind/start recovery, generation retention, caller isolation, bounded
nesting, service authority, cumulative accounting, output disclosure and archival. Preparation-hook
tests expire or cancel the public call before internal entry and require zero process entries.
Managed integration observes the exact editing handoff, its return between children, terminal
release, and retained claims after lost stop evidence and reopen. Peer protocol tests require
ancestry ceilings to survive serialization. The full gate includes these suites and the underlying
resource, authority, storage, runtime, adapter and CLI contracts.

The `published::managed::` control cases also lose a child's physical stop proof, tear down and
reopen the store, fence the exact child, and explicitly return its editing claim. Both cancellation
and authorized continuation are checked: a publication wrapper retains its `NoExternalEntry`
proof, while physical parents still require a fresh entry claim. Unrelated administrators and the
child's service authority cannot resume the original caller's claim. Stale callbacks refuse,
exact command replay is stable, and runtime uncertainty requires its own authorized resolution
before all holds settle. The physical-parent counterpart is redb's
`managed::exact_child_handoff_restart_fencing_and_authorized_return_never_have_two_editors`.
Removing the wrapper proof guard in an isolated worktree made the new continuation test fail;
the patch and assertion are retained in `target/adaptive-hosts/06/mutation-no-entry.*`.

For actual binaries, follow the [Slotbook setup](../../examples/adaptive-slotbook/README.md), then
run the following against a separate fresh prepared directory for each mode:

```sh
target/debug/slotbook-evidence qualify --root /absolute/private/published-direct --candidate /absolute/path/to/slotbook --published --invocation-mode direct
target/debug/slotbook-evidence qualify --root /absolute/private/published-workflow --candidate /absolute/path/to/slotbook --published --invocation-mode workflow
target/debug/slotbook-evidence qualify --root /absolute/private/published-peer --candidate /absolute/path/to/slotbook --published --invocation-mode peer
```

This lane uses one execution worker on the provider, its actual managed workspace, and the same
protected deployment method. It observes real parent/child editing claims and busy maintenance
refusal, exercises invoke-only disclosure and prospective repair, verifies immutable deployed bytes,
replays acceptance after restart, and removes its installations with data preserved. Peer mode uses
two daemons on one desktop. Seeded and corrected executables are explicit Rust fixtures; this lane makes no
live-model, second-machine, reboot or general security claim. Fault-injected loss and stop-proof
refusal remain distinct deterministic evidence, not physical interruption qualification.

The fresh desktop peer qualification under `target/adaptive-hosts/06/published-peer/` passed with
`--drain-before-renewal`. It observed the deployed service across daemon restart, then stopped only
that service through its owner before renewing verification. This permits the finite scenario
with two private container mappings while an unrelated service remains running. It establishes
sequential renewal, not enough capacity for verification beside both running services. The native
BusyBox worker lifecycle and protected verifier timeout/reopen/integrity tests also passed afresh;
their logs are under `target/adaptive-hosts/06/` and `target/adaptive-hosts/06-review/`, including
`native-verifier.log` and its retained `native/` inputs.

Original published-method evidence is under `target/adaptive-hosts/04-review/`;
the Rust follow-up retains native executable qualification under `target/rust-coherence/`. Current
configuration, caller permissions and operating limits belong to the
[published-method guide](../guides/published-methods.md).

## Selected learning and product variants

The [learning example](../../examples/adaptive-slotbook/README.md) continues the native published
source through `slotbook-evidence learn`. An operator profile supplies the proposal endpoint.
`slotbook-evidence model-fixture` is the separate deterministic HTTP lane; useful, unhelpful,
malformed, invented-evidence and forbidden-edit modes exercise the same adapter and readers.
Neither its synthetic usage nor the seeded application repair is real-model quality evidence.

Focused checks include `milkdrift-control --test learning`, daemon learning source-context tests,
the published control/daemon suites, the native Slotbook library, verifier cases and fixture reader.
Run them with `cargo test` and `--all-features`; the workspace gate includes them. Comparison tests
cover incomplete evidence, failed checks, paired regression, fixed input/verifier/configuration
identities, returned-product mismatch and account/submission bounds. A real host/runtime test
starts a narrowly granted method while its worker is unhealthy, refuses execution until health
returns, then completes the same invocation. Publication regressions replace the method during an
active call and inject failures before and after the publication commit, then reopen and replay.
Catalog tests distinguish changes to availability/contracts from diagnostic health refreshes.
The daemon resource test keeps an idle handle alive through shutdown and immediately reopens the
store, proving that the joined owner releases managed storage without a sleep or reopen retry.

The actual-binary scenario fixes four input pairs before model entry, retains three protected
submissions per method/input, restarts during the comparison and promotion, and checks separate
loan/class products before selecting one deployment. Its tool experiment records native bytes and
an exact OCI base, verifies a fresh managed setup and preserves working bytes during activation.
Knowledge supersession and prior context are checked across restart. These are finite desktop
observations, not model-quality, host-reboot or general deployment guarantees.

Evidence from 2026-09-25 lives under private `target/adaptive-hosts/05/` and the reviewed fresh
study in `target/adaptive-hosts/05-review/`; exact study inputs, reports and cleanup receipts remain
in those directories.
The earlier `verified` study proves an accepted pre-entry preparation refusal remains inconclusive,
automatic promotion is refused and the baseline publication remains available. Real Ornith
requests in `complete` retain malformed candidate responses and interrupted/timeout uncertainty;
none was substituted with a deterministic proposal. Raw requests, responses, stores and credentials
remain private local evidence under the selected retention policy.

The reviewed `05-review/qualified` study retains eight paired executions under the strengthened
input/output and public-completion checks: one baseline repair versus zero candidate repairs on
each input, passing final checks and settled accounts. Its separate policy promoted generation 2
and replayed the same publication after restart. The earlier `05/end-to-end` products retain their
baseline lineage; a later eligible comparison does not retroactively change their selected method.
Both reviewed candidate variants passed their own six checks, and class evidence could not publish
the loan target. Only the selected loan product was deployed. The result retains separate working
state, configurations, invocations and method lineage.
Fresh tool staging and an authorized drained worker update preserved the exact application bytes.
Explicit knowledge supersession left the earlier selection and model context unchanged; an update
for a different workspace was refused.

The review gate passes 943 unit/integration tests, 24 doctests and all 24 repository contracts,
alongside the remaining full-gate checks. Eight environment-specific tests remain ignored by that
gate; the native protected-verifier lifecycle also passed separately. The desktop's finite private
UID/GID pool caused an earlier renewal to remain unknown when two services occupied both mappings.
That refusal is retained in `05-review/study/`; it is not converted into a successful verification.

The fresh `target/adaptive-hosts/06/published-peer/learning-05/` study also passed all four pairs,
promoted generation 2 and verified both variants before deploying only the selected loan product.
With `--remove-disposable`, the driver drains its selected service before tool staging and removes
only the recorded study installations after the update/knowledge checks. It verifies exact
generation, recipe, accepted evaluation, resource inventory and preservation policy before cleanup,
including stopped installations; pending uses or changed dispositions refuse. The corrected driver
replayed the completed study without further model work. All 23 installations were removed, all
47 recorded volumes remained, and source application/guidance hashes matched after removal.
The unrelated service kept its ID, start time and zero restarts during that cleanup. These are
deterministic desktop observations; the physical host and real-model evidence is separate below.

## Integrated host acceptance

The maintained [Slotbook example](../../examples/adaptive-slotbook/README.md) provides the
`headless-cli-evidence --installed-hosts` and `slotbook-evidence develop` routes for physical hosts
and bounded source studies. Exact binaries, inputs and retained evidence live under
`target/adaptive-hosts/06/`; the final assisted run is under `completion/assisted/`.
Desktop–UM790 records establish direct and workflow process/model calls, exact replay with an
independent entry counter, owned CPU-service replacement/removal and automatic recovery after an
orderly idle reboot. They establish neither in-flight power-loss recovery nor managed Vulkan
isolation. Bounded live-source studies have not produced a verified application. Earlier response
losses remain uncertain. The latest alternating desktop Ornith 9B / UM790 Ornith 35B study completed
eight model calls with 32,768-token server contexts and 16,384-token output allowances. Seven
proposals failed compilation and one was refused for its rationale byte bound. The final model
source, authorized proposal and managed-workspace snapshot have identical bytes; repair claims in
the rationale were not treated as compiler evidence. No candidate reached verification or
publication. The driver retained every response
and compiler artifact and removed the exact recorded installations while preserving their volumes.
Private evidence is under `target/adaptive-hosts/06/completion/source-v6/`.

The operator explicitly selected assisted implementation for workflow acceptance. The corrected
source was submitted through `slotbook-evidence develop --assisted-source ... --maximum-attempts 1`
in a fresh private study on Drifty, using image
`sha256:91bdb925ac0f21a4205115badf9b5eea7e344dee4a363d32cd3279c9e8e36683`. Direct proposal provenance
distinguishes the assistant's correction from the preserved local-model response. All six unchanged
verifier checks passed and protected publication activated that exact 1,578,728-byte candidate.
The evaluation identity is
`b3_715b50df20dce80766df73e04a20cb5f5db99cb621c856c17b5c184f3f419425`.
The application remained healthy after the workflow finished. Exact start-command replay after
daemon reopen preserved the complete terminal run, four attempt identities, target version 13,
accepted evaluation and running container ID/start time. Preservation-aware removal then removed
both installations, with all four selected volumes physically present afterward. No model call was
made by the assisted route, and prior uncertain calls were not replayed or settled.

`completion/assisted/binaries.sha256` binds the tested CLI, daemon, driver, verifier and source;
`development-inputs.json`, `source-1-assisted-proposal.json`, `development-result.json`, `recovery/`
and `evidence/` retain the actual public-path records. The corrected source and managed snapshot
share SHA-256 `fcb1c21d136e595cac8f221fac29330a8f86874f6e13151d4b95eef7e718408f`.
`preserved-resources.log` records native post-removal inspection. The remote private root is
`/home/agent/milkdrift-acceptance/06-20260927/slotbook-assisted` on the operator-supplied `agent`
account. This establishes the finite assisted workflow, not unaided model coding, arbitrary
application correctness or a new physical reboot claim.

Other retained inputs remain distinct: `drifty-agent/installed-replay-reboot-v6.log` and
`installed-final-peer-replay.log` bind physical direct/workflow replay and the independent process
counter; `drifty-agent/reboot-automatic-recovery.log`, `reboot-retained-results.log` and
`reboot-artifacts.log` bind owned CPU-service recovery. Drifty used kernel `7.2.7-arch1-1`, Podman
6.1.2 and systemd 261.3 with delegated controllers. The attached native Ornith 35B Q4_K_M weights
have SHA-256 `42739874cc2ccfdb8523b23fbe52e29b2a7555c8176737ca9ca0b5d59859d41f`;
owned CPU inference used the same weights with a 4096-token context and 28 GiB service cap.
The later native two-model profiles and 32,768-token context observations are retained under
`completion/`; they declare self-hosted billing with no token charge. No managed Vulkan isolation
or inference performance promise follows from those attached endpoints.

The subsequent review retained fresh results under `target/adaptive-hosts/06-review/`: the full
gate passed 960 unit/integration tests, 24 doctests and all 24 repository contracts. Eight opt-in
tests remain ignored. The native protected-verifier renewal/timeout/reopen/integrity case and the
actual-binary independent-host scenario passed separately. The latter uses a deterministic model
and two desktop daemons. Default/all-feature public API inventories for redb-store, managed-linux,
peer-http, capability-host and evidence were regenerated without changes. The gate used pinned
Rust 1.95.0, optimization 1 and debug information 0 with debug assertions enabled. No physical reboot
or live-model generation was rerun in that review; retained records are not fresh executions.

The source-authoring and closeout review passes the full local gate under pinned Rust 1.95.0 with
the default unoptimized debug profile: 973 unit/integration tests, 24 doctests, warning-denying
Clippy and rustdoc, dependency checks, discovery and all 24 repository contracts. Eight opt-in
tests remain ignored. Logs are under `target/adaptive-hosts/closure-review/`; the earlier assisted
gate remains under `target/adaptive-hosts/06/completion/gate/*-assisted-final.log`.

The 14 source-driver tests and two source-writer tests cover bounded UTF-8/regular-file reading,
exact source and proposal replay, changed-input refusal, direct provenance, exclusive input modes,
the one-attempt assisted limit and symlink refusal. The review corrected unchecked reuse of cached
model responses and proposals: response bytes now match the product's artifact digest and size,
and authoring documents are reconstructed and compared before submission. The CLI reproduced a
retained model workflow's canonical document. Documentation contracts and CLI parsing checks cover
the maintained operator route after office closeout.

Retained assisted source/proposal/workspace bytes match, as do the terminal run and container
identity before and after replay. These inspected records support the physical result above;
this closeout review did not repeat live-model generation or physical-host operations. Local gate
results do not qualify a later hosted CI run.
