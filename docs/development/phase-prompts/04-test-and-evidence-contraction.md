# Pass 4 — Contract tests and evidence without losing proof

Reduce the test and evidence implementation until it independently proves the product without containing alternate versions of the product. Keep high-value hostile, fault, restart, mutation, longevity, and black-box coverage.

Follow `00-pristine-readiness-contract.md` in full. This is not permission to delete difficult tests, weaken assertions, increase ignored coverage, or replace evidence with mocks.

## Primary outcome

The repository should have:

```text
small package-local test support
  + reusable contract suites for open interfaces
  + scenario-specific fakes near their scenarios
  + actual-binary application evidence
  + no duplicate scheduler/store/adapter implementations
```

Test and evidence code in the supplied checkout is large enough that its own architecture must earn its cost.

## 1. Inventory proof by invariant

Build an external temporary inventory mapping every test and evidence scenario to the invariant it proves. Classify code as:

- semantic contract/golden/hostile-reader proof;
- state-machine/property/model proof;
- adapter conformance;
- transaction fault/reopen proof;
- daemon/CLI black-box proof;
- mutation support;
- benchmark/operational observation;
- longevity/stress proof;
- repeated fixture/fake/helper mechanics;
- obsolete, redundant, or implementation-restating coverage.

Do not commit the inventory. Use it to merge overlapping proof and delete repeated mechanics while preserving distinct fault boundaries.

## 2. Consolidate repeated fakes locally

Search for duplicate implementations of terminal adapters/reporters, deterministic clocks, fail-once injectors, HTTP request readers, temporary daemon configuration, profile builders, redb setup, event builders, polling loops, and process/model fixtures.

Known examples include repeated terminal adapter/reporting blocks across capability-host, control, and peer tests; repeated `FailOnce`-style injectors; and separate HTTP request readers in model mock endpoints and local-model evidence.

Consolidate according to ownership:

- interface conformance support belongs with the package that owns the interface;
- adapter-specific fixture construction belongs with that adapter’s tests;
- daemon process/CLI invocation support belongs in one development-only black-box harness;
- redb fault helpers belong with redb tests;
- scenario-specific behavior remains local rather than growing a universal fake backend.

Do not add a published test-utils crate or expose production internals merely to share tests. A small non-default package test-support module is acceptable only when several external implementations must run one owner-defined conformance suite.

## 3. Remove mirrored algorithms from tests

Review tests that reconstruct:

- event projection or compaction;
- controller accounting;
- authority containment;
- capability selection;
- reconciliation classification;
- context ordering;
- journal/index state;
- protocol canonicalization.

Expected values should come from independently stated fixtures, small hand-reviewed cases, model/state-machine invariants, or externally observable outcomes—not a second implementation of the same formula. Delete or replace helpers that can agree with production for the same defect.

Keep mutation testing focused on semantic branches; improve assertions for real survivors rather than maintaining large expected-state builders.

## 4. Separate embedded tests from production owners

Move large `#[cfg(test)]` tails out of production files when they materially obscure the implementation. Use named private test modules or integration tests grouped by behavior. Keep private unit tests near genuinely private mechanics when that locality is valuable.

Specifically review large embedded test sections in controller-account, adapter, protocol, and daemon files. Do not make internals public to move a test. Prefer testing through existing private child modules or public behavior.

## 5. Make evidence tooling a true development-only leaf

Review all evidence binaries and their dependencies, especially:

- `apps/daemon/src/bin/milkdrift-external-evidence/`;
- `tools/evidence/src/bin/local-model-evidence.rs`;
- headless CLI evidence;
- operational evidence;
- duplicated process/model/daemon composition code.

The external-evidence executable is development qualification tooling, not the daemon product binary. Move it into `tools/evidence` or another existing development-only leaf while preserving the documented Cargo alias and exact behavior, unless source analysis proves it is an intentionally shipped application. Do not create a new package for it.

Evidence tooling should prefer launching the actual `milkdrift-daemon` and `milkdrift` binaries when proving application composition. It may use direct library calls only for narrow benchmark or fault-injection paths that cannot be observed through the external protocol. Do not duplicate daemon configuration compilation, runtime composition, or protocol command handling in evidence code.

Share one bounded process owner for:

- selecting free loopback endpoints;
- private token/config creation;
- daemon start/readiness/stop/restart;
- CLI invocation and stable JSON decoding;
- hard deadlines and child cleanup;
- redacted diagnostics.

This owner stays development-only and never becomes product semantics.

## 6. Contract integration-test layout

Review every integration test file above 1,000 lines. Split by behavior families with small explicit support modules. Delete forwarding support and generic fixtures that require understanding most of the product to write one test.

At minimum address the largest controller-account, structured-runtime, capability-host registry, model endpoint, process execution, control service, peer storage, and redb journal test files. Preserve useful scenario names and exact command filters used by CI/workflow docs.

Where several files prove the same normal success path, retain one canonical scenario and use the remaining tests for independent failure boundaries rather than repeating setup and assertions.

## 7. Preserve and improve proof quality

The finished suite must still cover:

- strict schemas, canonical bytes, and hostile bounds;
- immutable identity and exact replay/conflict;
- authority and hidden-resource refusal;
- capability adapter conformance;
- final external entry and truthful uncertainty;
- cancellation, retry, late evidence, and recovery;
- projection replay/compaction and structured concurrency;
- artifact transaction/integrity/retention;
- peer durable acceptance/reconnect/archival;
- daemon overload/startup/shutdown;
- actual daemon/CLI workflows;
- mutation, operational, and manual longevity lanes.

Do not convert non-ignored correctness tests into manual evidence merely to reduce normal suite time. Tests may remain manual only for genuine scale or external-resource reasons.

## Required proof

Run:

- the full local gate;
- every reusable adapter/store/client contract suite;
- all seven mutation shards;
- operational and benchmark smoke commands;
- effect-worker shutdown proof;
- all existing release-mode longevity lanes;
- hermetic external-evidence and local-model evidence;
- actual-binary headless CLI evidence.

Measure before/after:

- integration-test and evidence source lines;
- duplicated helper/fake families;
- number and size of >1,000-line test files;
- product public items required only by tests;
- ordinary suite runtime and ignored test count.

## Completion threshold

This pass is complete only when:

- test/evidence source is net smaller;
- repeated fake/helper implementations are materially reduced;
- no product API exists solely for ordinary test convenience;
- evidence tooling is a development-only leaf and uses public binaries for application proof;
- the number of >1,000-line test files is materially lower;
- mutation and hostile coverage is preserved or improved;
- the complete suite remains deterministic, bounded, and cleanly terminating.
