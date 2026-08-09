# Work package: pristine workspace architecture, verification infrastructure, evidence, and project truth

## Mission

Make the repository's enforcement and evidence infrastructure scale with the project instead of mirroring today's incidental graph. Eliminate the CI disk-exhaustion design, compile only real benchmark targets, replace brittle hardware-test name registries, modernize pinned Actions, simplify architecture policy into durable invariants, and reconcile every current support/documentation claim with the corrected implementation and actual GitHub/local evidence.

This is not a cosmetic CI patch. The repository must emerge with one efficient, reproducible, fail-closed validation model that future workflow/workspace crates can extend without rewriting a hardcoded constitution.

## Read before editing

Read:

- `README.md` and `docs/vision.md`;
- the documentation ownership model and agent context;
- `docs/project/architecture.md`;
- `docs/project/workspace.md`;
- `docs/project/dependency-policy.md`;
- `docs/project/validation.md`;
- `docs/project/performance.md`;
- `docs/project/implementation-status.md`;
- `docs/agent/execution/current.md`, `execution-plan.md`, and `history.md`;
- all accepted ADRs affected by the preceding commits;
- `tools/xtask` policy/hygiene implementation and tests;
- `.github/workflows/quality.yml` and `cuda-hardware.yml`;
- all manifests and benchmark/evidence code;
- the preceding three commits and their reports.

Use the current tree as source of truth. Historical reports are evidence for their original commit only.

## Known remote evidence to reconcile

For Phase 12 closure commit `181a069ce81525e9c144fe8de051ced8e3c0b9d7`:

- GitHub Actions CUDA hardware run `31281013243` completed successfully on the self-hosted RTX 5070 Ti job.
- GitHub Actions quality run `31281013257` failed after the canonical `cargo xtask verify` work had succeeded. The same job then compiled the entire workspace bench profile, left approximately 49 MB free, started a second default `target/` for the WASM check, and failed with `No space left on device`; the following lld bus error was consequential, not a Rust/WASM product failure.

Do not preserve documentation that says the Phase 12 GitHub CUDA workflow never ran. Do not rewrite historical text as though the run had already happened when it had not.

## Owned area

Primary ownership:

- root `Cargo.toml`, package metadata, and feature topology;
- `tools/xtask/**`;
- `.github/workflows/**`;
- `benchmarks/runtime/**` and actual Criterion bench targets;
- architecture/dependency/hygiene tests;
- canonical project/agent documentation, ADR reconciliation, README support summary, and evidence history.

Make code changes outside this area only to correct integration failures found while validating the preceding work. Do not begin workflow/workspace feature implementation.

## Required architectural outcomes

### 1. Replace the exact-edge constitution with scalable layer enforcement

The current architecture validator hardcodes package names, exact ordinary edges, a complete role matrix, and special cases for the sole benchmark/tool packages. This is strict but will make every legitimate future workflow crate require editing a large Rust registry before Cargo can express a normal legal dependency.

Redesign enforcement around durable repository semantics:

- each workspace package has an explicit project-owned role/layer classification in manifest metadata or another single declarative source;
- unknown or missing roles fail closed;
- the validator enforces the permitted layer DAG and forbidden dependency kinds generically;
- portable F0/F1 packages cannot depend on runtimes, adapters, apps, native infrastructure, or unsupported external facilities;
- adapters cannot depend upward on runtimes/apps;
- E0/capability/E1/application directions remain explicit;
- benchmark/evidence packages are outer observers and cannot be depended upon by product code;
- tooling remains isolated;
- default features and reviewed CUDA forwarding remain fail-closed;
- exceptional edges require an explicit, small exception record with rationale;
- ordinary legal edges do not each require a second hand-maintained copy of Cargo metadata.

Preserve strict tests for:

- unknown roles/locations;
- illegal upward edges;
- cycles where the layer permits peers but the domain DAG forbids them;
- production-to-benchmark/tool edges;
- unreviewed feature aliases/forwarding;
- default CUDA reachability;
- build/dev dependency distinctions;
- exceptional edge validation.

The validator should become smaller, clearer, and easier to extend while remaining stricter about actual invariants. Do not replace hardcoded names with fragile path-prefix inference. Do not make all runtime-to-runtime or domain-peer edges universally legal without a defined rule.

Document how a future workflow-runtime, workspace, plugin SDK, headless app, or provider adapter would declare its role without modifying a giant match statement.

### 2. Redesign the canonical verification command for exact work

`cargo xtask verify` currently ends with:

```text
cargo bench --workspace --no-run
```

which builds release bench harnesses for every workspace package, including packages with no meaningful benchmark. This consumed most of the hosted runner disk and time.

Inventory actual benchmark targets and replace the workspace-wide bench build with exact compilation of only maintained benchmark targets. At minimum, account for the real `runtime-benchmarks` and sampling Criterion targets if both remain maintained.

The canonical gate should still cover:

- architecture and hygiene policy;
- formatting;
- workspace checks for intended targets;
- tests;
- strict Clippy;
- rustdoc with warnings denied;
- exact maintained benchmark compilation.

Requirements:

- no accidental omission of a maintained benchmark target;
- no compilation of every ordinary library as a release bench harness;
- no hidden network/model download;
- no Python/CMake/tool fallback beyond the explicitly accepted native dependency path;
- clear command failure reporting;
- direct Cargo commands remain visible and reproducible.

Add tests or metadata validation that ensure maintained benchmark targets are registered declaratively and unknown benchmark packages/targets fail appropriately.

### 3. Give portability checks isolated jobs and targets

Move WASM and embedded `no_std` checks out of the heavyweight native quality job. They should run in separate small jobs, preferably from a matrix, with:

- only the portable packages;
- their own isolated `CARGO_TARGET_DIR`;
- no Slint/native system dependencies;
- no dependency on the native gate's target artifacts;
- exact target installation/toolchain setup;
- clear per-target failure reporting.

Run them in parallel with native quality rather than after a large release bench build. A portability failure must mean a portability compilation failure, not exhaustion caused by unrelated native artifacts.

### 4. Make target lifecycle and disk use explicit

For every hosted and self-hosted workflow:

- use clearly named Milkdrift target directories rather than `llm-app-*` remnants;
- remove isolated targets with `if: always()` cleanup or an equivalent reliable mechanism;
- print disk usage before and after heavyweight steps when it aids diagnosis;
- fail early with a clear message if free space is below the documented minimum;
- avoid creating both an isolated target and root `./target` in the same job accidentally;
- avoid preserving giant release artifacts between unrelated jobs;
- do not add broad caches that risk stale feature graphs or consume more storage than they save.

Measure the clean hosted build's peak disk usage after the redesign and record the observation as CI infrastructure evidence, not product performance.

### 5. Replace brittle CUDA test-name enumeration

The CUDA workflow currently shells out to `--list`, greps exact test names, asserts an exact ignored-test count, and runs individual tests by name. This prevents silent zero-match, but it also makes renaming a test or adding another valid hardware test a workflow rewrite.

Create explicit hardware test targets or another stable test-suite boundary so the workflow can run the entire reviewed hardware suite without maintaining a shell registry of function names.

Required behavior:

- a missing test target fails;
- zero executed hardware tests fails;
- all tests in the dedicated target execute automatically;
- adding a new test to that target causes it to run without workflow edits;
- CPU-in-CUDA, mixed/homogeneous adapter behavior, E0 lifecycle/accounting, failure cleanup, no-fallback, and E1 lifecycle remain covered;
- tests remain download-free and deterministic;
- hardware identity/preflight remains exact and fails closed;
- CUDA does not become a default feature;
- the workflow still cleans its isolated target.

Prefer suite structure over shell parsing. Do not hide hardware tests behind environment-dependent early returns that report success without execution.

### 6. Remove deprecated GitHub Actions runtime warnings

The pinned checkout action used by the current workflows targets a deprecated Node runtime and is being forced to a newer runtime by GitHub.

Update first-party Actions to current official releases after verifying them against official sources. Continue pinning full immutable commit SHAs and annotate the release version in comments. Do not switch to floating tags.

Review all workflow permissions and keep them least-privileged.

### 7. Separate correctness, benchmark, and evidence responsibilities

Audit `runtime-benchmarks` after the Phase 12 schema expansion.

Keep:

- deterministic correctness tests where they validate the observer/report contract;
- real benchmark targets that answer maintained performance questions;
- explicit external evidence tooling that records provenance and limitations.

Remove or consolidate:

- production-like orchestration duplicated only for report generation;
- schema fields with no accepted measurement or decision consumer;
- duplicate scalar/device/footprint conversion logic that belongs in shared public observation vocabulary;
- legacy parser scaffolding for formats the project never reads;
- benchmark helpers exposed through production APIs solely for measurement;
- historical values rewritten into newer schemas.

A schema version change is not performance evidence. Exact historical reports retain their original schema and commit attribution.

Add a concise measurement registry documenting:

- what each benchmark/evidence runner measures;
- which public boundary it observes;
- whether it is correctness, synthetic performance, external product evidence, process sampling, or whole-device sampling;
- its required environment and artifacts;
- its output schema owner.

The benchmark package must remain an inward-only observer and must not force production API expansion without a real product consumer.

### 8. Reconcile support and evidence truth

After all implementation and infrastructure changes, update canonical owners so a contributor can tell exactly what is implemented and proven.

At minimum reconcile:

- `README.md`;
- `docs/project/architecture.md`;
- `docs/project/candle-backend.md`;
- `docs/project/inference-runtime.md`;
- `docs/project/application-runtime.md`;
- `docs/project/desktop-runtime.md`;
- `docs/project/implementation-status.md`;
- `docs/project/validation.md`;
- `docs/project/performance.md`;
- `docs/project/dependency-policy.md` and workspace documentation;
- ADR-0019 and ADR-0020 if superseded/extended;
- `docs/agent/execution/current.md`, `execution-plan.md`, and `history.md`;
- benchmark README/provenance.

Current-state documents must record:

- the corrected required-versus-observed scalar policy;
- unused-tensor behavior;
- source-identity paths;
- final/peak and retained-ownership semantics;
- strict declaration behavior;
- exact CPU and CUDA support boundary;
- the successful Phase 12 GitHub CUDA run;
- the hosted Quality disk failure as CI infrastructure history, not a product failure;
- the new successful Quality/portable runs once executed;
- remaining external-checkpoint evidence gaps without overstating them;
- that the next product program is not yet ratified beyond workflow/workspace direction.

Historical documents may append later facts but must not falsify chronology.

### 9. Remove stale project identity and dead infrastructure

Search the entire tracked tree for:

- `llm-app` names that are no longer intentional historical text;
- obsolete phase support claims;
- superseded mixed-dtype limitations;
- paths/commands that no longer exist;
- temporary schema language;
- broad lint exceptions;
- TODO/FIXME/dead compatibility code;
- duplicated documentation authority;
- untracked/generated artifacts accidentally referenced as canonical.

Migrate runtime data paths only if live user data could be affected; do not silently strand existing state. Historical names inside genuinely historical records may remain with context.

## Validation

Run validation from clean isolated targets after the redesign:

- architecture and hygiene tests;
- canonical `cargo xtask verify`;
- exact benchmark compilation;
- WASM portable matrix;
- embedded portable matrix;
- cargo-deny with the locked policy;
- offline local Markdown link validation;
- formatting and `git diff --check`;
- CUDA compile graph;
- local exact CUDA hardware suite on the available RTX 5070 Ti;
- any new CI workflow syntax/static validation available locally.

Capture target disk usage for the clean native gate. Ensure no second root target is created unexpectedly.

Do not claim remote Actions success until the commit is pushed and the run exists. In the local completion report, distinguish local validation from remote evidence.

## Completion

Create one coherent commit and do not push. Report:

- commit SHA and tree SHA;
- final role/layer enforcement model;
- final canonical gate and exact benchmark inventory;
- final CI job structure and target cleanup behavior;
- final CUDA test-suite boundary;
- local validation and disk observations;
- documentation/evidence changes;
- remote runs that must occur after push.
