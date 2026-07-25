# Repository Architecture and Project-Structure Analysis

**Repository reviewed:** `llm-app copy`  
**Review type:** static architecture, workspace, API, test, benchmark, and repository-hygiene review  
**Date:** 2026-07-22

## Scope and limitations

This report reviews the complete uploaded snapshot: the root package, all workspace manifests, approximately 19,300 lines of Rust, project documentation, tests, and the Criterion benchmark.

I could not run `cargo check`, `cargo test`, Clippy, or the benchmark because the analysis environment does not have a Rust toolchain installed. Findings about compilation and runtime behavior are therefore based on source inspection. The repository contains no earlier commit or baseline snapshot, so I cannot prove that a problem is a historical regression; I can identify current architectural degradation relative to the repository's own stated design.

## Executive verdict

The repository is **not badly designed**. Its lowest and most performance-sensitive layers are unusually disciplined:

- model ownership is exclusive rather than `Arc<Model>`-based;
- backend calls use static dispatch in the hot path;
- the portable feature crates are `no_std`;
- caller-owned buffers and explicit capacity failures are common;
- channels are bounded;
- model and request lifecycle state is explicit;
- drain timeouts escalate to cancellation;
- unsafe code is quarantined;
- workspace dependency versions and lints are centralized;
- the Slint frontend is thin.

The project is, however, beginning to optimize and formalize its architecture **ahead of validating the product's central vertical slice**. The most important fact in the repository is still this statement in `README.md:25-27`:

> Chat generation remains deliberately absent until context planning, prompt rendering, sampling, streaming decode, and generation scheduling are connected as one tested loop.

That means the project has two model backends, an architecture validator, allocation gates, a statistical sampler benchmark, a persistence adapter, a corrective workflow engine, and roughly 19,000 lines of Rust—but still does not run the core prompt → tokenize → prefill → sample → decode → stream loop.

That is the primary strategic risk. The individual abstractions are often good, but they have not yet been forced to work together under real product pressure. Until that happens, some boundaries may be protecting assumptions rather than protecting proven design.

## Priority summary

### P0 — address before expanding the architecture

1. Implement one complete chat-generation vertical slice.
2. Make inference load/start operations transactional and rollback-safe.
3. Put the architecture validator and ordinary Rust checks in mandatory CI.

### P1 — address while integrating generation

4. Preserve `application-runtime` as the frontend façade, but separate façade concerns from concrete native composition and unrelated corrective workflows.
5. Reconsider the absolute F1-to-F1 dependency ban before `domain-contracts` becomes a junk drawer.
6. Convert the root maintenance binary into a conventional `xtask` and stop wrapping Cargo commands that add no behavior.

### P2 — repository quality and maintainability

7. Correct stale documentation, the accidental README heading, and misleading status claims.
8. Split oversized source files into internal modules before adding more crates.
9. Add license files, dependency/advisory checks, explicit `no_std` target checks, and a duplicate-dependency audit.
10. Rewrite the internal Rust “knowledge” document where it states performance folklore as universal fact.

---

# 1. What the repository gets right

## 1.1 The core inference ownership model is strong

The repository has not regressed into shared model ownership. `domain-contracts/src/backend.rs` explicitly describes exclusive mutable access, and `InferenceRuntime` owns model slots and sequences directly. Backend operations execute through `&mut self`, which keeps model state and native sequence state on one owner thread.

This is a good fit for native inference libraries, deterministic release, and backends whose model or sequence values are not `Send` after construction.

The `Arc<LlamaBackend>` in `gguf-backend/src/loader.rs` is not an `Arc<Model>` regression. It wraps process-level/native backend initialization shared by the loader and model construction; the loaded model and sequences remain runtime-owned.

## 1.2 Hot-path dispatch is appropriately static

The backend contracts use associated types and generics rather than `dyn Trait` in token-level operations. That permits monomorphization and inlining where it matters without forcing every cold application-level dependency to follow the same rule.

This distinction should be preserved: static dispatch is valuable in tensor, token, and tight buffer loops; dynamic dispatch or boxed service ports are perfectly reasonable for cold operations such as storage, artifact resolution, application command routing, and backend selection.

## 1.3 Capacity, allocation, and lifecycle are first-class concepts

The repository consistently models bounded resources rather than treating allocation as invisible:

- `CapacityExhausted` is part of the portable error vocabulary.
- Tokenization and sampling receive caller-owned storage.
- Sampling and contract tests contain allocation gates.
- Runtime channels are bounded.
- Model/request memory footprints are admitted before use.
- Lifecycle transitions and drain deadlines are explicit.

This is stronger than most early-stage Rust application code.

## 1.4 Workspace configuration is modern and mostly idiomatic

The workspace uses:

- edition 2024;
- resolver version 3;
- a pinned Rust toolchain;
- `[workspace.dependencies]` for version and path centralization;
- inherited package metadata;
- inherited lints;
- strict checks against `unwrap`, `expect`, `panic!`, and unsafe code.

A static scan found no `unwrap`, `expect`, `panic!`, `todo!`, or `unimplemented!` calls in production Rust source. That does not prove correctness, but it shows the stated error-handling policy is actually reflected in the code.

## 1.5 Tests and benchmarks are colocated correctly

Package-specific integration tests are under each crate's `tests/` directory, and the sampling benchmark is under:

```text
crates/features/sampling/benches/sampling_pipeline.rs
```

That is Cargo's normal package-target layout. A benchmark for a production sampling algorithm belongs with the sampling crate; it does not need to live in a top-level repository benchmark folder.

---

# 2. The largest problem: architecture before vertical integration

The repository's phase ordering has allowed isolated subsystems to mature before the primary application behavior exists.

Currently:

- `sampling` is not a dependency of either runtime.
- `context-planner` is not a dependency of either runtime.
- tokenization is used mainly for tokenizer validation, not a generation loop.
- the application API exposes model resolve/load/unload/poll/shutdown, but no prompt submission or token stream.
- the corrective workflow system is approximately 2,000 production lines inside `application-runtime`, plus a 993-line integration test.

This is not merely “unfinished product work.” It creates an architecture-validation problem. The core loop is where the boundaries between tokenization, context planning, model prefill, logits, sampling, decode state, cancellation, output buffering, and frontend backpressure will be tested. Until those components are connected, the project cannot know whether:

- the current contracts carry enough information;
- ownership boundaries are convenient under a real decode loop;
- error taxonomies compose well;
- caller-owned buffers are sized and reused correctly;
- cancellation points are responsive;
- application events have the right granularity;
- frontend polling is sufficient for token streaming;
- the Candle and GGUF backends behave consistently through the same user flow.

## Recommendation

Freeze new architecture layers and new workflow features temporarily. Build one deliberately narrow path:

```text
prompt
  -> prompt template
  -> tokenize into caller-owned token storage
  -> context budget/selection
  -> start request
  -> prefill
  -> sample
  -> decode one token at a time
  -> incremental text decode
  -> bounded output accumulator
  -> application events/frame pulls
  -> cancellation
  -> sequence destruction
```

Start with one backend and one frontend. Then run the same application-level generation tests against the second backend. This will reveal more useful architectural information than adding another isolated feature crate or microbenchmark.

The next performance metrics should be system-facing:

- time to first token;
- steady-state tokens per second;
- cancellation latency;
- peak and resident memory;
- event/output backpressure behavior;
- model unload latency;
- behavior at context and output capacity boundaries.

The sampling benchmark should remain, but it is a component regression benchmark—not evidence that the application is fast.

---

# 3. `application-runtime`: valid façade, growing composition problem

## 3.1 The original purpose is valid

The stated purpose of `application-runtime` makes sense: Slint, Tauri, a CLI, or another native frontend should not each reimplement repository resolution, persisted settings, tokenizer validation, worker ownership, load/unload state, and event normalization.

A cohesive application service or façade is a normal and useful boundary. The existence of `application-runtime` is not the problem.

The distinction is:

- **frontend-neutral:** mostly achieved;
- **backend-neutral:** not achieved at E1;
- **deployment-neutral:** not achieved and not always possible.

A native Tauri frontend or native Leptos server can reuse this crate. A browser-only Leptos/WASM frontend cannot directly own Candle, llama.cpp, redb, native threads, and filesystem paths. It must talk to a native or remote host over HTTP, WebSocket, IPC, or Tauri commands. The repository's architecture document now acknowledges this correctly at `docs/architecture.md:87-90`.

## 3.2 E1 is concretely locked to the native Candle stack

`ApplicationRuntime` owns concrete implementation types in `crates/engines/application-runtime/src/runtime.rs:31-43`:

```rust
HostedRuntime<CandleLlamaSource>
RedbStorage
Option<HfTokenizer>
Option<ResolvedModelArtifacts>
```

`load_model()` constructs `CandleLlamaSource` directly at `runtime.rs:141-177`, and `support.rs` constructs the Candle loader.

Consequences:

- GGUF exists but cannot be selected through the application façade.
- Replacing redb, Hugging Face resolution, or the tokenizer requires modifying E1.
- Application orchestration is difficult to test with deterministic in-memory storage/resolver/model fakes.
- The façade is reusable only when every frontend wants the exact same native composition.

This is not necessarily wrong for the first native product, but it should be described honestly. E1 currently acts as both:

1. the frontend-facing application service; and
2. the native production composition root.

## 3.3 Do not solve this by genericizing everything

Making `ApplicationRuntime<S, H, T, L, ...>` generic over storage, hub, tokenizer, loader, host runtime, and event transport would spread type parameters across the whole application and make public APIs and compile times worse.

Better options are cold-path abstractions:

### Minimal option

Keep the current crate and introduce coarse ports only where replacement is real:

- `ArtifactResolver`
- `SettingsStore`
- `TokenizerFactory`
- `ModelRuntime` or a closed `NativeBackend` enum

These operations are coarse and cold. `Box<dyn Port + Send>` is acceptable here, or use an enum when the supported backend set is deliberately closed.

### Cleaner future option

Split responsibilities only when a second deployment requires it:

```text
application-api       serializable commands, state snapshots, and events
application-core      frontend-neutral use cases/state machine
native-runtime        Candle/GGUF/HF/redb/host-thread composition
```

Slint, Tauri, CLI, and a Leptos server would reuse `native-runtime`. Browser code would depend only on `application-api` through a transport adapter.

Do not create all three crates immediately unless Tauri/Leptos or remote hosting is actively being implemented. The important near-term correction is to prevent `application-runtime` from absorbing unrelated systems.

## 3.4 It is already becoming a god engine

The corrective workflow implementation accounts for approximately 2,000 of roughly 3,584 production lines in `application-runtime`—about 56% of the crate. It contains artifact storage, diagnostic normalization, validation, retry/admission policy, workflow scheduling, and model task ports, in addition to model acquisition and lifecycle orchestration.

`lib.rs:28-37` also re-exports dozens of workflow types at the crate root while exposing `pub mod workflow`.

The workflow code may be good code. The problem is cohesion: a frontend that only needs local chat model lifecycle must compile and conceptually absorb a large draft/validate/review/revise orchestration domain.

### Recommendation

At minimum:

- stop re-exporting every workflow internal from the `application-runtime` root;
- split `workflow/mod.rs` and `workflow/executor.rs` into smaller internal modules;
- keep a narrow workflow façade;
- move the corrective workflow to its own crate when it gains another consumer, another independent lifecycle, or substantially more functionality.

The goal is not “more crates.” The goal is one reason to change per major module.

## 3.5 The frontend API is not yet a transport API

`ApplicationEvent` and the public state types are ordinary Rust types, not an explicitly versioned/serializable command-event protocol. That is fine for Slint or an in-process Tauri backend. It is not enough by itself for a browser client, a separate worker process, or remote execution.

Do not force serialization into the core runtime prematurely, but when a transport is introduced, define stable DTOs at the application boundary rather than serializing backend/domain internals directly.

## 3.6 Single-model state conflicts with configurable model capacity

`ApplicationRuntimeConfiguration` exposes `maximum_models` (`configuration.rs:123-124`), but the application state contains only:

```rust
loaded: Option<LoadedModel>
```

and runtime commands always use:

```rust
const MODEL_ID: ModelId = ModelId::new(1);
```

E0 supports multiple models; E1 currently supports one resident model. That may be a deliberate desktop policy, but then `maximum_models` is misleading. Either:

- make the application configuration explicitly single-model and set E0's internal limit to one; or
- model a collection of loaded models and selection in E1.

Do not expose generality that the application state cannot represent.

---

# 4. Runtime correctness: missing transactional rollback

The most concrete correctness concern is in `inference-runtime/src/runtime.rs`.

## 4.1 Model-load contract failure bypasses explicit unload

At `runtime.rs:152-155`:

```rust
let model = self.loader.load(source, &configuration)?;
if model.handle() != handle || model.metadata() != &plan.descriptor.metadata {
    return Err(RuntimeError::BackendContractViolation);
}
```

If a backend returns a model that violates the declared plan, the model is dropped without calling `prepare_unload()`.

A Rust `Drop` implementation may release ordinary owned allocations, but the backend contract deliberately exposes explicit unload preparation for native synchronization and cleanup. A contract-violation path should not bypass the cleanup semantics the runtime requires on normal unload.

## 4.2 Sequence creation and request commit are not rollback-safe

At `runtime.rs:237-269`, the method:

1. creates a sequence;
2. validates its identifier;
3. advances lifecycle state;
4. inserts the request into the model slot;
5. changes slot reservation state;
6. inserts global request and sequence indexes;
7. changes global counters.

Several failures after sequence creation return without fully undoing earlier side effects:

- a wrong `sequence.id()` returns without `destroy_sequence()`;
- `lifecycle.start_request()` can fail after native sequence creation;
- duplicate global request/sequence index entries can return after the request is already inserted and reservation/lifecycle state changed;
- the occupied-request rollback calls `finish_request()` but does not explicitly destroy the newly created sequence first.

Some of these states should be impossible if all prechecks and indexes are internally consistent. That is exactly why the error is called `BackendContractViolation` or an invariant failure. Defensive code must still leave ownership and accounting consistent when an impossible condition occurs.

## Recommendation

Implement these operations as prepare/commit transactions:

- reserve no externally visible indexes until every fallible backend and lifecycle operation succeeds;
- use a local rollback guard owning the uncommitted model/sequence;
- call explicit backend cleanup from the guard when commit does not occur;
- update local slot and global indexes in an order with an infallible final commit, or preflight every potentially occupied entry before mutation;
- add fault-injection tests with deliberately nonconforming test backends.

This is more important than further micro-optimization because it protects native resource correctness on rare paths.

---

# 5. Workspace structure: does it make sense?

## 5.1 The current structure is understandable

The main categories communicate intent:

```text
crates/features
crates/adapters
crates/engines
crates/apps
```

Cargo assigns no semantics to those folder names; the manifests define the actual graph. A custom folder taxonomy is acceptable when it is documented and consistently enforced, which this repository attempts to do.

## 5.2 `sampling` belongs in a production logic area

The sampling crate is not “a crate created for benchmarking.” It implements production temperature, top-k, top-p, min-p, repetition penalty, RNG selection, and stop matching. Therefore it belongs with reusable domain/algorithm code.

`crates/features/sampling` is reasonable under the repository's vocabulary. The only naming concern is that “features” has an established Cargo meaning—Cargo feature flags—so new contributors may initially misread the directory.

Possible clearer names include:

```text
crates/domain
crates/core
crates/algorithms
```

A rename is not worth doing merely for convention. The dependency graph and ownership matter more than the directory label.

## 5.3 The “adapters” bucket is becoming semantically broad

The adapters folder currently mixes:

- model execution backends (`candle-backend`, `gguf-backend`);
- persistence (`redb-storage`);
- artifact/network access (`hf-hub`);
- tokenizer integration (`hf-tokenizer`);
- threads, channels, and clocks (`host-runtime`).

All are infrastructure boundaries, so the layering is defensible. As the project grows, however, “adapter” may stop helping contributors find things.

A future organizational cleanup could distinguish:

```text
crates/backends/         candle, gguf
crates/infrastructure/   hf-hub, hf-tokenizer, redb, host-runtime
crates/runtime/          inference-runtime, application-runtime
crates/domain/           contracts and algorithms
crates/workflows/        corrective workflow, if independently retained
apps/                    desktop-slint, future CLI/Tauri/server
```

Do not reorganize folders now unless it accompanies a real ownership change. Moving files without changing dependencies is architectural theater.

## 5.4 Per-package tests are more idiomatic than a magic top-level test folder

The current package `tests/` directories are normal Cargo integration-test targets. Keep them.

When true cross-workspace end-to-end tests arrive, create a dedicated workspace member such as:

```text
tests/e2e-runtime
```

with dependencies on the public crates it exercises. A top-level `tests/` directory is only automatically meaningful for the root package, not as a universal workspace test mechanism.

---

# 6. The F0/F1 dependency policy is too absolute

The repository enforces:

```text
F1 -> F0 allowed
F1 -> F1 forbidden
```

This produces a clean-looking graph, but “no horizontal dependencies” is not a general idiomatic Rust rule. The real requirement is an acyclic graph with clear ownership.

## 6.1 It encourages `domain-contracts` to become a junk drawer

`domain-contracts` owns backend/model/lifecycle/output contracts, but also generic identifiers and capacity vocabulary used by unrelated algorithms:

- `ArtifactId`
- `TaskId`
- `TokenId`
- sampling workspace capacity categories
- model and sequence contracts
- lifecycle and generation types

Because an F1 crate cannot depend on another F1 crate, every type shared by two algorithms is pressured downward into the single F0 crate. Over time, this can create exactly the monolithic “core” crate the architecture claims to avoid.

For example, token identity is conceptually owned by tokenization/model vocabulary, while workflow artifact/task identifiers belong to orchestration. Their coexistence in one universal foundation is a consequence of the policy rather than natural cohesion.

## 6.2 A DAG is enough

Allow legitimate directional dependencies when one domain owns a concept used by another. For example:

```text
context-planner -> tokenization types
sampling -> token vocabulary types
corrective-workflow -> task-graph
```

That does not create harmful coupling if the lower crate owns a stable abstraction and the graph remains acyclic.

## 6.3 Avoid swinging into micro-crates

Do not split every identifier into its own crate. Two reasonable approaches are:

### Conservative

Keep `domain-contracts`, but define a strict inclusion rule: only types that genuinely cross the engine/backend boundary belong there. Move workflow-only IDs and policies closer to the workflow/task-graph domain.

### Moderate

Split it once along a real boundary:

```text
foundation-types       IDs, capacity primitives, time primitives
inference-contracts    model/backend/sequence/generation/lifecycle contracts
```

Then permit a small, documented DAG among algorithm crates.

The repository's numerical “1–3 engine crates per domain” rule is similarly arbitrary. Crate count should follow ownership, reuse, compile boundaries, and change patterns—not a quota.

---

# 7. Benchmarks: what is normal and what should change?

## 7.1 `cargo bench` is the correct entry point

Cargo already provides benchmark targets and the `cargo bench` command. On stable Rust, projects commonly use a custom benchmark harness such as Criterion and set:

```toml
[[bench]]
name = "sampling_pipeline"
harness = false
```

The repository does exactly that. This is normal, modern Rust practice.

## 7.2 The benchmark is located correctly

The benchmark tests one crate's public production behavior, so this location is correct:

```text
crates/features/sampling/benches/sampling_pipeline.rs
```

A top-level benchmark package is useful only for cross-crate or end-to-end scenarios, such as comparing Candle and GGUF prefill/decode through the runtime.

## 7.3 The root `benchmark` command is redundant and overnamed

The root runner executes only:

```text
cargo bench -p sampling --bench sampling_pipeline
```

A wrapper is not wrong, but it adds no behavior. Calling the command `benchmark` suggests a repository-wide performance suite when it runs one component benchmark.

Prefer one of:

```text
cargo bench -p sampling --bench sampling_pipeline
cargo bench --workspace
cargo bench-sampling          # Cargo alias
cargo xtask bench sampling    # only if xtask adds baselines/profiling/metadata
```

## 7.4 The current measurement has narrow coverage

The benchmark:

- uses one vocabulary size: 32,768;
- uses only `SamplingConfig::default()`;
- includes `logits.copy_from_slice()` inside each measured iteration;
- does not exercise repetition processing because the default repetition penalty is `1.0`;
- does not benchmark greedy, min-p, alternative top-k/top-p values, or stop matching;
- does not connect the result to model decode throughput.

Including the copy is acceptable if the benchmark is explicitly “restore logits + sample.” It is misleading if interpreted as sampler-only latency.

Recommended benchmark matrix:

- greedy;
- default top-k/top-p;
- min-p enabled;
- repetition penalty enabled with different history lengths;
- vocabularies around 8k, 32k, and 128k;
- isolated sampler benchmark with untimed/batched setup;
- retained full sampling-pipeline benchmark;
- tokenizer encode/decode;
- context planning;
- backend prefill and decode;
- complete runtime time-to-first-token and token-throughput benchmarks.

Do not use wall-clock benchmark thresholds as hard pass/fail gates on shared CI runners. Compile benchmarks in CI; collect stable performance baselines on controlled hardware.

---

# 8. Root commands: building a runner is normal, but this is an `xtask`

## 8.1 The custom Rust runner is a known pattern

Rust projects commonly create a small workspace utility crate—usually named `xtask`—for repository-specific operations that are too complex for a Cargo alias. Architecture validation is an appropriate xtask responsibility.

The current root package is functionally an xtask but is named `llm-app`, which makes the root binary look like the product application.

Current invocation:

```text
cargo run --bin llm-app -- verify
```

More conventional invocation:

```text
cargo xtask verify
```

## 8.2 Recommended structure

Make the root manifest a virtual workspace and move the runner:

```text
Cargo.toml
.cargo/config.toml
tools/xtask/Cargo.toml
tools/xtask/src/main.rs
```

Example alias:

```toml
[alias]
xtask = "run -p xtask --"
bench-sampling = "bench -p sampling --bench sampling_pipeline"
```

This removes the need to duplicate every workspace member in both `members` and `default-members`, and separates product naming from repository maintenance.

## 8.3 Do not reimplement Cargo

Keep custom Rust code for:

- architecture-policy validation;
- generated-file checks;
- orchestrated release/package operations;
- performance-baseline handling;
- commands needing nontrivial logic.

Use Cargo directly or aliases for:

- `fmt`;
- `check`;
- `test`;
- `clippy`;
- a single benchmark selection.

The “no shell/Python automation” policy is a portability preference, not an architectural virtue by itself. A Rust xtask is good when it reduces platform differences; it is unnecessary ceremony when it only forwards arguments unchanged.

## 8.4 The `test` command selects too much

The runner uses:

```text
cargo test --workspace --all-targets --all-features
```

Cargo's `--all-targets` includes benchmark targets. That means the ordinary verification path selects the Criterion benchmark target even though benchmarks are conceptually separate from correctness tests.

Prefer:

```text
cargo test --workspace --all-features
cargo check --workspace --all-targets --all-features
cargo bench --workspace --no-run
```

As feature flags are introduced, be careful with `--all-features`. It works now because the packages expose no meaningful feature matrix, but mutually exclusive backend/device features often cannot all be enabled together. At that point CI should test an explicit feature matrix instead.

---

# 9. The architecture validator is useful but currently porous

## 9.1 Purpose of the tests in `src/main.rs`

The tests from the previous prompt verify the policy implementation itself:

- manifest paths map to expected layers;
- F1 and adapter dependency directions behave as declared;
- E0, E1, and app dependencies behave as declared.

They answer:

> Does `classify_manifest()` and `allows()` encode the intended table?

They do **not** answer:

> Does the current workspace graph obey that table?

The latter occurs only when the `architecture` or `verify` command invokes `cargo metadata`.

## 9.2 No CI enforcement exists in the snapshot

The snapshot contains no `.github/workflows`, other CI configuration, or equivalent mandatory check. Therefore a contributor can add a forbidden dependency while ordinary workspace unit tests still pass unless they explicitly run the custom verifier.

The architecture command should be a required CI job, not a documented convention.

## 9.3 It checks only workspace-local path dependencies

At `src/main.rs:203-208`, dependencies are filtered to paths under the repository root. This means an F1 crate could add a direct crates.io dependency on `tokio`, `reqwest`, Candle, a database, or another infrastructure library and still pass the layer validator.

The stated “adapter quarantine” is therefore not enforced by the checker.

Add layer-specific external dependency policies, especially for F0/F1:

- F0: no third-party dependencies unless explicitly approved;
- F1: a narrow allowlist of portable/no_std dependencies;
- adapters: vendor and `std` dependencies allowed;
- engines/apps: policy appropriate to their role.

## 9.4 Unknown paths become applications

`classify_manifest()` falls through to `Layer::Application`. A new crate under `tools/`, `tests/`, or a misspelled directory is silently treated as an app.

Classification should return `Result<Layer, UnknownManifestLocation>` and fail closed.

## 9.5 Dependency kind is ignored

The metadata model records only name and path. It does not distinguish normal, development, and build dependencies.

The architecture documentation speaks mainly about production edges, but the implementation applies the same table to every local dependency kind. Decide and encode the policy explicitly. Test-only backend compatibility dependencies may be legitimate even when the equivalent production dependency is forbidden.

## 9.6 The test matrix is partial

The unit tests sample selected edges rather than exhaustively testing the 7×7 layer matrix. A table-driven test should enumerate every source/target combination and assert the expected result.

## 9.7 Use `cargo_metadata`

Cargo's metadata JSON is a supported interface, but manually defining a partial schema creates maintenance work and currently omits useful fields. The `cargo_metadata` crate is the conventional typed interface for Cargo metadata consumers.

## Recommended validator improvements

1. Move it to `xtask`.
2. Use `cargo_metadata`.
3. Fail on unknown workspace paths.
4. Declare the entire layer policy as data and test the full matrix.
5. Examine dependency kind and source.
6. Enforce external dependency allowlists for portable layers.
7. Add explicit `no_std`/target checks.
8. Run it in required CI.
9. Add an integration test that analyzes the actual workspace graph, not only pure helper functions.

---

# 10. Source/module organization

Large files are not automatically bad, but several modules now contain multiple independent responsibilities:

- `task-graph/src/lib.rs`: 1,036 lines;
- `application-runtime/src/workflow/executor.rs`: 922 lines;
- `application-runtime/src/workflow/mod.rs`: 688 lines;
- `inference-runtime/src/runtime.rs`: 931 lines;
- `gguf-backend/src/metadata.rs`: 640 lines.

Do not create crates merely to reduce line counts. Split internal modules first.

Suggested internal splits:

### `task-graph`

```text
lib.rs
identifier.rs
graph.rs
validation.rs
artifact_flow.rs
attempt.rs
state.rs
error.rs
```

### corrective workflow

```text
workflow/mod.rs
workflow/configuration.rs
workflow/plan.rs
workflow/admission.rs
workflow/executor.rs
workflow/artifacts.rs
workflow/diagnostics.rs
workflow/event.rs
workflow/ports.rs
```

### inference runtime

```text
runtime/model_registry.rs
runtime/request_registry.rs
runtime/transaction.rs
runtime/operations.rs
runtime/shutdown.rs
```

This will make invariants easier to review without changing crate boundaries.

One minor visibility issue: helper functions inside the private `support` module are declared `pub`. They are effectively hidden because the module is private, but `pub(crate)` or `pub(super)` would state the intended boundary more clearly.

---

# 11. Shutdown and ownership ergonomics

`ApplicationRuntime` exposes an explicit `shutdown(&mut self)`, which performs bounded cooperative shutdown and joins workers. There is no `Drop` implementation, and `HostThread<T>` does not join on drop. Dropping a `JoinHandle` detaches the thread.

A blocking `Drop` implementation is usually undesirable, especially with native workers and user-configurable deadlines. The current explicit shutdown API can therefore be reasonable. However:

- documentation should state clearly that callers must invoke shutdown;
- frontend integration should guarantee it on normal window/application closure;
- tests should cover dropping or abandoning the application while workers are active;
- consider a best-effort nonblocking disconnect in `Drop`, while keeping bounded joins explicit.

`shutdown.rs` also constructs deadlines with `Instant::now() + timeout`. Extremely large configured durations can overflow on some platforms. Validate an upper bound or use `checked_add` and return a configuration error.

---

# 12. Documentation and repository hygiene

## 12.1 The README begins with an accidental model name

`README.md:1` is:

```text
# HauhauCS/Gemma4-12B-QAT-Uncensored-HauhauCS-Balanced
```

This looks like leaked working state and misrepresents a repository whose current Candle path is specifically described as unquantized Llama Safetensors.

## 12.2 Multiple documentation links are broken

The repository references files such as:

```text
docs/application-runtime.md
docs/inference-runtime.md
docs/desktop-runtime.md
docs/gguf-backend.md
docs/lifecycle.md
docs/candle-backend.md
```

but the files live under `docs/project/`. Broken references appear in the root README, implementation status, and crate READMEs.

Add a link checker to CI.

## 12.3 Status documentation claims validation that is not reproducible here

`docs/project/implementation-status.md:197-201` says the full Phase 8 maintenance sequence is validated. The snapshot contains no CI evidence, and this review environment could not rerun it. A status document should include the commit/toolchain on which validation occurred or point to CI.

## 12.4 License declarations lack license files

The workspace declares `MIT OR Apache-2.0`, but no `LICENSE-MIT` or `LICENSE-APACHE` files are present in the snapshot. Add both.

## 12.5 Supply-chain checks are planned but absent

The implementation plan mentions policy/CI work, but the snapshot has no `deny.toml`. Add a supply-chain check for:

- advisories;
- license policy;
- banned/duplicate dependencies where relevant;
- allowed dependency sources.

The lockfile contains multiple versions of several crates, including `redb` and `tokenizers`. Duplicate transitive versions are not automatically a problem, especially in a GUI/native stack. Run `cargo tree -d` and optimize only duplicates that materially affect binary size, compile time, or incompatible type boundaries.

## 12.6 Portable crates need explicit target checks

`#![no_std]` is good, but host tests alone do not prove useful bare-metal or embedded target compatibility. If portability is a real requirement, CI should check selected targets and avoid claiming generic bare-metal support without named target profiles.

---

# 13. Internal engineering rules that should be corrected

The project's strictness is valuable, but `docs/knowledge/rust_knowledge.md` and `docs/rules.md` contain absolute claims that are technically wrong or likely to drive premature optimization.

## 13.1 `const fn` does not automatically move work into `.rodata`

The document says constructors and derivations should be `const fn` to “offload runtime CPU cycles into static binary `.rodata`.” A `const fn` can be evaluated in a const context; when called at runtime, it behaves like an ordinary function. Mark functions `const` for API capability and compile-time use, not as a universal runtime optimization.

## 13.2 `core::error::Error` exists

The document says no_std code is “without `core::error::Error`.” Modern Rust exposes the `Error` trait in `core`. Allocation-free typed enums are still often the better design, but the rationale should be accurate.

## 13.3 Dynamic dispatch does not universally “destroy branch prediction”

Avoiding dynamic dispatch inside a measured token loop is reasonable. Treating it as forbidden across the architecture is not. Cold/coarse application ports are a good place for trait objects because the I/O, model load, database, or network cost dwarfs one indirect call.

## 13.4 `#[inline]` and `#[cold]` are hints

They do not guarantee layout or inlining, and indiscriminate annotation can make code worse. Use them only after inspection or profiling, especially `#[inline(always)]`.

## 13.5 ECS does not automatically guarantee SoA

The document recommends relying on ECS to manage data contiguity automatically. ECS storage strategies differ, and an ECS is not required to obtain data-oriented layouts. Introducing ECS solely for SoA would be a major complexity error in this project.

## 13.6 The universal 16-byte struct rule is oversimplified

Argument classification depends on target ABI, field classes, alignment, optimization, calling context, and inlining. Do not shrink semantically correct types or choose smaller integers merely to satisfy a universal “16-byte” rule without measuring generated code.

## 13.7 “No mocks” is the wrong rule

`docs/rules.md:9` forbids mock functions. Placeholder production behavior should be forbidden; deterministic test doubles should not be. Application orchestration, failure rollback, timeout behavior, and backend contract violations are precisely where fakes and fault-injection backends are useful.

Rewrite the rule as:

> No placeholder or fake logic in production paths. Tests may use deterministic fakes, stubs, and fault-injection implementations to verify behavior and invariants.

## 13.8 “No prototype phase” creates premature permanence

Every merged change should compile and have honest behavior, but the project needs room for spikes and experimental branches. Otherwise uncertainty gets encoded as elaborate production abstractions before the core product loop proves them.

The project's architecture documents should distinguish:

- hard safety/invariant rules;
- current design decisions recorded as ADRs;
- performance hypotheses requiring measurement;
- temporary implementation constraints.

---

# 14. Modern idiomatic Rust assessment

## Strongly idiomatic

- centralized workspace metadata, dependencies, and lints;
- edition 2024 and resolver 3;
- newtypes for identifiers and validated capacities;
- explicit `Result`-based APIs;
- associated types for backend families;
- package-local integration tests and benchmarks;
- Criterion custom benchmark harness;
- narrow unsafe quarantine;
- `#[must_use]` on important results/accessors;
- `no_std` where the crate is genuinely portable;
- no hidden global model state.

## Reasonable but project-specific

- the features/adapters/engines/apps taxonomy;
- a single E1 façade for native frontends;
- strict allocation-free project-owned portable paths;
- handwritten error enums instead of `thiserror`;
- a Rust maintenance runner instead of scripts.

## Less idiomatic or overly rigid

- naming the maintenance runner `llm-app` rather than `xtask`;
- manually parsing Cargo metadata;
- wrapping ordinary Cargo commands without adding behavior;
- absolute prohibition of all F1-to-F1 dependencies;
- forcing every app dependency through E1, then re-exporting a large API surface;
- broad root re-exports from the corrective workflow;
- `pub` helper functions inside private modules instead of scoped visibility;
- strict `clippy::nursery` combined with `-D warnings`, which can create toolchain-upgrade churn. Pinning the toolchain mitigates this, but review whether every nursery lint should block CI.

---

# 15. A better structure without a rewrite

The repository does not need a mass reorganization. The following is a low-disruption path.

## Step 1: keep current crate folders

Do not rename `features` or `adapters` yet. The names are understandable and documented.

## Step 2: create a virtual root and xtask

```text
Cargo.toml                  virtual workspace
.cargo/config.toml          cargo xtask / bench aliases
tools/xtask                 architecture and composite verification
```

## Step 3: integrate generation before new domains

Connect tokenization, context planning, sampling, inference, streaming decode, and application events.

## Step 4: fix transactional runtime operations

Add rollback guards and fault-injection tests.

## Step 5: narrow E1

Keep the frontend-facing model lifecycle API in `application-runtime`, but:

- introduce backend selection/composition deliberately;
- avoid root-level re-export of all workflow types;
- split corrective workflow internally;
- extract it only when its independent ownership is clear.

## Step 6: relax the feature doctrine

Replace “F1 may never depend on F1” with an explicit approved DAG and a rule that shared foundation types must have at least two genuine cross-domain consumers and stable semantics.

## Step 7: add a real quality gate

Required CI should include:

```text
cargo fmt --all -- --check
cargo xtask architecture
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo doc --workspace --no-deps --locked
cargo bench --workspace --no-run --locked
portable target checks
license/advisory/source policy
markdown link check
```

Use an explicit feature matrix once backend/device features appear.

---

# 16. Blunt answer: what are you fucking up?

1. **You are building validation infrastructure faster than the product's central loop.** The architecture is being optimized before it has been stressed by chat generation.
2. **You are turning architectural preferences into universal laws.** The F1 horizontal ban, engine-count quota, dynamic-dispatch doctrine, and performance folklore are stricter than the evidence supports.
3. **Your one shared F0 crate is at risk of becoming the monolith your rules reject.** The absolute tier policy pushes unrelated shared vocabulary into `domain-contracts`.
4. **`application-runtime` is both façade and native composition root, and it is absorbing a second major domain.** That is the start of a god engine, even though the frontend façade itself is a good idea.
5. **Rare failure paths in the inference runtime are not fully transactional.** Native resources and indexes can be left inconsistent when a backend violates a contract or an “impossible” invariant fails.
6. **Your architecture test tests the rule table more reliably than it tests the repository.** It is not in CI, ignores external dependencies, defaults unknown paths to Application, and ignores dependency kind.
7. **You are wrapping Cargo where Cargo already has the command.** The runner is an xtask in disguise; the benchmark wrapper adds no behavior.
8. **The sampling benchmark is valid but too easy to overinterpret.** It measures one default component path, includes buffer restoration, and does not prove application throughput.
9. **Documentation hygiene is poor relative to the code discipline.** Broken links, a leaked model heading, missing license files, stale plans, and unverifiable “validated” status weaken trust.
10. **Some internal Rust guidance is simply inaccurate.** If agents follow it literally, they will create unnecessary type machinery, annotations, and crate boundaries.

None of these mean the repository should be discarded. The core model/resource architecture is a solid base. The best correction is not a rewrite; it is to build the vertical slice, let real integration expose the necessary boundaries, and reduce rules that exist only to keep a diagram pure.

---

# 17. Recommended execution order

## Immediate

1. Add CI with the current verifier and Rust checks.
2. Fix README heading and broken documentation links.
3. Add rollback tests and transactional cleanup in `InferenceRuntime::load_model` and `start_request`.
4. Define a generation-loop milestone and stop adding unrelated phases until it works.

## During generation integration

5. Wire `tokenization`, `context-planner`, and `sampling` into runtime/application behavior.
6. Add bounded token/text output and cancellation tests.
7. Run the same generation contract tests against Candle and GGUF.
8. Add system benchmarks for TTFT, decode throughput, memory, and cancellation.

## After the first vertical slice

9. Decide whether `application-runtime` needs coarse ports, a backend enum, or a native-composition split based on actual Tauri/Leptos requirements.
10. Move or narrow corrective workflow APIs.
11. Replace the hard F0/F1 table with a reviewed dependency DAG.
12. Convert the maintenance runner to `xtask` and simplify Cargo commands.
13. Audit duplicate dependencies, licenses, advisories, and portable targets.

---

# References

Official and primary references used to evaluate Cargo/Rust conventions:

- Cargo package targets and benchmark layout: <https://doc.rust-lang.org/cargo/reference/cargo-targets.html>
- Cargo benchmark command: <https://doc.rust-lang.org/cargo/commands/cargo-bench.html>
- Cargo test target selection: <https://doc.rust-lang.org/cargo/commands/cargo-test.html>
- Cargo configuration aliases: <https://doc.rust-lang.org/cargo/reference/config.html#alias>
- Cargo external tools and custom subcommands: <https://doc.rust-lang.org/cargo/reference/external-tools.html>
- `cargo_metadata` typed Cargo metadata API: <https://docs.rs/cargo_metadata/latest/cargo_metadata/>
- Criterion.rs guide: <https://bheisler.github.io/criterion.rs/book/>
- Rust const evaluation/reference: <https://doc.rust-lang.org/reference/const_eval.html>
- `core::error::Error`: <https://doc.rust-lang.org/core/error/trait.Error.html>
- Rust code-generation attributes: <https://doc.rust-lang.org/reference/attributes/codegen.html>
