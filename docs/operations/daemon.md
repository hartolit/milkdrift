# Daemon operation and durable application state

`milkdrift-daemon` is the only supported local owner of one configured data root. Clients use the authenticated control API; they never open redb, mutate the journal, or write application state directly. A second daemon/store opener fails with a typed owner-busy error while the first process holds the domain lock.

Operator-authored daemon configuration is TOML schema 9. Loading is bounded, rejects duplicate and
unknown keys, normalizes paths relative to the configuration file, validates cross-section safety,
and compiles the raw document into immutable storage, authentication, runtime, adapter, peer, and
shutdown plans. JSON has no fallback reader, and internal subsystems do not receive the raw global
document. Use `--check-config` for validation and `--print-effective-config` for normalized redacted
TOML before startup.

## Startup and readiness

Startup is deliberately fail-closed and ordered:

1. Validate daemon configuration, normalized paths, credential references, grants, and bounds before opening storage.
2. Refuse a data root containing legacy `control-state-v1.json`, `peer-executions-v1`, or `peer-artifacts-v1`. This release neither imports nor ignores old sidecar/prototype idempotency and artifact authority; move to a fresh data root or perform an explicitly reviewed offline conversion.
3. Open exact-current redb physical schema 9/internal document format 11, verify the durable clock high-water boundary, and open the immutable artifact root.
4. Open runtime admission closed and construct the shared control service. The production daemon
   deliberately leaves the experimental controller lifecycle uninstalled, so marked continuous
   controllers fail closed during recovery or activation.
5. Validate bounded application-receipt and layout reads; corrupt or unsupported records fail startup.
6. Register and health-check workflow-control, process, and model adapters.
7. Build peer relationships and recover owned peer work when enabled.
8. Start bounded effect workers, resume runtime admission, and then report the API ready.

Until the final step, readiness returns unavailable and no external command is admitted. Axum owns sockets and streaming only; redb, runtime, control, layout, proposal, and artifact work crosses the bounded owner queue. Queue saturation returns overload instead of blocking an async reactor task or allocating an unbounded backlog.

## Bounded controller contracts

A controller is an ordinary immutable revision containing the validated
`org.milkdrift/controller-policy` schema-1 extension and an explicit pinned `Repeat`. The control
and runtime libraries expose one `ControllerLifecycleOwner` for focused integration and recovery
tests, and never add a separate controller scheduler. The production daemon does not install that
owner because projection-time cumulative accounting is not yet an atomic reservation at every
final external-entry boundary. A marked controller therefore fails closed rather than running
with a limit that concurrent or newly admitted work could exceed.

The controller read/command DTOs remain available for inspecting any durable lifecycle history
created by an explicit embedding. `milkdrift-cli controller continue` cannot make the production
daemon install or bypass the missing owner. Production support requires an entry-adjacent ledger
whose reservation, accounting, retry, cancellation, and restart behavior is proven for every hard
resource dimension; merely enabling the existing hook is unsupported.

## Application receipts and retention

Configuration schema 9 uses `application_receipts.hot_receipt_bound` for the recent operational tier and `application_receipts.archive_batch_size` for one oldest-first move. Startup first re-establishes the configured hot-receipt and security-audit bounds, including when a restart selects smaller limits. The runtime maintenance interval performs bounded archival, and a new-command transaction reclaims enough eligible batches to preserve the current ceiling. `security_audit_record_bound` is a separate evicting audit-prefix policy. In enabled peer mode, `peers.serving.maximum_hot_terminal_records`, `archive_batch_size`, and `observation_hot_retention_ms` independently govern peer execution detail. Artifacts and runtime event/snapshot retention are not derived from any of those values.

An application receipt binds the authenticated actor, exact grant identity/revision/digest, client command identity, canonical command/schema digest, accepted or intentional deterministic rejected result, effect reference, and timestamps. Reuse with the same digest returns that stored result from either tier; reuse with another digest permanently fails conflict. Same-store layout/proposal effects commit atomically with receipt insertion and any required hot-to-cold move. Runtime/control effects reconcile through their existing stable internal command identities when a crash separates runtime acceptance from receipt commit.

Receipt documents live in exactly one of `milkdrift.v2.application.command_receipts.hot` and `milkdrift.v2.application.command_receipts.cold`; `milkdrift.v2.application.command_receipts.hot_by_completion` is a derived bounded order index. Moving a receipt is one redb transaction: cold insertion, hot/index removal, counters, generation, and time become visible together or not at all. Complete history pages merge both tiers in stable actor/command order. Proposal rows are rebuildable streaming derived discovery entries validated against either authoritative tier, so archiving does not change proposal identity or ordering. Layout remains in its independent table with exact workflow/revision keys and optimistic generations.

Detailed health reports hot count/bound, archive batch, cold count, archive generation, last successful archive time, and redacted degraded/failure state. It never includes command content or stored result bytes. Cold receipt history has no configured record-count ceiling and grows until physical storage is exhausted; such exhaustion is reported as a storage failure, not a logical retention limit.

Peer execution health separately reports active, dispatch, hot-terminal, and compact tombstone counts, configured active/queue/hot/batch bounds, archive generation/time, and a redacted degraded state. Startup recovers claims with bounded pages, verifies placement/index/counter/observation-chain invariants before opening admission, performs one eligible archival batch, and verifies again. Retention maintenance uses the same atomic move as admission. A failed verification keeps peer admission closed; it never silently rebuilds request identity or invents terminal evidence.

## Shutdown

Shutdown means owner completion, not merely stopping the HTTP listener. The daemon closes external admission/readiness, stops new peer acceptance and durable claims, begins peer/runtime draining, disconnects registries, closes runtime admission, and applies the configured `drain`, `cancel`, or `retain` effect policy until its deadline. It joins fixed peer and effect workers plus the owner thread before dropping redb/artifact handles. The final result reports whether shutdown was clean and how many worker/effect identities remain retained or unresolved; retained or uncertain work is never reported as successful completion.

## Backup, compatibility, and repair

Stop the daemon cleanly before copying its data root. Artifact bytes remain in the content-addressed filesystem store; application, peer execution, runtime metadata, and the boundary-clock high-water fact remain in redb. This pre-release build implements no storage migration: physical schemas other than 9 and internal document formats other than 11 are refused. The advance refuses persisted schema-1 authority decisions rather than reinterpreting legacy capability envelopes. Do not edit rows or schema markers by hand.

Exact command replay is preserved only within one store generation. To create a new generation, stop the daemon, make and independently verify a complete backup/export of the old data root, configure an empty new data root, and retain the old generation read-only for forensic/replay needs. There is no automatic rotation and no implemented cold-archive export/delete command. Command IDs must not be reused across generations unless every caller also rotates an explicit namespaced client epoch; otherwise a delayed request from the old generation is indistinguishable from new intent.

Use the bounded resumable storage-integrity scan for administrative verification. Integrity-cursor schema 2 uses physical phases `0..=41`; application phases validate hot receipts, cold receipts, hot completion order, placement exclusivity, layouts, proposal-to-receipt links, security-audit records, and independent counters. Proposal projection rebuilding is an explicit streaming adapter operation and should follow diagnosis of projection damage, not replace validation of authoritative receipts.
