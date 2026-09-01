# 0029 — One daemon clock with durable rollback evidence

## Context

Peer authentication, relationship/catalog/transfer expiry, runtime scheduling, control receipts,
artifact publication, and observation streams all depend on boundary time. The embeddable runtime,
peer, and redb packages correctly expose separate narrow clock ports, but the daemon previously
composed them with unrelated system-clock implementations. Peer time rejected an in-process
backward observation, yet process restart discarded that evidence. A clock failure after durable
peer adapter entry could also defeat the worker's single recovery attempt and leave the exact claim
unresolved until a later daemon restart.

Time is an external capability, not semantic state manufactured by Milkdrift. The local operating
system remains the only available source of elapsed wall time while the process is absent. The
application still must not forget a later time it has already used for an authority or expiry
decision, and a transient failure must not erase the recovery identity of known-entered work.

## Decision

The daemon owns one fallible raw clock source in `host::clock`. Private adapters supply that source
to the existing runtime, peer, and artifact clock ports; those packages do not depend on the daemon
or share a generic clock abstraction.

The persistence contract owns one narrow `ClockWatermarkStore` port for externally supplied time
observations. Redb physical schema 9 stores the latest accepted Unix-millisecond observation. The
daemon owner queue serializes observations and advances that fact before returning time to runtime,
peer, control, or stream code. Artifact acceptance and startup receipt-retention recovery advance
it inside their own redb write transaction. Equal observations are read-only; older observations are rejected. Startup
checks and advances the watermark before recovery or readiness. Physical schema 8 and older are
refused so an older binary cannot reopen the store and ignore the safety fact.

Clock source failure, durable-store failure, queue failure, pre-epoch/overflow time, or rollback all
fail closed. The daemon emits a stable redacted log transition and health failure observation;
observation streams additionally record why they close. A peer worker that fails after claiming
work retains its exact recovery record, retries recovery before claiming anything else, and marks
known-entered work uncertain once the clock boundary recovers. Shutdown may stop that retry; normal
startup claim recovery then owns the still-durable record. Effect-worker ownership moves out of the
runtime owner for bounded shutdown so the owner continues servicing final clock/persistence calls;
the final owner-stop request is sent only after that drain result returns. Owner-queue adapters keep
weak sender leases so the owner does not keep its own receive channel artificially connected.

## Rejected alternatives

- Keeping only process-local monotonic state would allow a restart to forget a time already used
  for an expiry or authority decision.
- Placing a general clock in `milkdrift-contracts` would turn unrelated domain ports into a generic
  shared abstraction without shared domain meaning.
- Letting each package read and persist its own watermark would create competing time authorities
  and bypass the daemon owner boundary.
- Updating a sidecar file would create a second durability protocol outside the redb transaction
  used by authority and artifact facts.
- Periodically stealing every active peer claim for recovery would race healthy workers. Recovery
  instead retains only the exact record owned by the failing worker.

## Consequences

Every distinct daemon-observed millisecond may require a small redb metadata commit; equal
observations avoid a write. Clock availability therefore participates in the same bounded owner
queue and storage availability as other durable decisions. Physical schema 8 stores require an
operator-reviewed rebuild or migration and are not silently upgraded.

The durable watermark proves only time Milkdrift previously observed. Correct forward elapsed time
while the daemon is not running remains an explicit operating-system clock assumption; an external
trusted-time service can replace the raw source later without changing the inward ports or durable
comparison rule.

## Reconsideration triggers

Revisit the raw source when deployments provide an authenticated time service or a platform clock
with stronger cross-reboot guarantees. Revisit per-observation persistence only if measured owner
queue/storage cost is material and a replacement can preserve the same fail-closed restart and
artifact-transaction invariants.
