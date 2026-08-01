# ADR-0018: Separate benchmark roles and govern measurement artifacts

- **Status:** Accepted
- **Date:** 2026-08-01

## Context

Phase 10 needs component and cross-crate measurements, but creating a `benches/` directory for every checklist item would imply measurement coverage that does not exist. A top-level benchmark package also needs a non-production dependency role, one lockfile/target convention, and fail-closed artifact rules before it is created.

The repository additionally commits one tiny Candle/Safetensors integration fixture. History and tensor inspection showed that the prior bytes had synthetic structure and matched an in-repository deterministic generator, but the introducing squash did not record fixture-specific authorship, generation, licensing, or an authorized chain of title. Small size and apparent synthetic content do not establish redistribution rights.

Benchmarking real models creates a separate concern: download permission is not necessarily redistribution permission, and ordinary CI must remain download-free.

## Decision

### Measurement placement

A benchmark for one crate-owned operation lives in that crate's conventional `benches/` directory. Such a directory is created only in the same change as a real benchmark when:

- the operation is stable enough to measure;
- the benchmark answers a named performance or regression question;
- the measurement executes production code rather than a benchmark-only reimplementation.

No placeholder `benches/` directory or checklist-driven component benchmark is created.

Cross-crate E0/E1 and product-level measurements live in the dedicated root-workspace package at `benchmarks/runtime`, named `runtime-benchmarks`. That package is intentionally deferred to Phase 10; this decision does not create it or record a performance result.

`runtime-benchmarks` is a non-production outer consumer. Its dependencies point to exact reviewed public production APIs. No production, tooling, test, or application package may depend on the benchmark package. Benchmark-only helpers do not become speculative public production APIs. A shared benchmark-support package requires at least two implemented consumers and a clear ownership boundary.

### Workspace and build behavior

Every future package under `benchmarks/`:

- is an exact root-workspace member rather than a nested workspace;
- uses the root `Cargo.lock` and shared root `target` directory;
- declares `publish = false`;
- has no custom build target or `build.rs`;
- receives only exact reviewed local and external dependency edges;
- fails architecture validation when its path or role is unknown.

The authoring order is mandatory: register `benchmarks/runtime` in root `workspace.members`, create its manifest in the same change, and only then run Cargo from the repository root. An `xtask` invoked through Cargo cannot prove historical command order, so hygiene enforces the resulting root-membership invariant and documentation defines the required sequence.

Build scripts must never download models, access the network, execute measurements, generate benchmark results, probe runtime performance metadata, or write into the source tree. No benchmark package uses a build script at all.

### Generated measurements and caches

Cargo target directories are ignored at every repository depth, while all project commands use the shared root target directory. Raw Criterion samples and HTML, generated reports, compiler intermediates, flamegraphs, profiler output, heap dumps, and benchmark/model caches remain under root `target` or outside the repository and are not committed.

Curated baseline summaries belong in `docs/project/performance.md` or another explicitly designated canonical document. Repository hygiene rejects tracked `target` components, nested benchmark lockfiles, generated result trees, benchmark build scripts, and model-cache trees.

### Model fixtures

A committed model fixture requires a provenance record containing:

- project/external origin and redistribution basis;
- architecture, scalar type, and deterministic generation method;
- exact size and SHA-256 for every file;
- license;
- intended test scope and explicit non-claims.

Project-authored synthetic fixtures use Rust/Cargo-native generation, no external base-model weights or tokenizer assets, and no network access. They remain with their real consumer until a second consumer makes a shared location useful.

When provenance or redistribution permission is not established, existing bytes are not reused or expanded merely because they are small. They are replaced with newly generated, documented project-owned synthetic data. The Candle fixture was replaced under this rule; its audit, generator, old hashes, and current hashes are recorded in `crates/runtime/inference-runtime/tests/fixtures/candle-llama/PROVENANCE.md`.

Real-model performance measurements are explicit and opt-in. They name an external model identifier and immutable revision, use an existing local cache or explicit local artifact path, perform no ordinary-CI download, and do not redistribute model or tokenizer files through this repository. A hosting page that permits download is not sufficient evidence to commit its artifacts.

## Rejected alternatives

- **Create placeholder benchmark directories for every Phase 10 checklist item:** empty structure does not answer a measurement question and creates false completion signals.
- **Put all measurements in one top-level package:** crate-owned operations lose their conventional ownership and become easier to reimplement incorrectly.
- **Put cross-crate harnesses in a production crate:** that reverses dependency direction and exposes benchmark-only concerns to product code.
- **Create a shared benchmark-support crate immediately:** there are not yet two real consumers or a proven ownership boundary.
- **Commit raw Criterion or profiler output:** generated samples are large, environment-specific, and belong under `target`; only curated conclusions are durable project evidence.
- **Retain the prior tiny fixture based on size or inferred synthetic content:** those facts do not establish authorship or redistribution permission.
- **Commit a downloaded tiny/random model for convenience:** download availability does not establish repository redistribution rights, and correctness fixtures do not need external trained weights.
- **Use a build script for model acquisition or metadata capture:** build execution must remain reproducible and must not perform network, measurement, or source-tree mutation.

## Consequences

- Phase 10 begins from one documented benchmark architecture without an empty benchmark package.
- Component measurements remain close to their production owner, while one future package owns system measurements.
- Production-to-benchmark edges and unknown benchmark paths fail closed.
- Generated output and model caches cannot become tracked repository trees unnoticed.
- The committed Candle fixture has reproducible project-owned bytes and an explicit Apache-2.0 provenance record.
- Real-model measurements remain local/opt-in and cannot silently turn a cache into redistributed repository content.

## Review trigger

Review when a second real cross-crate benchmark package is required, two implemented consumers justify shared benchmark support or fixture ownership, Cargo workspace behavior changes materially, a model fixture needs external source material, or a controlled baseline requires an artifact policy not represented here.
