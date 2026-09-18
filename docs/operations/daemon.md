# Daemon operation and durable application state

Run one `milkdrift-daemon` for a configured data root. It owns storage, configured capability
adapters and, when enabled, workflow execution; clients use its authenticated API. A second store opener is
refused while the first holds the domain lock. Start with the
[operator recipe](../../examples/operator/README.md) for configuration, credentials, and a first run.

Configuration is TOML schema 10. `--check-config` validates it and resolves paths relative to the
configuration file; `--print-effective-config` prints normalized, redacted TOML. Neither starts
adapters or proves storage recovery. The running host uses an immutable compiled plan, so changes
to grants, profile sources, worker limits, or peer relationships require validation and restart.
The [authority guide](authority.md) owns grant choices and revocation procedure.

Set the required top-level `role` to `"workflow_enabled"` for local workflow execution, or
`"execution_only"` to serve configured capabilities without runtime/control services or workflow
workers. Execution-only startup requires disabled controller activation and refuses nonterminal
workflow runs, active leases or unresolved controller accounts. Preserve and inspect closed history
with `storage-admin`.
Health reports the configured role. Both roles expose authenticated direct execution and bounded
input uploads; use `milkdrift invocation catalog` to discover the exact callable generations.
Workflow-control capabilities are absent from execution-only hosts and from direct discovery.
Set a stable required `host_id`; an existing store refuses a different identity on restart.

## Startup and readiness

Startup establishes what can continue before it admits new work:

1. Validate configuration, referenced paths, credential mappings, grants, and limits.
2. Open the [current storage formats](../product/status.md) and durable clock boundary. Legacy
   `control-state-v1.json`, `peer-executions-v1`, and `peer-artifacts-v1` paths are refused because
   their old ownership cannot be silently imported or ignored. No conversion tool is implemented.
3. In workflow-enabled composition, construct the control service and install its single lifecycle owner when controller activation
   is explicitly configured, then recover active runtime work with admission closed. Check
   bounded application-receipt and layout reads; this is not a complete historical integrity scan.
4. Reclaim interrupted public input uploads through bounded pages, preserving resumable peer and
   workflow publications. Register and health-check configured process/model adapters, plus
   workflow-control when its role exists, then build configured peer relationships.
5. Start fixed effect workers only for the workflow role. Recover the common serving owner and
   open admission only after the other startup steps succeed, then return a ready host to HTTP.

The executable begins serving requests only after host startup succeeds. While it is starting,
readiness polling may fail to connect or receive a reply; inspect process exit and diagnostics if
it never becomes ready. Once serving, `daemon readiness` is the coarse readiness check, and
`daemon health` supplies separately authorized queue, worker, and retention details. Ready means
commands can be considered, not that every capability is available or every request is authorized.

An active lease with an unsafe older context selection also prevents ordinary startup. The saved manifest
remains readable, but its omissions may lack proof that required evidence was supplied or protected
metadata was hidden. Ordinary recovery refuses to forward it, leaving that startup's HTTP service
unavailable. Preserve the generation with offline inspection, then explicitly select
[authorized recovery controls](#authorized-recovery-controls) to review a prospective repair. See the
[retained-context decision](../decisions/0031-context-enforcement-and-retained-evidence.md) and
[offline inspection](#offline-storage-administration). Starting an empty generation does
not settle any outstanding effects in the preserved old root.

Requests enter one bounded owner queue. Saturation returns overload; it does not create an
unbounded backlog. After a timeout, a queued command may still have been accepted. Preserve its
exact request for [command replay](../reference/control-api.md#commands). The
[daemon package guide](../../apps/daemon/README.md) follows this path through the implementation.

## Controller activation

`runtime.controller_activation` defaults to `disabled`. Ordinary workflows run normally; marked
controllers cannot start without their lifecycle and cumulative account. Reopening active accounted
work with activation disabled fails before admission, including descendants whose own revisions
have no controller marker. Preserve that data root and use its compatible activation configuration.

`enabled` installs the existing control-owned lifecycle before recovery, using the same store,
authority service, and clock as other work. It is available in ordinary builds. The retained
`qualification` mode requires the non-default `controller-qualification` Cargo feature and uses
the same installation for isolated development evidence.
Installation is one-shot while admission is closed. A marked task cannot enter before its account
has been established, even if it precedes the repeat in the graph.
The child-creation transaction enforces the same rule for subworkflows: a child preceding
activation cannot enter under its own unmarked revision. The normal first controller child and
account establishment commit together.

Choose `enabled` only for the intended installation and its approved profiles, authority and
allowances. Source support does not change existing configuration. Supported model profiles bound
prepared text and settle declared billing; unknown and unsupported configurations remain refused.
The [accepted evidence and limits](../product/status.md#current-validationevidence-snapshot) describe
the exercised local external configuration and deterministic refusal/recovery cases.
See [accounting architecture](../architecture.md#controller-resource-accounting) and
[budget scope](authority.md#budget-scope).

## Application receipts and retention

Exact attempt inspection uses current attempt state when available, then the retained occurrence's
verified creation/terminal sequence anchors. A retired occurrence without an anchor requires two
bounded journal passes. Node inspection asks about the current frontier; timeline pagination asks
about all history. Reuse the returned cursor when browsing rather than repeatedly starting at page
one. A missing or rejected optional checkpoint can require full projection replay even for a small
read. No constant-time lifetime lookup is promised; see the
[measured workload](../development/verification-evidence.md#historical-query-cost).

A receipt lets a caller recover a lost reply. It binds the actor, exact grant, command ID, complete
canonical request digest, and accepted or intentionally retained deterministic rejected result.
An exact retry returns that result after restart; changing the request under the old ID conflicts.
For layout and proposal-discovery writes, receipt and effect commit together. If runtime/control
acceptance precedes the external receipt, redelivery recovers through the stable internal command
identity before committing the missing receipt.

Choose each retention setting for the state it owns:

| Configuration | Operator consequence |
| --- | --- |
| `application_receipts.hot_receipt_bound` | Limits recent receipts in the operational tier. Old complete receipts move to cold storage and still replay. |
| `application_receipts.archive_batch_size` | Limits one oldest-first archival batch. Maintenance and new-command transactions use the same atomic move. |
| `security_audit_record_bound` | Bounds retained security-audit entries independently of command replay. Old audit entries can be evicted. |
| `serving.maximum_hot_terminal_records`, `archive_batch_size`, `observation_hot_retention_ms` | Govern serving-peer detail independently; see [peer operations](peers.md). |

Startup re-establishes receipt/audit bounds, including smaller limits selected for a restart.
Maintenance refreshes capability health, schedules eligible work, notifies workers, and retires
eligible operational detail. Receipt archival preserves proposal discovery and exact replay;
it does not expire command IDs. Artifact retention and runtime history have separate owners.

Detailed health reports hot/cold receipt counts, configured bounds, archival generation/time, and
redacted failures. Peer health separately reports active, queued, hot-terminal, and tombstone
counts. Inspect degraded archival state and disk capacity when storage work fails. Cold receipts
and peer tombstones grow for the store generation; a hot count below its limit is not evidence
that total disk use is bounded. Peer recovery verification failure keeps admission closed.

## Shutdown

Stop a foreground daemon with Ctrl-C and wait for its process to exit before reopening the root.
The [operator recipe](../../examples/operator/README.md#startup-and-restart) explains the Windows
hidden-child case separately. Stopping a CLI observer does not stop the daemon.

Shutdown closes admission, begins peer/runtime drain, and disconnects registrations. The configured
`shutdown.effect_policy` chooses `drain`, `cancel`, or `retain` for outstanding effects, subject to
`shutdown.deadline_ms`. The storage owner remains available while peer and effect workers make
their final writes, then the host joins the owner and releases storage. A failed shutdown result
can retain unresolved work; neither a sent cancellation nor process exit establishes that an
external effect succeeded or stopped. After restart, inspect the exact attempt and follow the
[retained-work procedure](../guides/headless-dogfood.md#retained-or-uncertain-work).

For trusted processes, local pipe interruption prevents inherited stdout, stderr or unread stdin
from keeping owned I/O workers alive indefinitely. The process profile's termination allowance
covers final capture and joining. `drain` and `retain` still let already running work reach its
own bounded outcome; choose a daemon deadline that accommodates those invocation limits, or use
`cancel` to request termination. Incomplete pipe EOF remains uncertain through restart and appears
in the attempt's `terminal_detail`. Local worker completion is separate from external descendant
termination; see [process cleanup](../../adapters/local-process/README.md#bound-local-cleanup-without-claiming-descendant-containment).

## Backup, compatibility, and repair

Stop the daemon and wait for its process and owned workers to exit. Process exit does not prove
that a provider or remote worker stopped. `storage-admin backup` copies the complete closed
generation under the same exclusive database lock as a writer. Copying a live database and
changing artifacts separately cannot produce this guarantee.

A generation includes `milkdrift.redb`, `artifacts` (including unfinished `.tmp` publications), and
the daemon's `execution` materializations when present. The database retains run/attempt identities,
command results, hot/cold receipts, peer records/tombstones, controller accounts and the durable
clock. Materializations may contain sensitive inputs and working files; verification never resumes
them. Unknown root components, links/reparse points and special files are refused, not dropped.
Keep configuration, secret-source files and unrelated directories outside this data-root policy.
Backup never fetches referenced external files or resolves credentials.

Only physical schema 12 and internal document format 17 are supported by the
[current readers](../product/status.md), including offline. For other formats, preserve the untouched
root and its producer binary/source; inspection requires matching offline readers, which this
command does not supply. No migration, schema-marker patching, row editing or automatic repair occurs.

### Offline storage administration

`milkdrift-daemon storage-admin` uses OS file access, independently of workflow grants. It needs no
daemon configuration, credential or fabricated `ActorRef`. Ordinary startup never falls back to it.
No runtime service, effect workers, adapters, peer connections or HTTP service are constructed.

The pinned redb library can change housekeeping metadata during ordinary open. Offline administration
instead locks a read-only source database handle and copies and hashes it into private scratch.
Compatible readers open only that verified copy; artifacts are read from the locked source.
Source bytes and modification times remain unchanged, including failed inspection. Access times
and OS audit observations may change. No semantic clock advances, lease refreshes, account
settlements, retention, schema initialization or index rebuilding occur. Normal exit removes the
private database copy; an OS crash can leave scratch files for later operator cleanup.

Provide an existing private scratch directory outside the source. Unix requires current ownership
and mode 0700. Windows checks ownership and ACLs using system Windows PowerShell; only the current
account, SYSTEM and Administrators may have access. Broad inherited access is refused. Create a
new private parent in PowerShell, checking both commands succeed:

```powershell
$recovery = Join-Path $env:LOCALAPPDATA 'MilkdriftRecovery'
New-Item -ItemType Directory -Path $recovery -ErrorAction Stop
$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
icacls $recovery /inheritance:r /grant:r "$($identity):(OI)(CI)F"
```

On Unix, `mkdir -m 700 ./recovery` creates a suitable new parent. These examples use that directory;
in PowerShell substitute `--scratch "$recovery"` and destinations below it:

```sh
milkdrift-daemon storage-admin --root ./data --scratch ./recovery overview
milkdrift-daemon storage-admin --root ./data --scratch ./recovery runs --limit 32
milkdrift-daemon storage-admin --root ./data --scratch ./recovery records --family leases --limit 32
milkdrift-daemon storage-admin --root ./data --scratch ./recovery run --run RUN_ID --limit 32 --maximum-events 100000
milkdrift-daemon storage-admin --root ./data --scratch ./recovery records --family accounts
milkdrift-daemon storage-admin --root ./data --scratch ./recovery records --family cold-receipts
milkdrift-daemon storage-admin --root ./data --scratch ./recovery records --family peer-tombstones
milkdrift-daemon storage-admin --root ./data --scratch ./recovery records --family artifacts --hash-artifacts
```

Output is a schema-1 `inspection_report` with source database fingerprint and
`execution_authorized: false`. Redirect it only to a private location. It is a redacted report,
not a runnable backup. `overview` reports physical row counts and database bytes; record pages
report encoded bytes visited. Cold receipts/tombstones preserve permanent replay/conflict identity.
Their growth is a cost of retaining the generation, not a reason to delete them.

`records` also accepts `hot-receipts` and `peers`. It excludes receipt responses, peer requests,
grants and artifact content. Accounts expose totals, blocked state and exact reservations without
raw provider evidence. Artifact presence and size are checked; digest hashing is opt-in. Corrupt,
unsupported and oversized records occupy their page position with an explicit failure.

Pages accept 1–128 entries. Pass `report.data.next` unchanged to `--after`; record/scan cursors bind
source bytes, family and hash mode. Run-list continuations are exclusive run IDs. `run` replays
only the named history up to `--maximum-events` (default 100,000; maximum 1,000,000), then pages its
retained attempt frontier using `--after-attempt`. Compacted completed detail is explicitly outside
that projection. Each attempt shows revision, lease, uncertainty and frozen-context reuse checks.
`unsafe_but_readable` means the manifest decoded but cannot authorize reuse; unavailable/corrupt
content is separate. Policy, digest and bounded omission reasons remain visible. All saved
selection/omission source identities, sizes, provenance and content are withheld, including legacy
policy-v1 metadata. A passed context check is not an overall startup-readiness verdict.

Ordinary inspection performs no whole-history logical scan. To verify history deliberately, continue
the existing integrity scanner until `next` is null, retaining all page failures:

```sh
milkdrift-daemon storage-admin --root ./data --scratch ./recovery scan --limit 128 --hash-artifacts
milkdrift-daemon storage-admin --root ./data --scratch ./recovery scan --limit 128 --hash-artifacts --after "$cursor"
```

Set `$cursor` to the preceding `report.data.next`. Keep hash mode unchanged. Each response describes
only its page; failure details are redacted to component names. Index rebuilding remains a separate
Rust storage API requiring diagnosis of authoritative receipt links first. See the
[redb administration guide](../../adapters/redb-store/README.md).

### Create and verify an isolated copy

```sh
milkdrift-daemon storage-admin --root ./data --scratch ./recovery backup --destination ./recovery/backup-01
milkdrift-daemon storage-admin --root ./recovery/backup-01 --scratch ./recovery verify-backup
milkdrift-daemon storage-admin --root ./recovery/backup-01 --scratch ./recovery restore --destination ./recovery/restored-01
milkdrift-daemon storage-admin --root ./recovery/restored-01 --scratch ./recovery overview
```

Destinations must be absent, with an existing private parent outside the source. Existing stores
and backups are never overwritten. The bounded manifest records file lengths/digests, directories,
schema formats, clock watermark, producer version, executable digest and build-time Git revision/
dirty status when available. Source archives without Git explicitly report unavailable provenance;
the executable digest still identifies their producer. Verification hashes artifacts and traverses
compatible integrity readers without recovery.

`milkdrift-backup.json` is written last. A missing, truncated or inconsistent manifest refuses
verification and restore. Interrupted copies retain their execution guard. A completed copy with
`integrity_failures > 0` preserves damaged evidence; it is not a healthy-store recommendation.
Verification checks exact inventory and digests again, including the pre-housekeeping database
fingerprint and clock. Restore repeats these checks and creates another guarded complete copy.

Inspection copies at most a 64 GiB database with fixed buffers. Backup limits are 100,000
files/directories, 1 TiB of source files, a 32 MiB manifest and 100,000 integrity pages of 256 records.
Artifact trees allow three directory levels; execution materializations allow 32. Exceeding a
limit refuses completion. Record pages decode at most 8 MiB of encoded values, report individual
values above 4 MiB as failures without decoding, and bound final reports to 8 MiB. Artifact record
hashing has a 1 GiB per-file ceiling and reports `hash_bound_exceeded` above it. A small logical
inspection still needs scratch space for the complete database; unusually large stores may exceed
these operational limits.

Every copy carries `milkdrift-inspection-only.json`. All normal store-open paths refuse it before
opening redb, preventing accidental worker startup. Keep this marker for historical inspection.
Activation requires an explicit operator decision: fence the original daemon and its control
clients, account for external workers/providers, review outstanding effects and accounts, then
remove only this marker from the selected restored root and choose compatible configuration.
Removing it does not cure unsafe retained context: ordinary recovery still refuses that evidence.
Never run two copies of one generation or lower clock facts to bypass a rollback refusal.

To start a new store generation, retain an independently verified complete backup of the old root
and configure an empty root. There is no automatic rotation or cold-archive export/delete command.
Retain the old generation for historical inspection with a compatible binary. Exact replay applies
within a generation: all callers must rotate a namespaced client epoch before reusing command IDs,
or a delayed old request can look like new intent in the empty store. Starting empty does not settle
old effects. When startup succeeds, use existing authorized
[retained-work reconciliation](../guides/headless-dogfood.md#retained-or-uncertain-work). A blocked
generation can also receive authorized prospective repair through the explicit mode below.

These are software copy/read guarantees under exclusive ownership, not filesystem power-loss,
authenticity, rollback protection or protection from OS actors bypassing locks and renaming paths.
General export/delete, online rotation and historical migration remain unavailable.

### Authorized recovery controls

After preserving a blocked generation, stop and fence its previous daemon and external workers.
Start the selected generation with its existing configuration and grants:

```sh
milkdrift-daemon --config ./daemon.toml --recovery
```

This opens a mutable control service. It advances the durable clock, records ordinary commands,
security decisions and receipts, and maintains bounded receipt archival. It never schedules work,
recovers leases, fires timers, materializes execution directories, registers adapters, connects
peers or starts effect workers. Health reports `state: recovery`, `live: true`, `ready: false`;
readiness returns unavailable. Authentication and operation/resource authorization still apply.
The runtime handle cannot switch to execution: stop it and restart normally after repair.
Inspection guards on restored copies still require the explicit single-generation activation
decision described above; `--recovery` does not bypass the guard or support older storage formats.

Use the existing proposal path to replace the affected task prospectively. A schema-1 proposal
must name the run, exact base revision/digest and observed sequence, and carry the actual corrected
task/context policy as a blueprint mutation. Preserve the proposal file and each complete command
envelope for lost-reply replay. In recovery mode live proposals use `CancelAndRestartSafeWork`;
they never auto-apply, and application requires a recorded approval. Read the proposed revision
and reconciliation timeline before approving it. The run's original execution grant must still
authorize that revision; operator credentials do not replace its authority.

The normal CLI provides the operations (substitute the returned identities and current sequences):

```sh
milkdrift --command-id repair-submit --expected-sequence BASE_SEQUENCE --expected-revision BASE_REVISION proposal submit ./repair.json
milkdrift proposal show RUN_ID PROPOSAL_ID PROPOSED_REVISION
milkdrift --command-id repair-approve --expected-sequence PLAN_SEQUENCE --expected-revision PROPOSED_REVISION proposal approve RUN_ID PROPOSAL_ID PROPOSAL_DIGEST PROPOSED_REVISION DECISION_ID
milkdrift --command-id repair-apply --expected-sequence APPROVED_SEQUENCE --expected-revision PROPOSED_REVISION proposal apply RUN_ID PROPOSAL_ID PROPOSAL_DIGEST PROPOSED_REVISION
```

Endpoint and token configuration are the same as for the [operator CLI](../../examples/operator/README.md).
Stale sequence/revision guards, missing approval, changed replay content, insufficient or revoked
authority and unsafe reconciliation actions remain refusals. For a supported task cancelled before
dispatch, application records cancellation, releases its lease and reserves replacement work under
the new revision. It retains the old invocation, manifest, command results and accounting history.
It does not sanitize the old artifact or silently reselect context for the old attempt.

Stop recovery mode, start the same configuration without `--recovery`, and check readiness.
Normal startup validates every active run again before workers start. Replacement work receives
a new logical execution, invocation and manifest under the current selection policy. If the run
was paused, resume it with a fresh authorized command after normal startup. An unrelated remaining
blocker still refuses ordinary startup; repairing one run does not certify the whole generation.

Recovery also accepts pause, cancellation requests and existing evidence-based external-work
resolution. Create/start/resume, signal/timer delivery and controller continuation are refused.
A cancellation request alone does not settle an invocation. The existing safe-restart planner
supports side-effect-free/read-only work; unsafe side effects, missing/corrupt history or artifacts,
and controller budget/account violations are not repaired by this mode. Entered or uncertain
effects retain their normal evidence and reconciliation requirements. Context attachments and
artifact-content downloads are withheld in recovery mode because an older manifest may contain
undisclosed source identities; use offline preservation for the original bytes. Other reads retain
their ordinary scoped authorization and bounds.
