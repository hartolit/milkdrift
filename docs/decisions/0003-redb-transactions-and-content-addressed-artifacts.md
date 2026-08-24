# ADR 0003: Redb transactions and truthful local integrity

- Status: accepted
- Date: 2026-08-24

## Context

Milkdrift needs durable revision lookup, atomic command/event acceptance, workspace state,
discovery indexes, snapshots, and large artifact content. It is a local pre-release engine;
it does not have a host-held secret or monotonic external anchor. An unkeyed Merkle trie
inside the same database cannot prove freshness or origin because anyone able to replace or
rewrite the whole database can also recompute its hashes.

## Decision

`milkdrift-persistence` owns narrow durable documents and ports, and
`milkdrift-redb-store` implements them with named typed redb tables. One redb write
transaction commits a command result, its contiguous events, the authoritative run head,
the cumulative history-chain head/checkpoints, workspace accounting, artifact references,
and derived index changes. Expected sequence is checked against `RUN_HEADS` in that same
transaction. A command result is accepted only with its exact event range.

The adapter detects accidental partial corruption: malformed or non-canonical checksummed
documents; row key/document identity mismatch; missing or extra event rows around a head;
broken event checksums or cumulative history links; dangling or disagreeing direct indexes;
invalid workspace ancestry/provenance/accounting; invalid snapshots; and missing, wrongly
sized, or digest-invalid artifact content. Ordinary reads validate the rows they consume,
and an explicit bounded, resumable scrub walks the remaining authoritative and derived
families. A clean bounded health sample is not a complete-store proof.

The adapter does not detect replacement or rollback of the entire database, nor an attacker
who rewrites every affected row and recomputes unkeyed checksums. Those guarantees require a
future external keyed or monotonic trust anchor. The removed in-database authenticated trie
did not provide them and imposed path rewrites and duplicate membership state.

Authoritative immutable data consists of revision documents, event envelopes, workspace
scope/value versions, and committed artifact metadata. Authoritative aggregate state consists
of run heads, accepted-command results, history-chain heads, workspace usage/budgets, and
artifact publication/accounting coordination. Discovery rows, digest/ordered indexes,
pointers, and ownership/reference indexes are derived and verifiable, but no automatic repair
API is claimed. Snapshots are optional accelerators: invalid snapshots are rejected and the
runtime replays authoritative events.

Artifact bytes remain BLAKE3 content-addressed. Publication verifies and synchronizes bytes,
atomically renames and synchronizes the directory, then commits metadata; reads verify size
and digest. Durable path-inventory and delete-guard tables preserve crash-safe cleanup without
walking an authenticated structure.

Physical schema version 2 and internal document format 5 are exact-current only. Earlier and
future formats are refused; no migration is implemented. Pre-release users must create a new
store or wait for an explicit future migration tool rather than reinterpret old bytes.

## Rejected alternatives

- A generic key/value framework, because it obscures transaction and authority boundaries.
- Large blobs in events or redb values, because they duplicate content and weaken bounds.
- An in-database unkeyed authenticated structure, because it adds mutation cost without an
  external trust anchor.
- Recording artifact metadata before content durability, because accepted history could then
  reference missing bytes.

## Consequences

The integrity claim is deliberately limited and testable. Partial corruption remains explicit;
whole-store authenticity and freshness do not. Runtime startup validates active durable state,
while full historical and optional artifact-content verification remain operator-requested.
The blocking adapter must be hosted behind a bounded executor in a future asynchronous daemon.
