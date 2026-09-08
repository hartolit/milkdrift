# Daemon operation and durable application state

Run one `milkdrift-daemon` for a configured data root. It owns workflow execution, storage, and
the configured capability adapters; clients use its authenticated API. A second store opener is
refused while the first holds the domain lock. Start with the
[operator recipe](../../examples/operator/README.md) for configuration, credentials, and a first run.

Configuration is TOML schema 9. `--check-config` validates it and resolves paths relative to the
configuration file; `--print-effective-config` prints normalized, redacted TOML. Neither starts
adapters or proves storage recovery. The running host uses an immutable compiled plan, so changes
to grants, profile sources, worker limits, or peer relationships require validation and restart.
The [authority guide](authority.md) owns grant choices and revocation procedure.

## Startup and readiness

Startup establishes what can continue before it admits new work:

1. Validate configuration, referenced paths, credential mappings, grants, and limits.
2. Open the [current storage formats](../product/status.md) and durable clock boundary. Legacy
   `control-state-v1.json`, `peer-executions-v1`, and `peer-artifacts-v1` paths are refused because
   their old ownership cannot be silently imported or ignored. No conversion tool is implemented.
3. Recover active runtime work with admission closed and construct the control service. Check
   bounded application-receipt and layout reads; this is not a complete historical integrity scan.
4. Register and health-check workflow-control and configured process/model adapters, then build
   relationships and recover serving-peer work if peers are enabled.
5. Start fixed effect workers, open admission, and return a ready host to the HTTP server.

The executable begins serving requests only after host startup succeeds. While it is starting,
readiness polling may fail to connect or receive a reply; inspect process exit and diagnostics if
it never becomes ready. Once serving, `daemon readiness` is the coarse readiness check, and
`daemon health` supplies separately authorized queue, worker, and retention details. Ready means
commands can be considered, not that every capability is available or every request is authorized.

Requests enter one bounded owner queue. Saturation returns overload; it does not create an
unbounded backlog. After a timeout, a queued command may still have been accepted. Preserve its
exact request for [command replay](../reference/control-api.md#commands). The
[daemon package guide](../../apps/daemon/README.md) follows this path through the implementation.

## Controller activation

Continuous controller activation is refused by this daemon. Neither a CLI command nor a
configuration change bypasses the [qualification gate](../product/status.md#limitations-now).
[Architecture](../architecture.md#controller-resource-accounting) defines the implemented library
accounting boundary.

## Application receipts and retention

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
| `peers.serving.maximum_hot_terminal_records`, `archive_batch_size`, `observation_hot_retention_ms` | Govern serving-peer detail independently; see [peer operations](peers.md). |

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

## Backup, compatibility, and repair

Stop the daemon cleanly before copying its complete data root. Artifact bytes live in the
content-addressed filesystem store; runtime/application/peer metadata, controller accounts, and
the clock high-water fact live in redb. Keep them together. This pre-release build accepts only
the [current formats](../product/status.md), with no migration. Do not edit rows or schema markers.

To start a new store generation, retain an independently verified complete backup of the old root
and configure an empty root. There is no automatic rotation or cold-archive export/delete command.
Retain the old generation for historical inspection with a compatible binary. Exact replay applies
within a generation: all callers must rotate a namespaced client epoch before reusing command IDs,
or a delayed old request can look like new intent in the empty store.

Administrative integrity scanning and proposal-index rebuilding are Rust storage APIs, not CLI or
HTTP operations. A storage administrator can use `StorageAdmin::scan_integrity` for bounded,
resumable verification, explicitly choosing artifact-content checks. Rebuild a damaged proposal
projection only after diagnosing its authoritative receipt links. See the
[redb administration guide](../../adapters/redb-store/README.md) and
[persistence ports](../../crates/persistence/README.md) for these operations. A healthy sample or
successful startup does not replace the full scan.
