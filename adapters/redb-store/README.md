# milkdrift-redb-store

This adapter makes Milkdrift's [persistence contracts](../../crates/persistence/README.md) durable
on a local filesystem. It implements the journal, revision, workspace, artifact, account,
application, peer, clock, and administrative ports with one redb database and owned artifact
directories. Runtime decides what to commit; this package owns how those facts survive reopening.

## Open and share the store

`RedbStore::open` uses the ordinary limits for an owned data directory. Use
`RedbStoreConfig` with `open_with_config` when composing explicit artifact/read limits, receipt
archival, audit retention, or a clock. One `Arc<RedbStore>` can supply the narrow traits needed by
runtime and other consumers; callers do not receive a database transaction or table handle.
Import the relevant `milkdrift-persistence` trait to call its methods.

Opening validates the directory and exact physical schema, then restores configured application
retention bounds. It does not recover workflow executions or scan every historical record.
The daemon's [startup composition](../../apps/daemon/src/host/startup.rs) opens this store, supplies
its ports to runtime, and recovers active work before admission. Current versions and migration
limits belong in [status](../../docs/product/status.md). For setup, stopped-store backup, and
shutdown, use [daemon operations](../../docs/operations/daemon.md).

## What a write makes durable

[`journal/append.rs`](src/journal/append.rs) checks replay before optimistic guards. For a new
command it commits events, receipt/result, history links, workspace usage and values, discovery
indexes, and attached controller changes in one immediate-durability transaction. An error after
that commit can still reach the caller. Read or redeliver the exact command to determine acceptance;
do not infer rollback from the error alone.

Artifact publication crosses both files and database rows. The
[publication path](src/artifact/publication.rs) saves resumable intent, accepts bounded sequential
chunks, verifies and synchronizes content, then commits metadata and accounting. A failure between
content publication and metadata commit can leave an unreferenced blob. Exact publication replay
and bounded cleanup reconcile these states. Deduplicated content still has independently charged
logical metadata; replay does not charge it again.

[`application.rs`](src/application.rs) retains complete external receipts in hot or cold storage.
Archival moves ownership atomically, preserving exact replay and conflict checks across both
tiers. It does not expire old command identities. Peer records have a different lifecycle:
eligible records become compact tombstones that preserve acceptance and final disposition while
detailed observations retire. Security audit retains its own bounded window. These policies
do not delete runtime journal history or implicitly expire artifact bytes.

[`sample_clock`](src/clock.rs) samples the configured clock after obtaining the write transaction,
then advances or rejects the durable watermark. Sampling before waiting for a writer would let a
concurrent publication overtake that sample and falsely appear to move time backwards. The
`ClockWatermarkStore` port also accepts exact caller-owned observations. A watermark remembers
observed time across restart; it cannot establish elapsed downtime independently of the clock.

## Read and verify

Queries verify record checksums, identities, and the relationships needed for their result.
Run discovery uses derived indexes checked against journal facts; an empty or corrupt index
cannot be substituted for known active work. Snapshots are optional verified replay shortcuts.
Their journal-prefix and append-time commitments must agree before runtime can use them.

`StorageAdmin::health` returns a bounded sample. Use `scan_integrity` and retain its returned
cursor for a historical scrub, optionally including artifact content. Each page owns one read
transaction, not a frozen snapshot of the entire multi-page scan. Cursor validation binds its
store and verification mode. `ArtifactStore::cleanup_orphans` separately performs bounded cleanup
under the supplied age threshold and durable ownership checks; integrity inspection does not repair
or delete records.

The default build has no test helpers. `test-admin` enables fault injection and inspection used by
the integration suites. Run `cargo test -p milkdrift-redb-store --all-features` for transaction,
publication, retention, corruption, and reopen cases. Those software fault tests do not qualify
filesystem power loss or exactly-once external effects.
