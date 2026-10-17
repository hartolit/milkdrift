# milkdrift-redb-store

This adapter implements Milkdrift's [persistence contracts](../../crates/persistence/README.md) with
one redb database and owned artifact directories. Runtime decides what task transitions mean; this
package makes accepted snapshots, receipts, resource holds and artifacts durable.

The current task format uses internal document format 22 and physical schema 20. Earlier task
documents lack the enforced-duration reservations and elapsed-time watermarks required by runtime.
Existing databases are locked through a read-only
handle and copied privately for the format check before redb can open the source for housekeeping
writes. The exact prompt-03 format (physical schema 19, internal format 22) gains two empty daemon
metadata tables and the new physical marker in one transaction. Existing task, receipt and host
identity rows keep their exact bytes. Unexpected tables, partial adoption and invalid accounting
refuse before the source opens for writes. Other older formats are refused without migration.
Keep preserved generations with their matching
binary and use an explicitly new directory for task development. Tested byte and modification-time
invariance establishes software-level refusal, not power-loss durability.

## Open and share a store

`RedbStore::open` uses the ordinary limits for an owned data directory. Compose `RedbStoreConfig`
with `open_with_config` for explicit artifact/read limits, task capacity, audit retention or a clock.
One `Arc<RedbStore>` supplies narrow traits to runtime and other consumers; callers receive no redb
transaction or table handle. Import the relevant persistence trait to call its methods.

Opening validates the directory, physical schema and required accounting anchors, then restores the
configured security-audit bound. The daemon name's finite receipt history is checked together with
its generation; opening does not scan every task record or recover external work.
The daemon's [startup composition](../../apps/daemon/src/host/startup.rs) owns recovery before admission.
Use [daemon operations](../../docs/operations/daemon.md) for setup, shutdown and backups.

`offline::OfflineStore::open` takes the exclusive source lock and opens a verified private database
copy. Its read-only facade exposes bounded redacted diagnostics, task/artifact queries and integrity
scans without normal open's clock or retention writes. Backup and restore preserve a complete
generation and mark each copy inspection-only. The [operator procedure](../../docs/operations/daemon.md#offline-inspection-and-backup)
explains private destinations, copy limits, completion verification and explicit activation.

## What a write makes durable

[`daemon.rs`](src/daemon.rs) binds a shared display name to the stable installation identity.
Initialization uses the configured seed only when metadata is absent. A guarded rename saves its
new name and actor-scoped exact receipt together, so losing the reply does not require another rename.
Its independent allowance retains 1,024 receipts with at most 16 KiB of canonical request and result
bytes per receipt. At capacity, reads and exact replay remain available; new renames refuse and no
accepted key is evicted. Startup scans at most that finite receipt set to verify its generation and
current name. These records neither create workflows nor consume task settlement reservations.

[`task.rs`](src/task.rs) atomically saves a workflow snapshot, discovery fields, exact command result,
managed-execution holds and capacity counters. Defaults admit at most 1 GiB of retained task documents,
reserved remaining growth, pending artifact writes and committed physical content combined. A separate
256 MiB or 100,000 receipt allowance stops admission at whichever limit is reached first.
`with_workflow_limits` selects explicit bounds. Missing accounting is corruption, never an empty store.

Each unfinished workflow reserves its possible output growth and remaining state growth. Mandatory
stop controls reserve 8 KiB and one receipt entry each. The stable server-issued stop identity consumes
its slot once; ordinary requests cannot use those reserved slots. Exact request replay succeeds before
new capacity checks, including after reopening. No task record or accepted key is evicted to admit work.
Artifact bytes retain their charge after a workflow settles, while unused reservations are released.

Native-directory claims use a separate bounded table. Whole-set acquisition checks equal and nested
physical paths in the same transaction. Overlapping readers may coexist; any overlapping writer
conflicts. Restart and elapsed time do not release claims. Durable settlement releases them, while
uncertain work continues to block conflicting access.

Artifact publication crosses filesystem bytes and database records. The
[publication path](src/artifact/publication.rs) saves resumable intent, accepts bounded sequential
chunks, verifies and synchronizes content, then commits metadata and accounting. A failure between
content publication and metadata commit may leave an unreferenced blob. Exact publication replay and
bounded cleanup reconcile these states. Deduplicated content has independently charged logical
metadata, while physical content is charged once. Pending and committed output share a frozen
per-invocation ceiling; task provenance requires durable entry in the exact owning workflow.

Managed task acceptance acquires frozen installation holds in the snapshot transaction. Descriptor
changes, physical fences, conflicting editors and removal guards remain enforced independently of
task state. Entered uses need physical stop evidence before release; an unentered accepted task can
release its hold on settlement. Task workflows do not support legacy parent/child editing transfers.
Independent candidate evaluation and protected deployment evidence remain owned by managed resources.

Peer records retain their own durable admission and external-entry boundary. Eligible records become
compact tombstones preserving acceptance and terminal disposition while detailed observations retire.
Security audit keeps a bounded ordered window. Neither mechanism evicts task history or expires
artifact content implicitly. Managed installation/evaluation active-slot limits likewise preserve
historical evidence and block new admission when lowered below current obligations.

[`sample_clock`](src/clock.rs) samples after obtaining the write transaction so a waiting writer cannot
mistake another committed observation for a rollback. `ClockWatermarkStore` also accepts exact
caller-owned observations. The watermark remembers observed time across restart; it cannot establish
elapsed downtime independently of the clock.

## Read and verify

Reads verify checksums, identities and the relationships needed for their result. Discovery pages task
snapshots in stable workflow order with a 4 MiB aggregate page target. A larger valid document is
returned alone, within its separate document bound; boundary callers perform current authority filtering. Integrity
scans separately cover task documents, artifact content and retained indexes, returning bounded
continuations instead of silently dropping work. A sampled health result does not prove every record
or filesystem byte is sound.

Run `cargo test -p milkdrift-redb-store --all-features` for the adapter contracts. Focused tests cover
exact replay and conflict after lost responses, capacity reservations, schema refusal, native path
claims, managed protection, peer retention and artifact recovery. The
[development workflow](../../docs/development/workflow.md) owns the required verification schedule.
These tests do not establish power-loss behavior or exactly-once external effects.
