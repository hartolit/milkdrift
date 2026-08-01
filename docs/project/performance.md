# Performance evidence

## Policy

Optimization changes require before-and-after measurements on the same host with the same toolchain and benchmark configuration. Absolute timing varies with CPU frequency, thermal state, operating-system scheduling, compiler version, and background work, so local results are evidence rather than portable pass/fail thresholds.

Allocation tests remain deterministic enforcement gates. The canonical `cargo xtask verify` command compiles benchmark targets without running their statistical measurements. Run the sampling benchmark as a direct Cargo operation because ordinary Cargo commands are not forwarded through `xtask`:

```text
cargo bench --locked -p sampling --bench sampling_pipeline
```

## Benchmark placement and repository artifacts

A benchmark for one stable crate-owned operation lives in that crate's conventional `benches/` directory. Create the directory only with a real benchmark that executes production code and answers a named performance or regression question. Do not create placeholder benchmark directories or add a component measurement merely because a crate appears in a plan checklist.

Cross-crate E0/E1 and product-level measurements belong only in the future root-workspace package `benchmarks/runtime` (`runtime-benchmarks`). That package is not present and Phase 10 has not started. When created, it:

- is registered in root `workspace.members` before Cargo is run against its manifest;
- uses the root `Cargo.lock` and root `target`;
- declares `publish = false` and no custom build target or `build.rs`;
- depends only on exact reviewed public production APIs;
- receives no dependency from a production/tooling/test/application package.

A shared benchmark-support package requires at least two implemented consumers and a clear owner. Benchmark convenience is not a reason to expose speculative public production APIs.

Raw Criterion samples/HTML, generated reports, flamegraphs, profiler output, heap dumps, compiler intermediates, and model caches belong under the shared root `target` directory or outside the repository. They are not committed. Curated environment and baseline summaries belong in this document or another explicitly designated canonical path.

Build scripts are not measurement tooling. They must not access the network, download models, run benchmarks, generate results, probe runtime performance metadata, or write into the source tree; benchmark packages have no build script at all.

## Phase 10 entry scope

The mandatory Phase 10 measurements are:

1. expansion of the existing sampling benchmark;
2. one cross-crate runtime/system harness;
3. reproducible environment and artifact metadata;
4. controlled lifecycle and host-memory measurements.

Tokenizer encode/streaming-decode, context-planner, output-accumulator, and isolated Candle prefill/decode microbenchmarks are conditional. Before implementation, each must name its question and explain why the system harness is insufficient.

Real-model measurements use an explicit external model identifier and immutable revision, an existing local cache or explicit local artifact path, and opt-in execution. Ordinary CI performs no model download, and model/tokenizer files are not redistributed through this repository. The committed synthetic Candle fixture is for execution/lifecycle integration, not model-quality or real-model performance measurement.

## Sampling pipeline

`crates/domain/sampling/benches/sampling_pipeline.rs` measures the production `Sampler::sample` implementation with the default sampling policy and 32,768 logits. Each iteration restores the mutable logit slice, performs top-k selection, probability filtering, and random selection, and returns the selected sample to a compiler black box.

The prepared sampler reserves:

- mutable F32 logits: 128 KiB;
- U32 candidate indices: 128 KiB;
- U32 repetition epoch table: 128 KiB;
- total reserved sampler slices: 384 KiB, excluding repetition history.

The default repetition penalty is one, so this benchmark does not mutate the epoch table. Its active mutable working set is 256 KiB for logits and candidate indices. It additionally reads a 128 KiB baseline slice to restore input because sampling overwrites logits. That restoration cost is included in the reported end-to-end time. All vectors are allocated before the measured region.

### Baseline recorded 2026-07-22

Environment:

- CPU: AMD Ryzen 9 5950X, 16 cores and 32 hardware threads;
- target: `x86_64-unknown-linux-gnu`;
- compiler: Rust 1.96.1, LLVM 22.1.2;
- profile: Cargo `bench` optimized profile;
- Criterion: 0.8.2, 100 measured samples.

Observed interval:

```text
time:       80.726 µs to 82.028 µs per sample
throughput: 399.48 Melem/s to 405.92 Melem/s
```

Six measurements were classified as high outliers. No source optimization is justified from this baseline alone; a proposed change must be compared under equivalent conditions and should include profiler evidence identifying the cost.

## Allocation enforcement

`crates/domain/sampling/tests/allocation.rs` executes the production sampler 64 times over preallocated logits, candidate indices, repetition state, and history. The test enables repetition processing and fails if the measured region allocates or reallocates.

This allocator observes Rust global-allocator traffic. It does not observe native allocation inside Candle, device drivers, or operating-system mappings. `candle-backend` therefore does not advertise an allocation-free hot path: Candle 0.11 grows KV-cache tensors and creates forward-pass intermediates.

## Current product measurements

No Phase 10 performance result has been recorded. Historical Candle smoke measurements are preserved in [execution history](../agent/execution/history.md) and apply only to their named baseline. They are not current-tree performance evidence.

The active Rust-native E1 smoke in [validation](validation.md#rust-native-candle-hub-smoke) is a correctness/lifecycle check, not a stable benchmark. Phase 10 must measure the current Candle path's time to first token, prefill/decode throughput, cancellation latency, load/unload duration, repeated lifecycle stability, and host memory with the exact repository, immutable revision or fixture hash, prompt, settings, toolchain, hardware, OS/kernel, build profile, and environment controls recorded.

There is no current second local engine or GPU path to compare. Future Candle-native format, quantization, or device work must establish its own controlled baseline rather than inheriting CPU/Safetensors measurements.
