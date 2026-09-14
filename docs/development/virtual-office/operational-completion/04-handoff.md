# Assignment 04 handoff

Owner: Codex, assigned by the user at base `d537059`. Changes remain in the working tree.
No live installation or operator store changed. Assignment 05 has not started; the user or
coordinator accepts this result before advancing.

## Result and ownership

The daemon now has an explicit `storage-admin` command for offline inspection, backup, verification
and isolated restore. It needs no successful runtime recovery or daemon configuration. The redb
adapter owns exclusive source locking and the verified private database copy; the daemon composes
bounded, redacted reports with runtime's existing pure retained-context validator. The same validator
still governs retry/recovery. No alternate execution validation or migration was added.

The [operator procedure](../../../operations/daemon.md#offline-storage-administration) is canonical
for trust, permissions, supported commands, limits and disposition. [ADR 0034](../../../decisions/0034-offline-generation-inspection-and-preservation.md)
owns the copy/activation decision. Package READMEs, architecture and current status point to that
boundary. The previous generic backup guidance is replaced by executable create-new copy and
verification commands.

Copies preserve the database, artifacts, unfinished publications and daemon execution
materializations. The final bounded manifest records producer executable/source provenance, current
formats, exact file digests, durable clock and compatible-reader results. Incomplete copies lack
valid completion evidence. Every backup, restored root and inspection scratch clone has an
execution guard before database copying; normal storage opening refuses the guard.

## Verification

The complete local gate passes on Windows x86_64/MSVC with Rust 1.95.0. The final executable diff
passes 761 workspace tests and 24 doctests, including all 24 repository contracts. Five existing
manual longevity tests remain ignored in this ordinary gate. Formatting, all-target/all-feature
checking, warning-denying Clippy/rustdoc, dependency audits and test discovery pass. The dependency
audit retains its existing `syn` 2/3 duplication warning; no unused dependency or new lockfile entry
was introduced.

| Evidence | Result and retained location |
| --- | --- |
| Full gate | `target/assignment-04/fmt.log`, `build-final.log`, `check-final.log`, `test-final.log`, `clippy.log`, `doc.log`, `deny.log`, `machete.log`, `duplicates.log` and `discovery.log`. |
| Offline boundaries | Six redb offline unit tests prove source bytes/modification times, writer exclusion, pagination, exact cold replay/conflict and clock, incomplete/altered copies, execution guards, unsupported/corrupt records, linked-root and broad-permission refusal. The artifact contract preserves unfinished offsets and classifies missing/corrupt content; the account contract preserves a blocked reservation. All pass in `test-final.log`. |
| Blocked current-format generation | The structured-runtime `context_enforcement::offline_binary_inspects_blocked_legacy_context_without_disclosing_or_rewriting_it` scenario passes for required-evidence and protected-disclosure cases beside healthy history. It invokes the actual daemon, proves normal startup refusal and redacted offline inspection, and preserves frozen artifacts/database bytes through backup/restore. |
| Peer replay | `storage::retention::offline_restore_preserves_peer_tombstone_exact_replay_and_conflict` passes through the peer service contract in `test-final.log`. |
| Actual maintenance binary | Both tests in `apps/daemon/tests/storage_admin.rs` pass: offline command composition, create-new copy/restore, unchanged source, guard refusal and invalid/missing-root refusal. |
| Existing application scenarios | All three required lanes pass. Operator: `target/assignment-04/operator.log`; deterministic model: `target/assignment-04/model/report.json`; controller qualification: `target/assignment-04/controller/report.json`. Exact executable SHA-256 identities are in `target/assignment-04/binary-hashes.json`. |
| Public APIs | Four default/all-feature inventories were generated and reviewed under `target/public-api/assignment-04/`. Offline production surfaces match between feature modes; the private-directory fixture helper is absent by default. |

The application scenarios use the [maintained build and invocation procedure](../../verification-evidence.md#actual-binary-scenarios)
with `--output target/assignment-04/model` and
`--controller-output target/assignment-04/controller` to retain their disposable sessions. They use
deterministic loopback fixtures, not an external-model qualification. The daemon/process fixtures
were built before tests, and bundled Python 3 preceded Windows execution aliases on the harness PATH.
The final repository/documentation checks are retained in `target/assignment-04/repository-contracts.log`.

## Compatibility and limits

Physical schema 11, internal document format 16, control protocol 2.5, daemon configuration 9 and
ordinary CLI JSON 2 are unchanged. The backup envelope and inspection report each have schema 1.
Unsupported physical/document formats remain refused; no marker patch or row repair is supported.

Inspection preserves source bytes and modification times under exclusive supported ownership.
Access times and filesystem audit observations can change. Each invocation needs scratch for the
whole bounded database; backup additionally scans the complete included generation within explicit
file, byte, depth and integrity-page limits. Diagnostics report the scope actually read and keep
unavailable, corrupt and unsafe retained evidence distinct. They withhold all retained selection/
omission source identities, sizes, provenance and content.

A verified physical copy can preserve unsafe context or damaged evidence without making it safe to
run. Nonzero integrity failures remain explicit. Activation requires the operator to fence the
original host/clients and review unresolved external effects before removing the guard; ordinary
recovery validation still applies. A new empty generation requires fresh client/request namespaces
and does not settle old effects. Cold receipt/tombstone deletion, online rotation, historical
migration, filesystem power-loss guarantees and forensic authenticity remain outside the result.

## Public API review

The redb offline facade, typed diagnostic results and backup envelope are consumed by the daemon;
they are workspace adapter and versioned document contracts. Paths, row decoders, copy machinery,
cursor encodings and integrity traversal stay private. The shared execution-directory name and
inspection guard identify the complete-generation/activation contract. The database copy ceiling
stays private. Runtime exposes only its existing pure retained-manifest validator for the daemon's
diagnostic composition. The private-directory fixture helper is gated by `test-admin` and is used
by cross-crate storage tests. Default and all-feature inventories are retained under
`target/public-api/assignment-04/` for the affected redb/runtime packages.

## Next assignment

Assignment 05 can consume this offline operating path and unchanged retained-context rules. It must
not reinterpret an inspection result as permission to reuse context or treat a new store generation
as settlement of an old attempt. No provider, workflow primitive, network read-only mode or normal
CLI storage dependency was introduced here.
