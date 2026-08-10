# redb-storage

ACID desktop persistence for application settings and the logical model catalogue.
All integer fields use little-endian encoding, strings use a `u32` byte length
followed by UTF-8 bytes, and decoders reject truncation and trailing bytes.

## Application settings

Settings saves use the existing table and key with the `LAS1` v2 device and
accelerator-memory-policy schema. Exact `LAS1` v1 records remain readable and
migrate in memory to CPU plus `Automatic` for a zero legacy device limit or
`Limit` for a nonzero value. Reads do not rewrite either version.

Application settings may retain an empty default repository before the first
repository selection; revision and drain timeout remain required. `LAS1` v2
continues to persist CPU or a CUDA ordinal and either automatic accelerator-memory
admission or an explicit nonzero limit.

## Model catalogue

New model saves use deterministic `LAM1` v3 records under the existing model
table. After the magic, version, name, repository, and revision, v3 stores:

1. a declaration-presence tag (`0` absent or `1` present);
2. only when present, a scalar code (`0` F32, `1` F16, or `2` BF16); and
3. `last_resolved_unix_milliseconds` as a `u64`.

Absence therefore has no scalar sentinel in v3. Exact legacy layouts remain
readable without an implicit rewrite:

- `LAM1` v1 requires one scalar code (`0`, `1`, or `2`);
- `LAM1` v2 uses scalar code `3` for absent metadata in its mandatory scalar slot.

The legacy timestamp bytes decode unchanged into the truthfully named
`ModelRecord::last_resolved_unix_milliseconds` field. Explicitly passing a record
read from v1 or v2 to `upsert_model` writes it back as v3.

Every model read verifies that the redb table key exactly matches the model name
embedded in the record. Cache paths, per-tensor scalar inventory, and execution
scalar are deliberately not persisted.
