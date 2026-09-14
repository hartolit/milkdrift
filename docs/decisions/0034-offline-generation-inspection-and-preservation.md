# ADR 0034: Inspect and preserve generations outside runtime recovery

- Status: accepted
- Date: 2026-09-13

## Context

An unsafe retained context manifest can prevent ordinary startup before HTTP becomes available.
Opening storage normally also observes the durable clock and restores retention bounds; redb itself
can write housekeeping metadata. Reusing those open paths would not provide source-preserving
diagnostics. A physical clone without an execution guard could duplicate external effects.

## Decision

The daemon's explicit `storage-admin` path uses OS file authority without loading workflow grants
or constructing a runtime service. The redb adapter owns exclusive source locking through redb's
file backend, a verified private database copy and a facade exposing only reads. Artifacts stay
under the locked source. Normal store initialization, clock observations and retention are bypassed.
Runtime's existing pure projection and retained-context validator supply the application diagnosis;
they do not authorize recovery. Diagnostic projections withhold saved selection/omission identities,
sizes and content, including evidence written before the corrected selector.

Complete stopped-generation backup includes the database, artifacts and daemon execution
materializations. Unknown components and unsafe paths refuse the copy. A bounded schema-1 backup
manifest records file inventory/digests, current store formats, clock watermark and producer
provenance. The final completion manifest is written only after copying and compatible-reader
verification. Missing or malformed completion evidence refuses restore. Verification failures are
retained as a damaged-copy classification rather than a healthy-store claim.

Backups, restores and private inspection scratch carry `milkdrift-inspection-only.json` before
database copying starts. All normal redb store open paths refuse this marker. Removing it is an
explicit OS-owner activation decision requiring original-host/client fencing and review of effects;
it does not bypass ordinary recovery checks. The backup verifier requires the original guard.

## Alternatives and reconsideration

Ordinary read transactions were rejected because opening redb can write before the transaction.
A separate authenticated network service would introduce another service lifecycle just to read a
stopped root. OS-owner administration provides the required access through one application path.
Repairing retained manifests or deleting archives would change the evidence that inspection must
preserve and would not resolve external effects.

Reconsider the private copy when a supported redb read-only open can retain compatible writer
exclusion and pass the same source-byte/modification-time tests. Raise copy limits only with
measured larger-generation evidence that preserves bounded memory and explicit refusal behavior.

## Compatibility and consequences

Existing physical schema 11 and internal document format 16 are unchanged. Their strict readers,
event/manifests, command receipt/conflict behavior, peer tombstones, account obligations and clock
facts survive the copy exactly. No migration or historical repair is introduced. Backup schema 1
and diagnostic report schema 1 are separate from ordinary CLI JSON and control protocol versions.

Every offline operation copies the complete database within a fixed ceiling before logical reads.
Backups additionally traverse all included files and the integrity scanner within explicit limits.
The [operator guide](../operations/daemon.md#offline-storage-administration) owns permissions,
commands, bounds and activation procedure. Guarantees cover source bytes/modification times under
exclusive supported ownership; they exclude access times, power loss, authenticity and malicious
OS actors bypassing locks. A new empty generation never settles the old generation's effects.
