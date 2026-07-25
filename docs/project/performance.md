# Performance evidence

## Policy

Optimization changes require before-and-after measurements on the same host with
the same toolchain and benchmark configuration. Absolute timing varies with CPU
frequency, thermal state, operating-system scheduling, compiler version, and
background work, so local results are evidence rather than portable pass/fail
thresholds.

Allocation tests remain deterministic enforcement gates. Statistical benchmarks
are invoked separately because they require an optimized build and sustained host
execution:

```text
cargo run --bin llm-app -- benchmark
```

## Sampling pipeline

`crates/features/sampling/benches/sampling_pipeline.rs` measures the production
`Sampler::sample` implementation with the default sampling policy and 32,768
logits. Each iteration restores the mutable logit slice, performs top-k selection,
probability filtering, and random selection, and returns the selected sample to a
compiler black box.

The prepared sampler reserves the following contiguous slices:

- mutable F32 logits: 128 KiB;
- U32 candidate indices: 128 KiB;
- U32 repetition epoch table: 128 KiB;
- total reserved sampler slices: 384 KiB, excluding repetition history.

The default repetition penalty is one, so this benchmark does not mutate the epoch
table. Its active mutable working set is therefore 256 KiB for logits and candidate
indices. The benchmark additionally reads a 128 KiB baseline logit slice to restore
input because sampling intentionally overwrites logits. That restoration cost is
included in the reported end-to-end time. All vectors are allocated before the
measured region.

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

Six measurements were classified as high outliers. No source optimization is
justified from this baseline alone; a proposed change must be compared against it
under equivalent conditions and should include profiler evidence identifying the
cost being addressed.

## Allocation enforcement

`crates/features/sampling/tests/allocation.rs` executes the production sampler 64
times over preallocated logits, candidate indices, repetition state, and history.
The test enables repetition processing and fails if the measured region performs
an allocation or reallocation.

This allocator observes Rust global-allocator traffic. It does not observe native
allocators inside Candle, llama.cpp, drivers, or operating-system mappings.
