# redb-storage

ACID desktop persistence for settings and the logical model catalogue. Settings
saves use the explicit `LAS1` v2 device and accelerator-memory-policy schema under
the existing table/key. Exact `LAS1` v1 records remain readable and migrate in
memory to CPU plus `Automatic` for a zero legacy device limit or `Limit` for a
nonzero value. Application settings may retain an empty default repository before
the first repository selection; revision and drain timeout remain required. Model
catalogue records remain exact `LAM1` v1 and require name, repository, and revision.
Cache paths are resolved from repository and revision on startup instead of persisted.
