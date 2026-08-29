# Daemon operation and durable application state

`milkdrift-daemon` is the only supported local owner of one configured data root. Clients use the authenticated control API; they never open redb, mutate the journal, or write application state directly. A second daemon/store opener fails with a typed owner-busy error while the first process holds the domain lock.

## Startup and readiness

Startup is deliberately fail-closed and ordered:

1. Validate daemon configuration, normalized paths, credential references, grants, and bounds before opening storage.
2. Refuse a data root containing legacy `control-state-v1.json`, `peer-executions-v1`, or `peer-artifacts-v1`. This release neither imports nor ignores old sidecar/prototype idempotency and artifact authority; move to a fresh data root or perform an explicitly reviewed offline conversion.
3. Open exact-current redb physical schema 5/internal document format 8 and the immutable artifact root.
4. Open runtime admission closed and recover active runtime state.
5. Validate bounded application-receipt and layout reads; corrupt or unsupported records fail startup.
6. Register and health-check workflow-control, process, and model adapters.
7. Build peer relationships and recover owned peer work when enabled.
8. Start bounded effect workers, resume runtime admission, and then report the API ready.

Until the final step, readiness returns unavailable and no external command is admitted. Axum owns sockets and streaming only; redb, runtime, control, layout, proposal, and artifact work crosses the bounded owner queue. Queue saturation returns overload instead of blocking an async reactor task or allocating an unbounded backlog.

## Application receipts and retention

The schema-3 configuration field `command_ledger_bound` is retained for compatibility but now bounds external application command receipts and, independently, the security-audit window. Receipts are never evicted: when the receipt bound is full, an unseen command identity is rejected with overload so an older idempotency result cannot be forgotten. Audit eviction never changes receipt retention.

An application receipt binds the authenticated actor, exact grant identity/revision/digest, client command identity, canonical command/schema digest, accepted or intentional deterministic rejected result, effect reference, and timestamps. Reuse with the same digest returns that stored result; reuse with another digest fails conflict. Same-store layout/proposal effects commit atomically with the receipt. Runtime/control effects reconcile through their existing stable internal command identities when a crash separates runtime acceptance from receipt commit.

The application tables are `milkdrift.v1.application.command_receipts`, `milkdrift.v1.application.layouts`, `milkdrift.v1.application.proposals`, and `milkdrift.v1.application.security_audit`. Records are independently keyed, bounded, versioned, and checksummed; growth does not rewrite a whole application document. Proposal rows are rebuildable derived discovery entries validated against authoritative accepted receipts. Layout rows use exact workflow/revision keys and optimistic generations and never participate in blueprint semantic identity or run history.

## Shutdown

Shutdown means owner completion, not merely stopping the HTTP listener. The daemon closes external admission/readiness, stops new peer acceptance and durable claims, begins peer/runtime draining, disconnects registries, closes runtime admission, and applies the configured `drain`, `cancel`, or `retain` effect policy until its deadline. It joins fixed peer and effect workers plus the owner thread before dropping redb/artifact handles. The final result reports whether shutdown was clean and how many worker/effect identities remain retained or unresolved; retained or uncertain work is never reported as successful completion.

## Backup, compatibility, and repair

Stop the daemon cleanly before copying its data root. Artifact bytes remain in the content-addressed filesystem store; application, peer execution, and runtime metadata remain in redb. This pre-release build implements no storage migration: physical schemas other than 5 and internal document formats other than 8 are refused. Do not edit rows or schema markers by hand.

Use the bounded resumable storage-integrity scan for administrative verification. Its current physical phases are `0..=39`; application phases validate receipts, layouts, proposal-to-receipt links, security-audit records, and receipt/audit counters. Proposal projection rebuilding is an explicit adapter operation and should follow diagnosis of projection damage, not replace validation of authoritative receipts.
