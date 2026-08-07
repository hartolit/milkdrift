# Milkdrift Phase 12 — core loader and runtime ownership

This is one ownership segment of Phase 12, not a new project phase. Work directly in the local codebase in Zed and implement the changes. Do not return a patch or code block.

## Purpose

Replace the current homogeneous Safetensors scalar assumption and approximate load planning with a truthful per-tensor inspection, preparation, admission, loading, and cleanup transaction for the local Candle endpoint.

This segment owns the complete lower model-loading path because the generic backend contract, E0 ownership rules, and Candle implementation must agree. Do not spread this work into the application layer, frontend, workflow runtime, or future provider/peer abstractions.

## Read before editing

Read only the documents needed to preserve the governing invariants:

- `docs/vision.md`
- `docs/rules.md`
- `docs/project/architecture.md`
- `docs/project/candle-backend.md`
- `docs/project/inference-runtime.md`
- `docs/project/lifecycle.md`
- `docs/agent/execution/analyzer.md`
- `docs/agent/execution/milkdrift-phase12-per-tensor-safetensors-compatibility.md`
- the accepted ADRs governing backend verification, explicit cleanup/shutdown, device execution, and memory accounting

Then inspect the current implementations and tests in:

- `crates/domain/domain-contracts`
- `crates/runtime/inference-runtime`
- `crates/adapters/candle-backend`

Understand the existing prepare/validate/commit and cleanup-quarantine behavior before changing public contracts.

## Scope

You may change:

- portable model-loader, loaded-model, descriptor, planning, receipt, footprint, and error contracts when a generic change is genuinely required;
- E0 model admission, validation, accounting, failure retention, snapshots, commands/events, and deterministic fake-backend tests;
- Candle source inspection, device-aware planning, tensor materialization, conversion, model construction, failure mapping, and CPU/CUDA tests;
- package manifests and the lockfile when a direct parsing dependency is justified;
- Rustdoc local to these contracts.

Do not change:

- `application-runtime`, Hugging Face resolution, redb persistence, or Slint in this segment;
- workflow, workspace, authority, plugin, provider, peer, or control-center architecture;
- unrelated domain algorithms or repository-wide documentation;
- the accepted no-fallback device policy.

## Required semantic separation

Use precise terms throughout code, tests, errors, and Rustdoc:

- **Configuration-declared scalar:** optional or recognized model-configuration metadata. It is evidence about producer intent, not proof of tensor homogeneity.
- **Observed tensor dtype/layout:** facts read from every Safetensors tensor header across all selected shards.
- **Execution scalar:** the scalar selected for materialized backend execution tensors on the requested device.

Do not retain or reintroduce a type or field whose name implies that a single declared scalar is the observed dtype of every source tensor.

## Architecture constraints

### Keep format details inside the adapter

Safetensors tensor names, byte offsets, shard paths, header DTOs, and per-file parsing structures belong to `candle-backend`.

A generic domain or E0 contract may contain only format-neutral facts required to:

- admit resources;
- validate a prepared load against the loaded result;
- identify declared versus actual execution semantics;
- retain cleanup ownership;
- report stable failures.

Do not publish an unbounded vector of tensor names or a Safetensors-specific structure from `domain-contracts` merely because the adapter needs it internally.

### Prefer one coherent preparation/load transaction

Inspect the current `ModelLoader::plan_load` and `ModelLoader::load` relationship. The accepted plan and the load must not silently operate on materially different artifact facts.

Design the narrowest generic contract that provides a truthful transaction. A likely shape is a backend-owned prepared load value that:

- contains or securely references the inspected artifact state;
- exposes a format-neutral public `LoadPlan` for E0 admission;
- is consumed or mutably owned during materialization;
- preserves enough ownership to clean up a partial failure;
- prevents the runtime from publishing a model before all validation succeeds.

Do not force this exact API if a simpler design satisfies the same invariants. If the existing contract can remain, prove how it prevents inspection/load drift and retains partial-load ownership. Do not add abstractions merely for symmetry.

### Preserve static execution boundaries

Token- and tensor-sensitive work remains statically dispatched. Do not introduce trait objects into the model hot path. Cold preparation may use ordinary readable allocations and maps when bounded by inspected artifact data.

### Preserve portability

`domain-contracts` remains `no_std`. Do not leak filesystem, path, serde, Candle, Safetensors, device-driver, owned diagnostic, or host-thread types into it.

## Implementation requirements

### 1. Inspect every shard before device allocation

Build a safe header-inspection path that runs before model tensor allocation on CPU or CUDA.

It must:

- inspect every selected Safetensors shard;
- validate header length and bounds before allocation proportional to untrusted metadata;
- validate tensor shapes and checked element counts;
- validate non-overlapping/in-range payload offsets according to the selected parser's guarantees;
- reject duplicate tensor names across shards deterministically;
- classify every observed tensor dtype;
- reject unsupported dtypes for the current unquantized Llama path before creating device tensors;
- use checked arithmetic for tensor bytes, aggregate bytes, conversion bytes, and peak calculations;
- produce deterministic results independent of filesystem enumeration order.

Use an established safe parser API when it provides the required metadata semantics. Add a direct dependency rather than relying accidentally on a transitive crate. Do not add project-authored `unsafe` or use mmap APIs that violate the workspace policy.

### 2. Define the reviewed conversion policy

The current unquantized path should support the reviewed floating source subset required by Phase 12, including deterministic mixed F16/F32 and BF16/F32 fixtures.

Make the policy explicit for:

- homogeneous F32;
- homogeneous F16;
- homogeneous BF16;
- mixed F16/F32;
- mixed BF16/F32;
- absent, recognized, contradictory, or unsupported configuration-declared scalar metadata;
- CPU execution;
- supported CUDA execution and BF16 capability;
- unsupported source dtypes or combinations.

The declared scalar may influence execution-policy selection if that remains the reviewed design, but it must never be used as a substitute for observed header facts. Avoid silent precision loss that is not represented by the policy.

### 3. Make memory planning exact for the chosen algorithm

Remove the approximation that scales complete shard file lengths by source/execution bytes per element.

Derive the accepted footprint from inspected tensor shapes and dtypes:

- exact source tensor payload bytes;
- exact final execution tensor bytes;
- exact model cache bytes per token for the execution scalar;
- final host or device weight ownership;
- host and device transient peaks caused by the actual loading and conversion algorithm;
- any full-shard, per-tensor, staging, conversion, or duplicate-residency peak;
- checked arithmetic and explicit overflow failures.

Header bytes, metadata, allocator behavior, driver state, and physical RSS/VRAM observations must not be mislabeled as deterministic tensor ownership. Document precisely what `MemoryFootprint` accounts for and what it does not.

If the existing aggregate footprint shape cannot truthfully express the chosen algorithm, make the smallest format-neutral contract change needed. Do not add a general memory ontology without a consumer.

### 4. Resolve partial-load ownership

Treat this as a release-blocking requirement.

For every failure point after host or device tensor materialization begins, determine:

- which value owns already materialized tensors;
- whether pending device work must be synchronized before destruction;
- how synchronization failure is represented;
- whether cleanup can be retried;
- how ownership and admitted bytes remain visible when cleanup does not complete;
- when ordinary Rust drop is verified to be sufficient and when it is not accepted as proof of backend release.

A failed load must result in one of two truthful states:

1. no backend resources remain and the failure returns normally; or
2. a retained cleanup owner remains reachable and accounted through the existing E0 cleanup model or a narrowly extended equivalent.

Do not lose the only owner inside a converted error. Do not claim cleanup success based only on scope exit when the selected device may retain asynchronous work.

### 5. Preserve E0 verification

E0 must continue to validate the backend rather than trusting trait conformance.

At minimum, preserve or strengthen validation of:

- model handle and generation identity;
- complete descriptor facts;
- requested versus actual execution device;
- planned versus actual execution scalar;
- planned versus adapter-reported accounted footprint;
- lifecycle transition ordering;
- no publication before complete validation;
- retained accounting after post-load validation or cleanup failure.

If preparation becomes a consumable transaction, test that E0 admits against exactly the plan belonging to that transaction and cannot accidentally load an unrelated or stale preparation.

### 6. Keep cleanup local and remove superseded assumptions

Within these three owned areas, remove obsolete helpers, duplicate scalar mappings, comments, tests, and error branches that existed only for the homogeneous-source assumption.

Do not perform an unrelated global cleanup. Do not create micro-crates for one type or one parser helper.

## Required tests

Use project-authored tiny deterministic artifacts. Do not download models in ordinary tests.

Add or update tests for:

- homogeneous F32/F16/BF16 regression behavior;
- mixed F16/F32 inspection, exact planning, conversion, load, prefill/decode, and unload;
- mixed BF16/F32 inspection and CPU execution policy;
- supported CUDA mixed-dtype execution under the existing feature/hardware boundary;
- unsupported dtype rejection before device allocation;
- duplicate tensor names across shards;
- malformed header, invalid bounds, impossible shape/byte count, and arithmetic overflow;
- exact payload and conversion-aware footprint calculations;
- host and device budget rejection before materialization;
- declared metadata differing from one or more observed tensors without false corruption classification;
- plan/load or preparation identity consistency;
- backend contract violation after load and successful cleanup;
- partial materialization failure with successful cleanup;
- partial materialization failure with cleanup/synchronization failure and retained ownership/accounting;
- no double release or double accounting during retries;
- existing cancellation, generation, unload, and shutdown behavior after the contract change.

Use deterministic fault injection at the narrowest layer rather than trying to force real CUDA failures nondeterministically.

## Validation

Run targeted gates before ending this segment:

```text
cargo fmt --all -- --check
cargo check --locked -p domain-contracts -p inference-runtime -p candle-backend --all-targets
cargo test --locked -p domain-contracts -p inference-runtime -p candle-backend
cargo clippy --locked -p domain-contracts -p inference-runtime -p candle-backend --all-targets -- -D warnings
cargo xtask architecture
cargo xtask hygiene
```

Also compile the CUDA feature path. Run hardware tests only when the accepted CUDA environment is available; otherwise report them as pending without making a support claim.

Preserve the existing portable target checks for changed `no_std` crates.

## Completion report

Finish with a concise report containing:

- the final contract between preparation, admission, loading, and cleanup;
- the final declared/observed/execution scalar meanings;
- the exact CPU and CUDA memory-peak formulas implemented;
- the partial-load failure ownership state machine;
- public API breaks and migration performed inside the repository;
- tests and commands executed, including any unavailable hardware evidence;
- remaining risks or explicitly unsupported layouts.

Create one coherent commit only after the targeted gates pass, unless the repository's current execution instructions require a different commit policy.
