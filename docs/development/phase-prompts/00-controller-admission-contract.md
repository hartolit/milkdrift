# Controller final-entry admission contract

**Purpose:** Task contract for resolving ADR 0027. The repository’s `AGENTS.md`, vision, architecture, status, roadmap, ADRs, source, and tests remain canonical. Do not commit this file or create a duplicate design document in the repository.

## 1. Required end state

Milkdrift must have one durable resource-accounting owner for each exact continuous-controller occurrence. No controller-owned external capability entry or logical artifact publication may make the following invariant false:

```text
settled use + outstanding reservations + candidate obligation <= immutable controller limit
```

This must remain true under concurrency, retries, cancellation, delayed or missing usage, uncertain effects, duplicate delivery, artifact deduplication, crash/reopen, compaction, and legacy event histories.

The daemon may install `ControllerLifecycleOwner` only after this boundary and ADR 0027’s evidence are complete. Until then, marked controllers remain refused in production.

## 2. Ownership

Preserve the existing architecture. Add no controller scheduler and no generic resource-framework crate.

| Owner | Responsibility |
| --- | --- |
| `milkdrift-control` | Parse controller-policy schema 1, derive the immutable resource budget/account identity, assess controller lifecycle state, and consume the durable account view. It must not mutate or independently reconstruct resource accounting. |
| `milkdrift-capability` | Own the provider-neutral, request-specific admission-envelope contract shared by adapters and runtime. |
| `milkdrift-runtime` | Resolve immutable controller ancestry, coordinate the single final-entry path, and include accounting transitions in the same commit as the corresponding runtime fact. |
| `milkdrift-persistence` | Own validated account, binding, reservation, transition, and read-port contracts. |
| `milkdrift-redb-store` | Apply those contracts transactionally, maintain exact-current records/indexes, and verify whole-store integrity. |
| `milkdrift-capability-host` | Prepare one exact generation without entering adapter execution, expose its enforceable envelope, hold/release its permit through RAII, and carry the committed reservation into adapter artifact publication. |
| Concrete adapters | Derive truthful bounds from immutable host-owned request/profile facts or return `Unknown`. Provider estimates and later provider claims are not admission guarantees. |
| `milkdrift-daemon` | Validate/recover the accounting store and install the existing lifecycle before runtime recovery/admission only after qualification. |

A cross-crate type or trait is justified only where two real owners exchange the same semantic contract. Prefer private modules for implementation mechanics.

## 3. Canonical admission envelope

Add one narrow request-specific contract, with names adapted to existing conventions if necessary:

```text
AdmissionBound<T> = Bounded(T) | NotApplicable | Unknown

InvocationAdmissionEnvelope
  input_units
  output_units
  artifact_bytes
  monetary_cost { maximum_micros, exact_currency }
```

Requirements:

- `Bounded` is a host-enforceable upper bound for this exact immutable request and exact capability generation.
- `NotApplicable` is an asserted semantic impossibility, not missing data and not a synonym for zero.
- `Unknown` means Milkdrift cannot enforce that dimension before entry. A controller-owned invocation with any required `Unknown` dimension is rejected before `CapabilityAdapter::execute`.
- Cost currency must exactly equal the immutable controller-policy currency. No conversion or implicit default is permitted.
- Process/model invocation counts are derived by runtime from the frozen `CapabilityCategory`; adapters do not claim their own category.
- Duration remains owned by controller elapsed-time assessment and is not added to this resource ledger.
- Existing `ResourceObservations` remains observational. It must never be reused or reinterpreted as an enforceable bound.

Do not add tokenization, pricing, provider discovery, or a new model-profile family merely to avoid `Unknown`. Current model/peer generations without locally enforceable input/cost facts must fail closed for controller-owned entry while ordinary execution remains unchanged. A capability that can prove a dimension is impossible may return `NotApplicable`; otherwise it returns `Unknown`.

Every production adapter and every runtime test executor must implement the contract. Add a reusable conformance suite for common envelope rules, plus adapter-specific evidence that each bound is derived from the actual immutable profile/request limits.

## 4. One-shot prepared entry

Do not add a detached envelope preflight followed by a second racy generation lookup. Replace the runtime executor’s final execution path with a one-shot prepared-entry lifecycle, or an equally narrow mechanism with the same guarantees:

```text
TaskExecutor::prepare_exact_entry(dispatch)
    -> PreparedExecution

PreparedExecution
    - owns the exact generation and bounded host permit
    - exposes the immutable admission envelope
    - has not called CapabilityAdapter::execute
    - is consumed exactly once by enter/execute
    - releases the permit on drop before entry
```

The capability host must acquire the exact generation permit, release its registry lock, evaluate the adapter’s local admission envelope without holding that lock, and return an opaque prepared handle. Catch adapter panics during envelope derivation as pre-entry failures. The handle must reject a final dispatch that differs in run, revision, node, execution, attempt, lease, request, invocation, generation, or authority basis.

The runtime’s normal path becomes:

```text
validate exact active attempt/lease/generation
    -> re-evaluate final authority
    -> prepare exact generation and obtain envelope
    -> resolve immutable controller account binding
    -> build allowed or denied controller-account transition
    -> atomically commit final adapter-entry fact + account transition
    -> consume prepared handle and enter adapter exactly once
```

If authority is denied, no preparation or adapter entry is needed. If resource admission is denied, commit the exact denial and terminal rejection, drop the prepared handle, and never call `CapabilityAdapter::execute`. If the commit conflicts, drop the handle, reload authoritative state, and retry within a fixed bound. Never hold a database transaction or mutex while invoking external adapter execution.

A successful reservation commit followed by process failure before adapter execution remains a conservative outstanding obligation after restart. Do not manufacture a refund from absence of evidence.

## 5. Exact controller account and run binding

Create one immutable account declaration for one logical controller occurrence. Its identity must bind at least:

```text
controller run
controller node execution
controller policy digest
```

The declaration contains only the resource ceilings owned by this ledger:

- cost and exact currency;
- input units;
- output units;
- logical artifact bytes;
- process-category admissions;
- model-category admissions.

Controller cycles, revisions, proposal shape, elapsed time, failures, rejections, static repeat/child depth, and human checkpoints remain lifecycle-owned.

Each run has at most one immutable optional controller-account binding. The controller run and every controller-owned repeat body, nested subworkflow, retry, and detached child must inherit the same account at the durable child-creation boundary. Never infer ownership from actor identity, proposer text, workflow identity, mutable metadata, or whichever controller is currently active.

Binding replay must require exact equality. A conflicting binding is corruption/immutable conflict. A controller policy encountered inside an already account-bound descendant is refused until explicit nested/multi-account semantics exist. Multiple active-controller proposal attribution remains a separate limitation and must not be silently “solved” with heuristics.

`milkdrift-control` should derive the stable declaration from the validated policy and return it through the existing lifecycle boundary. Persistence owns account mutation. The runtime supplies a durable account view to lifecycle assessment; control must not query redb directly.

## 6. Durable account model

Use one closed, validated persistence module rather than scattered counters. The exact API may follow repository conventions, but it must represent:

- immutable account declaration and budget;
- immutable run-to-account binding;
- account revision/digest for optimistic comparison;
- settled totals;
- outstanding reservation totals and exact reservation records;
- blocked-unknown and contract-violation state;
- stable transition identity/fingerprint and exact replay behavior.

Use a small closed transition set for real lifecycle operations, such as account establishment/binding, entry admission, terminal settlement, artifact settlement/direct charge, and integrity blocking. Do not expose arbitrary delta mutation.

Every transition must use checked arithmetic, validate its resulting state, and be idempotent. Exact equality with a ceiling is allowed; the next positive obligation is denied. Monetary values from another currency are never combined.

The account’s committed view consumed by controller assessment is:

```text
settled totals + all unresolved reservation remainders
```

A blocked-unknown or contract-violation state also fails closed, even when the numeric worst case appears to leave room.

## 7. Atomic persistence boundaries

Extend the existing narrow transactions; do not build a sidecar store.

### Runtime journal commits

`AtomicRunCommitRequest` already coordinates events, workspace accounting, indexes, and lease-set optimistic state. Add the smallest validated controller-account transition/guard needed so redb atomically commits:

- the current final adapter-entry event and its resource-admission result;
- the corresponding account establishment, binding, reservation, settlement, or denial guard;
- all existing receipt, event, workspace, result, and index facts.

The current final event must remain the one canonical adapter-entry gate. Evolve `CapabilityAdapterEntryDecisionRecorded` (or replace it in the new current event schema) so it records both the fresh authority result and controller admission outcome: not controlled, exact reservation accepted, or exact resource denial. Do not create a second competing “entry decision” event path.

Use optimistic account revision/digest comparison where the event’s allowed/denied result was planned from a prior read. A stale plan retries; it never commits a decision against different totals.

### Artifact publication

Extend the existing `BeginArtifactPublication`/commit path and redb artifact transaction rather than adding an adapter-owned counter.

- An artifact published by an invocation consumes bytes from that attempt’s exact reservation.
- At first logical metadata commit, atomically move those exact bytes from the reservation’s outstanding artifact allowance into settled account use.
- A controller-bound runtime artifact without an invocation reservation, including causal context materialization, receives a deterministic direct charge against the run’s account at first logical commit.
- Publication replay performs no second charge.
- Content deduplication still charges logical published bytes; it is not a budget refund.
- Aborted or never-committed temporary publications charge nothing.
- A terminal output reference and later child-terminal aggregation must not charge bytes already charged at publication.
- Concurrent publications in different descendant runs cannot consume the same remaining account or reservation capacity.

If exact bytes exceed the reservation remainder or direct account remainder, reject before logical metadata/accounting commit. Preserve current global/workspace artifact bounds independently.

### Redb records and integrity

Add exact-current tables/records only where required by the durable account, reservation history/state, and run binding. Apply every related row change in the enclosing redb write transaction. Extend startup/full-store integrity to recompute or cross-check:

- declaration digest and policy binding;
- run binding uniqueness;
- account revision and totals;
- reservation ownership and summed remainders;
- artifact settlement links;
- final-entry references;
- impossible, missing, duplicate, or over-budget transitions.

New physical tables require an exact storage-schema bump. Changed stored record shapes require the appropriate internal-document-format bump. Preserve the repository’s current refusal policy; do not add an unreviewed online migration.

## 8. Reservation and settlement semantics

At an accepted final entry:

- immediately settle one process count for `CapabilityCategory::Process` or one model count for `CapabilityCategory::Model`; these are conservative final-admission charges and are not refunded after the durable commit;
- reserve every bounded input-unit, output-unit, cost, and artifact-byte maximum;
- treat `NotApplicable` as no obligation;
- reject any required `Unknown`, cost-currency mismatch, overflow, blocked account, or candidate that would exceed a ceiling.

For each exact terminal observation:

- when an authoritative observed value is present and does not exceed its reservation, move the actual value to settled use and release only the proven unused remainder;
- when a bounded input/output/cost value is absent, retain that remainder and mark the account unknown/fail-closed;
- settle artifact bytes only from durable artifact-publication facts; a terminal may release a proven unused artifact remainder after all publication operations for the attempt are closed;
- if observed use exceeds its enforceable envelope, durably retain the actual evidence, mark an adapter-contract/integrity violation, and block future admission; never silently expand the controller limit;
- exact replay or late duplicate evidence changes totals at most once.

Cancellation before final admission consumes nothing. After final admission, cancellation acknowledgement alone does not prove remote cost or usage. Release only dimensions for which the local capability contract and durable terminal evidence prove finality. An uncertain external outcome keeps every unresolved remainder. A retry receives a new reservation and must fit while prior uncertain obligations remain held. Later authoritative terminal evidence may settle the original reservation exactly once without rewriting the historical uncertainty fact.

## 9. Controller lifecycle integration

Remove the current projection-derived external resource accounting as the hard owner.

`ControllerLifecycleOwner` must consume the durable account’s committed totals and blocked state for cost, input units, output units, artifact bytes, process admissions, and model admissions. It may continue to encode those values in the existing progress/read shape if that remains exact.

Retain lifecycle ownership of controller cycle count, prospective revisions, proposal limits, elapsed time, failures, rejections, depth, and checkpoints. Remove static body-shape process/model pre-admission as a competing hard limit; final exact entries are the authority. Static body traversal may remain only for the depth invariants it genuinely owns.

Generic run/subworkflow `ResourceUsage` may remain an observational execution summary for non-controller features. It must no longer be used to enforce controller resource ceilings, and artifact/terminal aggregation must not mutate the controller ledger a second time.

## 10. Compatibility and refusal

- Keep controller-policy schema 1 unless its serialized semantics genuinely change; this task should enforce the existing policy rather than invent a new one.
- Add a new current run-event schema only when required by changed executable event meaning. Preserve exact v1/v2 readers and golden fixtures; the writer emits only the new current form.
- Bump projection/snapshot or public protocol schemas only if their persisted/wire meaning changes. Do not bump unrelated documents.
- Legacy controller assessments or marked histories without exact account/binding facts are unknown, never zero. Recovery and activation fail closed.
- Ordinary non-controller process/model/peer execution must preserve its current semantics and must not be forced to provide controller-only metering.
- No configuration flag, CLI command, actor grant, or embedding shortcut may bypass admission.

## 11. Scope exclusions

Do not expand this correction into:

- provider discovery, tokenizer implementation, pricing catalogs, currency conversion, managed sessions, or a new provider family;
- multiple-controller proposal attribution or nested controller accounts;
- a new controller scheduler, workflow primitive, UI, plugin framework, or generic quota service;
- process sandboxing or peer discovery;
- dynamic profile reload;
- automatic migration of old redb roots.

The implementation must leave clean extension points for future locally enforceable model metering, but unsupported facts remain explicit `Unknown` now.

## 12. Non-negotiable evidence

The finished boundary must independently prove:

1. Concurrent exact-bound admission: exactly `N` accepted entries and entry `N+1` denied before adapter execution.
2. Request-specific envelopes are deterministic, exact-generation-bound, and never sourced from `ResourceObservations`.
3. Prepared-entry drop/commit-conflict paths release the host permit without entering the adapter.
4. Retry after uncertainty retains the earlier reservation and requires new remaining capacity.
5. Pre-entry cancellation consumes nothing; post-entry cancellation does not manufacture usage certainty.
6. Missing bounded usage remains durable, blocks continuation, and survives restart.
7. Observed usage above an envelope becomes a durable blocking contract violation.
8. Exact artifact boundary succeeds; one extra logical byte is rejected. Replay, deduplication, abort, and crash/reopen never mischarge.
9. Every nested/retried/detached descendant inherits the exact originating account; actor/proposer changes cannot alter it.
10. Duplicate and late terminal/publication facts settle at most once.
11. Compaction/snapshot/restart retain account truth and do not reset limits.
12. Legacy marked histories without ledger facts fail closed.
13. Ordinary non-controller behavior and authority semantics remain unchanged.

Use reusable contract tests for open interfaces and independent state-machine/concurrency tests for persistence and runtime. Tests must observe adapter call counts and durable rows/events rather than merely call internal arithmetic helpers.
