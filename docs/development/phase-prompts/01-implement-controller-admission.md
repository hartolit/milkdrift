# Implement controller final-entry admission

Implement the complete controller resource-admission boundary defined by ADR 0027 and the attached `00-controller-admission-contract.md`.

This is an implementation pass, not a planning or review-only pass. Modify the repository, finish the boundary end to end, run the required verification, and leave a usable patch/commit. Do not enable continuous controllers in the production daemon during this pass.

## Governing material

Read in this order before editing:

1. `AGENTS.md`
2. `docs/product/vision.md`
3. `docs/architecture.md`
4. `docs/development/engineering-rules.md`
5. `docs/product/status.md`
6. `docs/product/roadmap.md`
7. `docs/decisions/0026-durable-bounded-controller-lifecycle.md`
8. `docs/decisions/0027-controller-final-entry-reservations.md`
9. `docs/development/workflow.md`
10. `docs/development/verification-evidence.md`
11. the attached task contract

Inspect current source, consumers, tests, manifests, schema constants/fixtures, and Git state before choosing exact APIs. Preserve unrelated user work. Do not reset, rebase, rewrite history, or copy this prompt into the repository.

## Required implementation

### 1. Establish one design and remove competing paths

Trace every current producer and consumer of:

- `ControllerProgress` resource fields;
- `RunProjection::resource_usage` and `subworkflow_usage_for_execution`;
- `CapabilityAdapterEntryDecisionRecorded`;
- `TaskExecutor::execute_streaming` and all implementations/fakes;
- `CapabilityAdapter` and every production implementation;
- `ResourceObservations` and `UsageObservation`;
- `AtomicRunCommitRequest` and every constructor;
- `BeginArtifactPublication` and every constructor/publication path;
- child/subworkflow run creation and inheritance;
- daemon runtime/control construction and recovery.

Implement one canonical path across all applicable producers and consumers. Delete obsolete resource-enforcement logic and adapters created only to preserve the old path. Do not leave TODOs, parallel ledgers, fallback inference, or a “temporary” controller-only shortcut.

### 2. Add the enforceable admission envelope

In the capability domain, add the smallest validated public contract for explicit `Bounded`, `NotApplicable`, and `Unknown` request-specific resource dimensions. Keep fields private, constructors fallible, serialization strict where persisted, and invalid currency/bound combinations unrepresentable.

Add the narrow adapter method needed to derive an envelope from the exact immutable generation/request. Implement it for all production adapters and test adapters. Derive only host-owned guarantees:

- local-process bounds from the exact validated process profile and materialization/publication limits;
- workflow-control facts from its actual no-external-resource behavior;
- model-provider and peer facts from exact request/profile/relationship limits where genuinely enforceable, with `Unknown` for missing tokenization, pricing, or other unverifiable dimensions;
- no use of descriptor `ResourceObservations` as an admission guarantee.

Add one reusable adapter-envelope conformance suite and mechanism-specific assertions. Do not introduce tokenizer/pricing/provider-discovery scope in this pass.

### 3. Replace the final executor path with a prepared-entry owner

Refactor the runtime-owned `TaskExecutor` boundary and capability-host implementation so final execution uses a one-shot prepared handle that owns the exact generation and permit without entering `CapabilityAdapter::execute`.

Fully migrate:

- `CapabilityHost`;
- `DeterministicExecutor`;
- runtime integration fakes;
- peer or direct-host consumers that share the same acquisition/execution lifecycle where applicable.

The prepared handle must expose the exact envelope, bind all immutable dispatch coordinates, release on drop, and be consumed exactly once after the durable final-entry commit. Avoid a public trait if a concrete/private owned handle can satisfy all real implementations; if runtime selection requires an object-safe trait, keep it narrowly consumer-owned and add conformance tests.

Ensure no registry/database lock is held while adapter code runs. A resource denial, authority denial, stale account/sequence conflict, or failed commit must never call adapter execution.

### 4. Add the durable controller account contract

In `milkdrift-persistence`, add one cohesive controller-accounting module containing only the cross-owner durable contract:

- stable account and reservation identities;
- immutable policy-derived budget and exact currency;
- immutable run binding;
- settled and outstanding resource values;
- account revision/integrity state;
- a small closed validated transition set;
- typed admission/settlement failures;
- narrow read/commit data needed by runtime and artifact publication.

Do not create a new crate. Do not expose arbitrary deltas, database rows, or redb types.

Extend `RuntimeStore` through a narrow persistence port. Extend `AtomicRunCommitRequest` and artifact publication contracts with the smallest accounting guard/transition shape that permits one redb transaction to own each durable change. Update every constructor; do not retain a default that accidentally bypasses controller accounting for controlled runs.

### 5. Implement exact identity and inheritance

Have `milkdrift-control` derive the account declaration from the validated controller policy and exact controller occurrence. Return it through the existing controller lifecycle boundary rather than reparsing policy in runtime or persistence.

Persist one immutable optional run-to-account binding. Establish/bind the controller account at activation and propagate it at every child-run creation path, including repeat bodies, nested subworkflows, retries, and detached child ownership supported by the current runtime. Replayed creation must prove the same binding.

Reject ambiguous nested controller ownership and legacy marked histories without account facts. Do not use actor, proposer, workflow, current revision, or active-controller heuristics.

### 6. Make final adapter entry atomic with admission

Refactor `RuntimeService::execute_invocation_effect` and its planning/commit support so the final exact-generation path is:

1. reload and validate the exact active attempt, lease, resolution, request, and authority basis;
2. evaluate the fresh final authority decision;
3. prepare the exact generation and obtain its enforceable envelope;
4. resolve the immutable account binding;
5. derive the category charge and candidate reservation;
6. read the exact account revision and plan allowed or denied admission;
7. atomically commit the current final adapter-entry event, account transition/guard, terminal rejection when denied, and all existing command/index facts;
8. only after a newly accepted commit, consume the prepared handle and enter adapter execution.

Evolve the current event schema so `CapabilityAdapterEntryDecisionRecorded` remains the sole canonical final gate and records the controller-admission outcome. Add exact schema-version validation and golden fixtures. Do not append one authority event and a separate competing resource-decision path.

For an ordinary unbound run, record `not controlled` and preserve current behavior. For a controlled run, reject `Unknown`, currency mismatch, blocked state, overflow, or a candidate beyond any limit. Use fixed bounded retries for sequence/account conflicts.

### 7. Settle usage, uncertainty, retry, and cancellation

Integrate the account transition into the same journal transaction as the exact terminal, late-terminal, cancellation, or uncertainty fact whenever that fact changes accounting.

Implement the contract’s conservative semantics:

- process/model category count charged at accepted final admission;
- bounded usage/cost/artifact maxima reserved;
- authoritative terminal values settle actual use and release only proven remainder;
- absent bounded usage leaves an outstanding/unknown obligation;
- over-envelope observations persist a blocking contract violation;
- uncertain effects keep reservations;
- retries obtain separate reservations while earlier uncertainty remains;
- late evidence settles the exact original reservation once without rewriting uncertainty;
- cancellation does not imply missing remote usage is zero.

Make transition identity exact and replay-safe. Verify report sequence replay, command replay, terminal duplication, and crash/recovery cannot double-settle.

### 8. Charge controller artifact bytes at the artifact owner

Integrate controller accounting into the existing redb artifact publication transaction and `StoreInvocationDataAccess` path.

Invocation publications must reference the committed reservation carried by `AdapterExecutionContext`. Runtime/context artifacts in a controller-bound run must receive deterministic direct charges. First logical metadata commit settles/charges exact logical bytes; replay and aborted temporary streams do not. Digest deduplication is still a logical charge. Reject over-reservation/over-account bytes before metadata/accounting commit.

Update every `BeginArtifactPublication::new` producer and all direct artifact-store test fixtures. Preserve workspace/global artifact accounting as an independent invariant. Remove any later controller artifact charge derived from terminal or subworkflow aggregation.

### 9. Make lifecycle a consumer, not a second accountant

Change `ControllerLifecycleOwner` so external resource progress comes from the durable account view supplied by runtime. Remove projection-derived controller resource totals and static body-shape process/model admission as hard enforcement.

Retain lifecycle ownership of cycles, revisions, proposal limits, elapsed time, failures, rejections, depth, and human checkpoints. Keep generic run/subworkflow usage only for its remaining observational/runtime consumers and correct any misleading names/comments uncovered by the change.

Update control read models and tests only as required to report the exact committed account view. Do not create a second public controller resource DTO when the existing progress shape can remain truthful.

### 10. Redb schema, integrity, and recovery

Implement exact-current account, reservation, and run-binding records/tables in redb. Apply journal, artifact, and accounting changes atomically. Extend full-store/startup integrity and corruption tests to verify every cross-reference and total.

Review and update:

- physical storage schema;
- internal document format;
- run-event current schema and golden fixtures;
- projection snapshot schema only if its durable payload changes;
- repository contract/version statements.

Follow the current no-unreviewed-migration policy. Earlier physical roots remain explicitly unsupported. Existing v1/v2 run events retain exact reader behavior; missing controller-account evidence fails closed rather than becoming zero.

### 11. Tests required in this pass

Add independent tests for every item in section 12 of the shared contract. At minimum include:

- reusable envelope and prepared-entry conformance;
- exact-generation/permit drop and no-execute-on-denial call counters;
- redb concurrent exact-bound admission from multiple runtime/store handles;
- exact-limit and +1 cases for every resource dimension;
- account arithmetic overflow/currency/immutable-binding rejection;
- uncertainty plus retry; cancellation before and after final entry; missing terminal usage; late evidence; over-envelope violation;
- artifact exact boundary, multiple publications, direct context artifact, replay, digest deduplication, abort, write/commit fault boundaries, and reopen;
- nested/repeat/detached inheritance and ambiguous nested-controller refusal;
- current-schema round trips, exact legacy readers, unsupported versions, checksum-correct corrupt rows, restart, and snapshot/compaction behavior;
- ordinary non-controller regression tests;
- controller progress proving it uses account committed totals rather than child projection totals.

Use behavior-focused test modules and existing builders. Do not build one oversized universal fake or expose internals solely for tests.

Add or update an ignored release-mode controller-admission longevity test if the existing controller longevity lane does not exercise account turnover, reservations, artifact settlement, checkpoints, and restart.

### 12. Documentation and production state

Update only canonical current documents whose facts changed: architecture ownership/path, product status, roadmap qualification state, daemon operation notes, verification evidence, and relevant reference/version text. Keep ADR 0027 as accepted historical rationale. Do not add a pass diary, implementation report, prompt copy, or duplicate overview.

The production daemon must still deliberately leave `ControllerLifecycleOwner` uninstalled after this pass. Status must distinguish “boundary implemented and locally tested” from “production activation qualified.” No CLI/configuration bypass may be added.

## Verification

Run focused package/test suites while iterating, then run all of the following before completion:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps
cargo deny check
cargo machete
cargo tree --workspace --duplicates
cargo test --workspace --all-features -- --list
cargo test -p milkdrift-evidence --test repository_contracts --all-features
```

Run the changed semantic mutation shards at minimum:

```sh
cargo mutation-evidence controller
cargo mutation-evidence runtime
cargo mutation-evidence uncertainty
cargo mutation-evidence retention
```

Fix missing assertions rather than casually classifying survivors. Record a classification only when it meets the repository’s exact accepted policy.

Run the existing controller longevity lane and the new admission longevity lane in release mode with `--ignored --exact --nocapture`. Run the hermetic external-evidence fixture to detect integration regressions; it remains non-qualifying.

## Completion report

Return a concise report containing:

- the final ownership and execution path;
- schemas/versions changed and compatibility behavior;
- obsolete paths removed;
- hostile tests and mutation results;
- complete command results, including any failure;
- explicit confirmation that production controller activation remains closed pending independent qualification;
- any remaining limitation stated precisely, without proposing a weaker fallback.
