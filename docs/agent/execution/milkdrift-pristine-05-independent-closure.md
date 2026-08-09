# Work package: independent whole-tree closure review

## Mission

Act as an independent senior reviewer after the four owned-area remediation commits. Do not trust their summaries. Review the complete diff from the original Phase 12 closure commit through the current HEAD, reproduce the important invariants, fix residual defects directly, and determine whether the repository is genuinely ready to become the foundation for the workflow/workspace program.

The standard is not “tests are green.” The standard is coherent ownership, correct algorithms, scalable structure, truthful evidence, maintainable policy, and no known defect intentionally deferred inside the remediated foundation.

## Read before editing

Read the repository context, vision, architecture, implementation status, validation model, accepted ADRs, and all five remediation prompt files. Read each preceding commit and its actual diff. Inspect the source rather than relying on completion reports.

Use `181a069ce81525e9c144fe8de051ced8e3c0b9d7` as the historical pre-remediation comparison point unless the user has explicitly rebased history. Do not rewrite or reset preceding commits.

## Review scope

Review the whole tracked repository, with concentrated attention on:

- `candle-backend` artifact inspection, identity, scalar policy, footprint calculation, materialization, and cleanup;
- `hf-hub` declaration and immutable artifact evidence;
- portable load/ownership contracts;
- E0 admission, retained accounting, cleanup fairness, unload, shutdown, and snapshots;
- E1 resolution/load/retained state and persistence;
- Slint boundary;
- architecture policy and package metadata;
- xtask canonical commands;
- benchmark/evidence ownership;
- hosted and self-hosted workflows;
- README, project docs, ADRs, execution context/history, and exact support claims.

Do not implement workflow/workspace features. You may fix any residual issue within the reviewed foundation.

## Required review questions

### Artifact and scalar truth

Verify from code and tests that:

- complete observed artifact dtypes and required execution dtypes are distinct;
- only required tensors choose primary/execution precision;
- unused tensors cannot force downcast, rejection, device allocation, or transfer;
- required F16/BF16 mixtures remain explicitly unsupported unless intentionally changed with evidence;
- absent, supported, unsupported, malformed, and contradictory configuration declarations remain distinct;
- no vendor fallback hides a present unsupported modern declaration;
- all selected shards are structurally bounded and duplicate-safe;
- verified immutable and mutable-source paths preserve source identity without unnecessary repeated full-model work;
- mutation cannot publish a model different from the accepted preparation.

### Algorithm and resource planning

Verify that the implementation's documented formulas match actual allocation order for CPU and CUDA:

- required retained tensors;
- staging buffers;
- cast tensors;
- transfer tensors;
- temporary model-construction maps;
- synchronization and release;
- cache bytes per token;
- metadata/inspection limits.

Look for double counting, missing simultaneously live allocations, conservative values mislabeled as exact, and capacity charged for ignored tensors.

### Ownership and failure truth

Verify every load path:

- plan rejection before materialization;
- failed partial load with immediate cleanup success;
- failed partial load with retry and exhaustion;
- complete-model contract mismatch with cleanup success;
- complete-model mismatch with cleanup failure and unverified ownership;
- successful commit from peak to final;
- ordinary unload;
- sequence cleanup failure;
- shutdown and process-lifetime retention.

Check that no sole owner is lost, no identity is reused early, no reservation is released twice, and no unknown ownership is displayed as an exact byte count.

### Layering and public API

Verify:

- portable crates contain no filesystem/vendor/UI assumptions;
- E0 remains backend-neutral in production;
- E1 does not duplicate Candle scalar/layout policy;
- persistence contains only durable selection/catalogue facts;
- Slint is a projection only;
- benchmark code remains an outer observer;
- architecture enforcement is declarative, strict, and extensible for future workflow/workspace roles;
- public engine APIs are understandable without reading the desktop app.

### Maintainability

Search for:

- oversized modules that still combine distinct responsibilities;
- duplicate scalar, footprint, identity, error, or state conversion logic;
- broad lint suppressions;
- TODO/FIXME/temporary branches;
- `unwrap`, panic, unchecked indexing, or overflow assumptions contrary to policy;
- stale `llm-app` names outside intentional history/migration;
- brittle test-name shell parsing;
- exact dependency registries that still mirror Cargo unnecessarily;
- tests that assert implementation trivia rather than invariant behavior;
- dead report/schema code and documentation duplication.

Refactor residual problems rather than only listing them when they are inside the current foundation and can reasonably affect the next iterations.

### Evidence truth

Verify current documentation against actual code and runs:

- local CPU validation;
- portable target validation;
- local CUDA validation;
- GitHub CUDA run `31281013243` on the original Phase 12 closure;
- GitHub Quality disk failure `31281013257` and its real cause;
- any later remote runs created after remediation;
- external mixed-checkpoint evidence or its absence;
- historical schemas and measurements.

Do not infer generic NVIDIA, performance, leak freedom, or external-model compatibility from fixture tests.

## Validation execution

Run from clean isolated targets and retain command logs outside tracked source:

1. `cargo xtask architecture`
2. `cargo xtask hygiene`
3. the final canonical `cargo xtask verify`
4. exact maintained benchmark compilation
5. WASM portable checks
6. embedded portable checks
7. cargo-deny locked policy
8. offline local Markdown links
9. formatting and `git diff --check`
10. default CPU adapter/E0/E1 focused matrices
11. CUDA feature-graph check and Clippy
12. the full dedicated local CUDA hardware suites on available hardware
13. repeated load/unload and failed-cleanup tests where needed to prove no stale ownership

Record peak disk use for the clean hosted-equivalent native gate and prove the portable checks do not reuse or duplicate its target.

If a command cannot run because of an actual environment limitation, report it precisely and do not substitute a weaker claim.

## Closure criteria

The repository is ready only when all are true:

- no known correctness issue remains in declared/observed/required/execution scalar handling;
- no unused tensor is materialized or charged to device execution;
- source identity is secure and scalable for both trusted immutable and mutable inputs;
- E0 never reports uncertain retained ownership as exact;
- cleanup ownership remains reachable through success, retry, exhaustion, and shutdown;
- E1 and Slint contain no adapter conversion policy;
- architecture policy can accept future declared workflow/workspace packages without a giant name registry while still failing closed;
- the canonical gate fits a standard hosted runner with deliberate target cleanup;
- portable checks are isolated and pass;
- maintained benchmark targets are exact;
- CUDA hardware suites run as suites rather than shell-maintained test names;
- current docs match current code and evidence;
- the working tree is clean and free of generated artifacts.

Do not approve closure with a list of “later” fixes for defects inside this boundary. A genuinely new product capability may remain future work; a known flaw in the existing local foundation may not.

## Completion

Fix every issue discovered that belongs to this foundation. If changes were required, create one coherent final closure commit and do not push. If no tracked changes were required, do not create an empty commit.

Report:

- final HEAD and tree SHA;
- comparison base reviewed;
- findings, grouped by severity, including those fixed during review;
- final architecture/algorithm verdict;
- exact validation commands and results;
- local CUDA matrix and environment;
- peak disk observations;
- remaining evidence gaps that are genuinely external rather than implementation debt;
- a clear greenlight or refusal to greenlight the workflow/workspace program.
