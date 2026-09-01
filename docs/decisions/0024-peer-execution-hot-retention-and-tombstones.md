# ADR 0024: Bounded hot peer history and permanent compact tombstones

## Status

Accepted.

## Context

ADR 0018 made remote acceptance and request-id idempotency durable, but its retained-record ceiling counted terminal executions forever. Long-running peers therefore eventually rejected every new request even though old detailed observations were no longer operationally useful. Merely deleting old records would make a delayed exact retry indistinguishable from new intent and could repeat an external side effect.

Application command receipts, security audit, peer execution history, and core artifacts have different authority and retention semantics. Sharing a capacity knob or deleting artifacts as a consequence of peer-history compaction would couple unrelated truth owners.

## Decision

Peer execution persistence has three lifecycle classes:

- active records retain complete request, dispatch/claim/entry, cancellation, and observation state and count only against active and queue bounds;
- hot terminal or uncertain records retain complete reconnectable observation history and count against `maximum_hot_terminal_records`;
- compact tombstones retain permanent request/execution identity truth, immutable provenance and authority summary, cancellation and accounting facts, observation count and rolling digest, and either the final terminal observation or an explicit uncertain disposition.

Each observation append advances a domain-separated rolling chain digest. Eligible terminal/uncertain records are selected oldest first after `observation_hot_retention_ms`. One redb transaction inserts the tombstone, changes the execution/request location authority, deletes detailed observation and peer observation-artifact rows, removes the terminal/hot record, and updates independent active/hot/tombstone/archive counters. Faults expose the complete pre-state or complete post-state. Dual placement, missing placement, broken request/location indexes, counter drift, or a rolling-digest mismatch fails integrity verification and keeps peer admission closed.

Maintenance archives at most `archive_batch_size` records per pass. If a new request finds the hot terminal bound full, its admission transaction performs the same bounded reclaim before reserving a slot. Existing request identities are resolved first: same digest replays from hot or tombstone state and a different digest conflicts regardless of draining, catalog freshness, or current capacity. Active executions are never compacted.

Protocol 1.1 makes archival visible. Acceptance can be `accepted` or `archived`; lookup and observation pages identify hot versus archived history. Archived history returns the retained terminal or uncertainty disposition and never fabricates detailed observations. A consumer that races with compaction completes from that disposition and never submits replacement work.

Protocol 1.2 subsequently adds the exact queried request identity to every lookup result. This lets
the consuming client reject a semantically swapped response from an authenticated but untrusted
peer instead of trusting the HTTP path alone; earlier minor versions are now refused.

Peer compaction deletes only peer-owned detail and observation-to-artifact mappings. It does not delete or rewrite core artifact bytes, metadata, ownership, retention class, or provenance. Compact tombstones have no automatic destructive expiry. Physical reclamation beyond them requires an operator-managed, fully retained store-generation rotation and a new client request-id epoch.

## Consequences

Completed executions no longer consume live execution capacity forever, while exact replay/conflict remains durable for the store generation. Operational observation history is intentionally unavailable after the configured horizon, but its count/digest and final disposition remain auditable. Tombstones continue to grow on disk; they are compact identity truth, not a bounded audit log. Configuration schema 6, peer protocol 1.1, redb physical schema 7, and internal document format 10 are exact-current and older incompatible forms are refused without migration.
