# ADR 0023: Exact application replay with bounded hot receipts

- Status: accepted
- Date: 2026-08-31
- Refines: ADR 0022 application-receipt retention

## Context

External application receipts bind an authenticated actor and client command identity to the first canonical command digest and its exact accepted or intentionally durable rejected result. The prior single receipt table was safe but also served as a configured capacity ceiling. Once full, it refused every new identity even when the store remained writable. Evicting old receipts would permit command reuse, duplicate effects, and invented authority provenance.

## Decision

One receipt document has exactly one authoritative physical placement, hot or cold, within one store generation. Both tiers contain the same immutable schema-1 document. Exact lookup checks both and fails corruption if an identity appears in both. Administration pages merge both tiers in stable actor/command key order, so placement changes do not invalidate their cursor meaning.

The hot tier is bounded and has a derived completion-ordered index. Archival selects a configured bounded oldest-first batch. One redb transaction inserts every selected document into cold storage, removes its hot document and order row, updates hot/cold counts, and advances the archival generation and time. Faults before commit expose the original hot placement; a fault after commit exposes the complete cold placement. Stale archival generations fail explicitly.

A new application command transaction first checks both tiers for replay or conflict. When hot is full, that same transaction archives a bounded batch before committing the new receipt and any same-store layout or proposal effect. Runtime-owned effects keep their stable runtime transaction and reconcile a missing external receipt on redelivery as before. Daemon maintenance uses the existing bounded interval to reclaim a full hot tier proactively; command commit remains the final guarantee.

Proposal validation and streaming rebuild use the logical union of both tiers. Security audit, peer execution, runtime events/snapshots, and artifacts retain separate policies. This decision introduced daemon configuration schema 5 and redb physical schema 6; ADR 0024 later advances the exact-current forms to configuration 6 and physical schema 7 without changing application-receipt semantics. Older stores remain refused and no migration is claimed.

Exact replay is promised for the lifetime of one store generation. Cold receipt history grows until physical storage or an explicit operator-created generation boundary. Creating a new generation is an offline rotation, never automatic deletion. Callers must also rotate a namespaced client epoch before reusing command IDs across generations.

## Rejected alternatives

- Digest-only tombstones, because current command results and effect references must replay exactly.
- A finite replay horizon, because transparent cold storage preserves the existing contract without a wire-protocol weakening.
- A lifetime in-memory identity map, because long-running memory must not scale with durable history.
- Automatic destructive rotation, because it would silently change idempotency and provenance truth.
- A generic retention framework shared with peers, because peer execution has different observation, entry, uncertainty, and artifact semantics.

## Consequences

Configured record counts bound recent operational work rather than daemon lifetime. Cold storage may grow without a logical record ceiling, and real filesystem/redb exhaustion remains a truthful physical failure. Startup and integrity scans verify counts, hot ordering, tier exclusivity, proposal links, and receipt documents. Health exposes counts, bounds, archival generation/time, and redacted degradation without command or result content.

## Reconsideration triggers

Introduce export only with checksums, schema versions, effect references, verifiable counts, and a restartable independently verified completion boundary. Introduce a finite replay horizon only through an explicit versioned protocol and operator-visible contract after proving exact cold replay is insufficient.
