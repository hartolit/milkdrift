# Daemon operation and durable application state

`milkdrift-daemon` is the only supported local owner of one configured data root. Clients use the authenticated control API; they never open redb, mutate the journal, or write application state directly. A second daemon/store opener fails with a typed owner-busy error while the first process holds the domain lock.

## Startup and readiness

Startup is deliberately fail-closed and ordered:

1. Validate daemon configuration, normalized paths, credential references, grants, and bounds before opening storage.
2. Refuse a data root containing legacy `control-state-v1.json`, `peer-executions-v1`, or `peer-artifacts-v1`. This release neither imports nor ignores old sidecar/prototype idempotency and artifact authority; move to a fresh data root or perform an explicitly reviewed offline conversion.
3. Open exact-current redb physical schema 8/internal document format 11 and the immutable artifact root.
4. Open runtime admission closed, construct the shared control service, install its single
   controller lifecycle owner, and only then recover active runtime state.
5. Validate bounded application-receipt and layout reads; corrupt or unsupported records fail startup.
6. Register and health-check workflow-control, process, and model adapters.
7. Build peer relationships and recover owned peer work when enabled.
8. Start bounded effect workers, resume runtime admission, and then report the API ready.

Until the final step, readiness returns unavailable and no external command is admitted. Axum owns sockets and streaming only; redb, runtime, control, layout, proposal, and artifact work crosses the bounded owner queue. Queue saturation returns overload instead of blocking an async reactor task or allocating an unbounded backlog.

## Bounded controllers

A controller is an ordinary immutable revision containing the validated
`org.milkdrift/controller-policy` schema-1 extension and an explicit pinned `Repeat`. The daemon
does not scan for work in a separate controller loop. Runtime scheduling calls the installed
`ControllerLifecycleOwner` before activation, each cycle, and checkpoint continuation; the
assessment and any admitted iteration are committed together. A marked controller cannot run if
the owner was not installed or its policy version/digest/wrapper binding is invalid.

Use `milkdrift-cli controller status RUN CONTROLLER_EXECUTION` to inspect the exact policy,
progress/limits, last durable assessment, checkpoint or bound, and eligibility. Continue a pending
checkpoint with `milkdrift-cli controller continue RUN CONTROLLER_EXECUTION DECISION_ID`; this is a
confirmed mutating command using the configured actor's normal approval authority. Restart does not
approve a checkpoint. A stale/duplicate different decision, grant revocation, elapsed/resource
bound, or terminal controller prevents continuation. Reaching a bound deterministically fails the
controller repeat without retrying a provider. Raising a bound requires an authorized immutable
revision and prospective reconciliation; the controller actor cannot widen or remove its own
policy.

## Application receipts and retention

Configuration schema 7 uses `application_receipts.hot_receipt_bound` for the recent operational tier and `application_receipts.archive_batch_size` for one oldest-first move. The existing runtime maintenance interval performs a bounded archive when the tier is full; the new-command transaction also reclaims a bounded batch if needed, so maintenance delay cannot create a permanent refusal. `security_audit_record_bound` is a separate evicting audit-prefix policy. `peers.serving.maximum_hot_terminal_records`, `archive_batch_size`, and `observation_hot_retention_ms` independently govern peer execution detail. Artifacts and runtime event/snapshot retention are not derived from any of those values.

An application receipt binds the authenticated actor, exact grant identity/revision/digest, client command identity, canonical command/schema digest, accepted or intentional deterministic rejected result, effect reference, and timestamps. Reuse with the same digest returns that stored result from either tier; reuse with another digest permanently fails conflict. Same-store layout/proposal effects commit atomically with receipt insertion and any required hot-to-cold move. Runtime/control effects reconcile through their existing stable internal command identities when a crash separates runtime acceptance from receipt commit.

Receipt documents live in exactly one of `milkdrift.v2.application.command_receipts.hot` and `milkdrift.v2.application.command_receipts.cold`; `milkdrift.v2.application.command_receipts.hot_by_completion` is a derived bounded order index. Moving a receipt is one redb transaction: cold insertion, hot/index removal, counters, generation, and time become visible together or not at all. Complete history pages merge both tiers in stable actor/command order. Proposal rows are rebuildable streaming derived discovery entries validated against either authoritative tier, so archiving does not change proposal identity or ordering. Layout remains in its independent table with exact workflow/revision keys and optimistic generations.

Detailed health reports hot count/bound, archive batch, cold count, archive generation, last successful archive time, and redacted degraded/failure state. It never includes command content or stored result bytes. Cold receipt history has no configured record-count ceiling and grows until physical storage is exhausted; such exhaustion is reported as a storage failure, not a logical retention limit.

Peer execution health separately reports active, dispatch, hot-terminal, and compact tombstone counts, configured active/queue/hot/batch bounds, archive generation/time, and a redacted degraded state. Startup recovers claims with bounded pages, verifies placement/index/counter/observation-chain invariants before opening admission, performs one eligible archival batch, and verifies again. Retention maintenance uses the same atomic move as admission. A failed verification keeps peer admission closed; it never silently rebuilds request identity or invents terminal evidence.

## Shutdown

Shutdown means owner completion, not merely stopping the HTTP listener. The daemon closes external admission/readiness, stops new peer acceptance and durable claims, begins peer/runtime draining, disconnects registries, closes runtime admission, and applies the configured `drain`, `cancel`, or `retain` effect policy until its deadline. It joins fixed peer and effect workers plus the owner thread before dropping redb/artifact handles. The final result reports whether shutdown was clean and how many worker/effect identities remain retained or unresolved; retained or uncertain work is never reported as successful completion.

## Backup, compatibility, and repair

Stop the daemon cleanly before copying its data root. Artifact bytes remain in the content-addressed filesystem store; application, peer execution, and runtime metadata remain in redb. This pre-release build implements no storage migration: physical schemas other than 8 and internal document formats other than 11 are refused. The advance refuses persisted schema-1 authority decisions rather than reinterpreting legacy capability envelopes. Do not edit rows or schema markers by hand.

Exact command replay is preserved only within one store generation. To create a new generation, stop the daemon, make and independently verify a complete backup/export of the old data root, configure an empty new data root, and retain the old generation read-only for forensic/replay needs. There is no automatic rotation and no implemented cold-archive export/delete command. Command IDs must not be reused across generations unless every caller also rotates an explicit namespaced client epoch; otherwise a delayed request from the old generation is indistinguishable from new intent.

Use the bounded resumable storage-integrity scan for administrative verification. Integrity-cursor schema 2 uses physical phases `0..=41`; application phases validate hot receipts, cold receipts, hot completion order, placement exclusivity, layouts, proposal-to-receipt links, security-audit records, and independent counters. Proposal projection rebuilding is an explicit streaming adapter operation and should follow diagnosis of projection damage, not replace validation of authoritative receipts.
