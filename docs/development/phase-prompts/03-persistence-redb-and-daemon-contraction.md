# Pass 3 — Contract persistence, redb, and daemon ownership

Refactor the persistence contracts, redb implementation, and daemon composition into explicit cohesive owners while preserving every atomicity, integrity, recovery, protocol, and shutdown invariant.

Follow `00-pristine-readiness-contract.md` in full. Do not redesign runtime semantics or add storage features. This pass must make the authoritative host understandable without turning it into forwarding façades.

## Primary outcome

Leave a concrete ownership chain:

```text
persistence contract
  -> one redb transaction implementation
  -> one daemon owner/composition route
  -> one control/peer transport adaptation
```

The database remains an adapter. The daemon remains the sole live owner. Persistence contracts do not absorb redb mechanisms, and transport code does not become runtime or storage truth.

## 1. Trace every durable transaction family

Before editing, map all production producers and consumers of:

- journal/event/command-result commits;
- revision and snapshot storage;
- workspace values, scopes, imports, and accounting;
- artifacts and publication sessions;
- controller accounts/reservations/bindings;
- application receipts, layouts, proposals, and security audit;
- peer execution, observations, accounting, retention, and tombstones;
- clock high-water evidence;
- startup recovery indexes and integrity scans;
- daemon owner-queue calls and shutdown ordering.

For each family, identify the persistence port, validated request, redb transaction, indexes/derived rows, integrity validation, recovery behavior, and daemon caller. Remove any second owner or direct redb access outside the adapter.

## 2. Split persistence contracts by semantic lifecycle

Review `crates/persistence/src/controller_account.rs`, event validation/reference code, journal request types, artifact contracts, and package-root exports.

`controller_account.rs` currently combines a large contract model, transition logic, validation, serialization, and embedded tests. Organize it into private named modules such as model, identity, transition, validation, and tests only when those are real responsibilities. Keep one package-level public contract and one closed transition vocabulary. Do not create another crate or generic quota framework.

Refactor long event-reference validation, especially `RunEventKind::validate_for_run`, into family-specific private validators sharing one small invariant context. Preserve independent rejection of impossible history. Do not move runtime decisions into persistence merely because persistence validates stored facts.

Consolidate repeated checked-arithmetic, digest, strict-reader, and reference-validation mechanics only when they represent one persistence invariant. Reduce root re-exports to real ports and durable schemas.

## 3. Make redb schema declaration singular

Review redb schema initialization, validation, table definitions, version checks, and integrity phase registration. The current schema code repeats opening/checking many tables across initialization and validation.

Create one small declarative source of truth for physical table membership and required open/validation behavior when it can remove those synchronized copies without hiding types or transaction semantics. A local macro or typed registration table is acceptable when expansion remains reviewable. Do not build a database abstraction framework.

Ensure:

- initialization and validation cannot silently omit different tables;
- physical schema and internal document versions remain exact;
- older/future stores are refused under current policy;
- schema movement does not cause a version bump;
- every table/index family is included in full integrity coverage;
- fault injection still observes real transaction boundaries.

## 4. Contract large redb owners

Perform a responsibility review of:

- `adapters/redb-store/src/peer.rs`;
- `controller_account.rs`;
- `application.rs`;
- `journal/workspace.rs`;
- admin cursor/service modules;
- artifact publication/accounting modules;
- their integration-test support.

Split ports into private command/read/validation/accounting/retention or transaction-family modules only where responsibilities differ. Keep atomic operations physically close. Do not split one redb write transaction across several independently callable public services.

Remove repeated document decode/checksum/key validation, table-open boilerplate, retry loops, and error mapping through the smallest shared private owner. Preserve exact corruption classification and all fault/reopen behavior.

## 5. Contract daemon composition and owner queue

Review `apps/daemon/src/host.rs`, `http.rs`, `config.rs`, command/read modules, peer composition, authentication, clock, health, maintenance, and shutdown.

`host.rs` must cease being the place where every daemon concern accumulates merely because one process owns them. Leave one small composition/owner façade and separate private owners for genuine responsibilities, including as applicable:

- immutable startup plans and component construction;
- owner-thread request queue and occupancy lifecycle;
- runtime/control commands and receipts;
- read-model queries;
- adapter/peer registration;
- maintenance/recovery;
- health projection;
- ordered shutdown.

Similarly, split HTTP by control route families, peer realm, SSE, artifact ranges, and common public-failure adaptation where those are separable. Split configuration into strict wire input, normalization/validation, and effective owner-specific plans. Do not create modules that merely re-export or forward every call.

Preserve:

- one bounded owner queue for synchronous state;
- typed one-shot request/result behavior;
- no redb/runtime work on Tokio reactor tasks;
- no lock held around external adapter work;
- weak handles that cannot extend storage lifetime;
- closed admission during recovery;
- peer/effect workers draining while the owner can service final persistence/clock calls;
- exact readiness and shutdown truth.

## 6. Contract dispatch and read-model duplication

Review daemon command planning, control service adaptation, protocol DTO construction, and read-model projection for repeated parsing, validation, resource mapping, and error truncation.

One owner must parse and validate each semantic document. The daemon may adapt versioned protocol input into owner types, but it must not repeat blueprint, proposal, authority, runtime, or persistence validation. Read models may redact/project facts but must not recalculate semantic state differently from runtime/persistence owners.

Consolidate route authorization/resource mapping through typed declarations only where it removes repeated decisions and remains statically reviewable. Do not introduce dynamic route registries or middleware that hides operation/resource intent.

## 7. Cohesion targets for this scope

Every production file at or above 1,500 lines in persistence/redb/daemon must be split or materially contracted below that level through real ownership changes. At minimum address:

- `crates/persistence/src/controller_account.rs`;
- `adapters/redb-store/src/peer.rs`;
- `apps/daemon/src/host.rs`;
- `apps/daemon/src/http.rs`;
- `apps/daemon/src/config.rs`.

Also review all remaining >1,000-line files in these packages. Remove generic cohesion exceptions for corrected files. A remaining exception must name one specific invariant and demonstrate that model, validation, I/O, and tests are not needlessly combined.

Do not satisfy thresholds by minifying, using `include!`, arbitrary `part1` modules, wildcard re-exports, or moving production implementation into tests.

## Required proof

Add or strengthen independent tests for:

- schema initialization and validation using the same complete table set;
- missing, extra, malformed, checksum-valid corrupt, and cross-reference-invalid rows;
- every atomic transaction family under before/after commit faults and reopen;
- owner-queue overload, dropped response, panic containment, and occupancy release;
- startup corruption and clock rollback before readiness;
- shutdown with peer/effect workers requiring final owner persistence and clock calls;
- command/read behavior unchanged through actual daemon transport;
- exact old/future schema refusal and current canonical fixtures.

Run the full gate, repository contracts, public-API inventories, all changed redb/persistence/daemon focused suites, relevant mutation shards, operational evidence, and all affected release-mode longevity lanes.

## Completion threshold

This pass is complete only when:

- persistence ports, redb mechanisms, and daemon lifecycle remain distinct owners;
- schema/table membership has one implementation;
- no named large file remains a grab bag of separable responsibilities;
- direct storage/runtime bypasses are absent;
- daemon composition is traceable from startup through shutdown;
- production code and public APIs in scope are net contracted;
- durability, integrity, recovery, and shutdown evidence still passes.
