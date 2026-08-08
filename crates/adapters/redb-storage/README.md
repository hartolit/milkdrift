# redb-storage

ACID desktop persistence for settings and the logical model catalogue. Settings
saves use the explicit `LAS1` v2 device and accelerator-memory-policy schema under
the existing table/key. Exact `LAS1` v1 records remain readable and migrate in
memory to CPU plus `Automatic` for a zero legacy device limit or `Limit` for a
nonzero value. Application settings may retain an empty default repository before
the first repository selection; revision and drain timeout remain required.

Model catalogue saves use `LAM1` v2 under the existing table and retain optional
configuration-declared scalar metadata. Exact `LAM1` v1 records remain readable;
their mandatory scalar code migrates in memory to present configuration metadata
without rewriting the stored record. New writes are deterministic v2 records.
Cache paths, per-tensor scalar inventory, and execution scalar are not persisted.
