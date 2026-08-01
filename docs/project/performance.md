# Phase 10 performance evidence

## Evidence policy and boundaries

Phase 10 establishes reproducible evidence; it does not set portable pass/fail timing thresholds. Absolute timing varies with CPU frequency, thermal state, operating-system scheduling, compiler version, and background work. Compare changes on the same host with the same toolchain, profile, fixture, workload, and benchmark configuration. Hard harness timeouts stop hangs only. Lifecycle, identity, accounting, cleanup, or output mismatches fail a run; slow measurements do not.

No performance optimization was made in Phase 10. The implemented work is measurement and regression infrastructure only.

| Evidence class | Implemented harness | Boundary and use | Not claimed |
|---|---|---|---|
| Sampling component | `crates/domain/sampling/benches/sampling_pipeline.rs` | Criterion measurements around public sampling and stop-matching calls using caller-owned, preallocated storage. | E0/E1 latency, model decode throughput, product performance, or native/device allocation behavior. |
| Hosted E0 component-like | `benchmarks/runtime/benches/runtime.rs` | Criterion measurements from public hosted command submission through the matching E0 completion event. This intentionally includes bounded transport, dispatch, Candle execution, event publication, and receive. | Raw Candle kernel timing, E1/product latency, full-generation throughput, RSS, or allocation attribution. |
| Synthetic system/integration | `benchmarks/runtime/src/bin/baseline.rs`, default mode | Bounded, download-free public E0 lifecycle/output measurements plus repeated download-free E1 startup/shutdown, accounting, cleanup, and sampled process RSS. | Product-model speed or quality, production steady state, isolated allocator behavior, or device-memory attribution. |
| Real-product system/integration | The same normal runner with `--mode real-product` | Opt-in public E1 resolution, loading, decoded output, usage, unload, shutdown, and RSS for one exact immutable model pin. | A model survey, production serving load, quality evaluation, or isolated allocation evidence. |

Criterion targets are comparative statistical component evidence. The normal runner is system/integration evidence and emits bounded per-cycle measurements rather than Criterion distributions.

## Sampling component matrix

The implemented Cartesian product is:

```text
{sample_only,restore_and_sample}/<case>/{8192,32768,131072}
```

This yields 48 sampling targets: two timing boundaries, eight cases, and three vocabulary sizes. Every target uses deterministic pseudo-random logits bounded to `[-8, 8]`, sampler seed 29, one prepared `Sampler`, and caller-owned logits, indices, seen-token state, and history. Fixture construction, allocation, sampler construction, and capacity reservation occur before measurement; each iteration reuses those capacities. `Throughput::Elements` means one vocabulary-sized logit set per sample, not generated tokens.

### Timing boundaries

| Prefix | Question | Exact timed boundary |
|---|---|---|
| `sample_only/<case>/<vocabulary>` | What does the production sampling call cost after mutable logits are ready? | The baseline-to-working-logit copy completes before `Instant::now`. Timing includes public workspace-view construction, `Sampler::sample`, result checking, and `black_box`. |
| `restore_and_sample/<case>/<vocabulary>` | What does a caller pay when it must restore the logit buffer that sampling overwrites? | `Instant::now` precedes the baseline-to-working-logit copy. Timing includes that copy and the complete `sample_only` boundary. |

### Cases and questions

Each case below is implemented at 8,192, 32,768, and 131,072 logits under both timing boundaries.

| Case segment | Question | Configuration and history |
|---|---|---|
| `greedy` | How does deterministic highest-logit selection scale with vocabulary size? | `SamplingConfig::greedy()`; empty history. |
| `default_top_k_top_p` | What is the component cost of the application default top-k/top-p policy? | `SamplingConfig::default()`; currently top-k 40 and top-p 0.95; empty history. |
| `min_p_0_05_full_vocabulary` | What does min-p filtering cost when top-k does not pre-truncate the vocabulary? | Default temperature, top-k 0, top-p 1.0, min-p 0.05; empty history. |
| `repetition_disabled_history_256` | Does a long supplied history remain a cheap baseline when repetition penalty is disabled? | Default policy with penalty 1.0 and full-history convention; 256 entries cycling over four token IDs. |
| `repetition_enabled_empty` | What fixed cost appears when repetition processing is enabled but history is empty? | Default top-k/top-p, penalty 1.1, full-history convention; empty history. |
| `repetition_short_unique_8` | What does penalizing a short distinct-token history cost? | Same enabled policy; eight distinct token IDs. |
| `repetition_medium_unique_64` | How does the enabled path change at a 64-token history? | Same enabled policy; 64 distinct token IDs. |
| `repetition_repeated_heavy_256` | What does scanning a longer duplicate-heavy history cost? | Same enabled policy; 256 entries cycling over four token IDs. |

Fixed target storage is:

| Vocabulary | Three sampler slices: logits, indices, seen state | Untimed baseline logits | Maximum history |
|---:|---:|---:|---:|
| 8,192 | 96 KiB | 32 KiB | 256 `TokenId` values |
| 32,768 | 384 KiB | 128 KiB | 256 `TokenId` values |
| 131,072 | 1.5 MiB | 512 KiB | 256 `TokenId` values |

The measured loops call no allocation API and perform no per-iteration clone, but Criterion does not count allocator events. `crates/domain/sampling/tests/allocation.rs` remains the deterministic Rust-global-allocator gate over 64 preallocated sampling calls with repetition enabled. It does not observe Candle, native-library, driver, operating-system, or device allocations.

### Stop matching

All inputs are caller-owned and prepared outside timing. Only public `match_stop_suffix` plus `black_box` is timed.

| Target | Question and input |
|---|---|
| `stop_matching/token_hit/1_pattern_generated_128` | What is the one-token stop cost when the sole configured sequence matches? The generated history has 128 tokens and one one-token pattern. |
| `stop_matching/pattern_hit_last/8_patterns_generated_128` | What is suffix-match cost when a four-token match is last among eight patterns? Seven misses precede the hit. |
| `stop_matching/pattern_miss/8_patterns_generated_128` | What is the no-match scan cost across eight patterns and 128 generated tokens? |

These are statistical component-regression targets only. They do not establish allocation freedom, E0/E1 behavior, or product throughput.

## Runtime normal-runner contracts

### Synthetic mode

Default and recorded settings are one warmup cycle followed by three sample cycles. Warmup records remain in JSON for lifecycle auditability but are excluded from summary statistics. Both counts must be non-zero; the CLI bounds warmups to 10 and samples to 20.

Each cycle starts a fresh hosted E0 worker, verifies and loads the fixture once, runs a separately checked prefill and three bounded generation scenarios, unloads, performs ticketed shutdown, waits for worker completion, and joins. Any `CleanupPending`, `CleanupExhausted`, degraded model, maintenance error, residual request/workspace, unexpected shutdown cleanup, or failed join invalidates the cycle.

| Metric/question | Exact measured boundary | Setup, validation, or interpretation outside timing |
|---|---|---|
| E0 worker start | `start_hosted_runtime` call through returned client and thread handles. | An immediate `before-load` snapshot is an untimed readiness/accounting handshake. |
| Model load | Immediately before public `RuntimeCommand::LoadModel` submission through matching `ModelLoaded`. | Source construction and fixture checks precede timing; descriptor and accounting checks follow it. |
| Hosted checked prefill | Immediately before public `RuntimeCommand::Prefill` submission through matching `PrefillCompleted`; throughput is four consumed prompt tokens. | Load, request/sequence creation, prompt ownership, and vocabulary-sized logits allocation are outside timing. Logits, checked outcome/usage, completion, release, and accounting are then validated. |
| Submission to first token at pull | Immediately before public `RuntimeCommand::Generate` submission through the first generated token observed at `HostedRuntime::pull_token_output`. | Request construction is outside timing. Admission and token/output identities are checked; pull capacity one makes the first public observation unambiguous. |
| Four-token post-first proxy | First-token observation at the public pull boundary through four additional observed tokens. | This is a synthetic short-window integration proxy, explicitly **not production steady state**. Matching `Terminal` and `Released` remain mandatory outside the window. |
| Backpressure hold and recovery | A fixed 100 ms no-pull hold is recorded separately. Recovery begins immediately before the pull that frees the full one-token accumulator and ends when the next token is observed at a later public pull. | The hold is controlled setup, not a threshold. The run requires `Yielded(OutputBackpressure)`, a during-backpressure snapshot, token-limit `Terminal`, `Released`, and clean accounting. |
| Cancellation | One start immediately before public `CancelRequest` submission; independent durations end at the matching acknowledgement, observable cancellation `Terminal` at pull, and observable `Released`. | An untimed first token proves progress, then a short no-pull hold keeps the request active. Non-zero partial generation and `Cancelled(UserRequested)` terminal states are required. |
| Model unload | Immediately before `UnloadModel(RejectIfBusy)` submission through `ModelUnload(Unloaded)`. | All requests must be released, cancellation count must be zero, and the post-unload snapshot must be exactly empty. |
| E0 shutdown and join | Total begins immediately before `Shutdown` submission and ends after successful worker join; event and post-event join portions are also recorded. | A clean receipt unloads zero models and cancels zero requests. |
| Download-free E1 start | `ApplicationRuntime::start` through returned runtime, using a unique temporary redb file and starting both production workers without Hub resolution. | Clean idle initial state is mandatory. |
| Download-free E1 shutdown | `ApplicationRuntime::shutdown` through successful bounded worker shutdown/join. | Terminal worker state is validated and the temporary workspace is deleted. |

Each E0 cycle records paired public `RuntimeSnapshot` accounting and process memory before load, after load, after checked-prefill release, after first-token/proxy release, during backpressure, after backpressure release, after cancellation release, and after unload. Each download-free E1 startup cycle records process memory before start, after start, and after shutdown.

The E1 startup/shutdown lifecycle uses the same warmup/sample counts as E0 but fresh `ApplicationRuntime` instances. Together these repeated cycles answer whether load/generate/release/unload/shutdown restores deterministic ownership state; they are not a long-running soak test.

### Deterministic fixture identity and provenance

The runtime harness references, without copying, `crates/runtime/inference-runtime/tests/fixtures/candle-llama`, anchored from `CARGO_MANIFEST_DIR`. It is project-authored deterministic synthetic integration data documented by its `PROVENANCE.md`; it contains no trained or externally sourced model weights.

| File | Exact size | Required SHA-256 |
|---|---:|---|
| `config.json` | 360 bytes | `052b5c325859dc723ed0825f711950cbff112a140239953273cebacdb36afdd0` |
| `model.safetensors` | 4,800 bytes | `cc4798af93488b4fb2ae0548c2b28ace600521732b52023a7786c3227d72d672` |

Before normal-runner or Criterion setup, shared fixture verification checks presence, regular-file status, exact byte sizes, recomputed SHA-256 using `sha2` 0.11, and parsed configuration fields. The loaded public descriptor must then report Candle / Llama / Safetensors / unquantized F32 / vocabulary 16 / context 16.

This fixture proves execution and lifecycle integration only. It is not evidence of model quality, real-product performance, representative vocabulary/context scale, production steady-state throughput, or device/native-memory behavior.

### Hosted E0 Criterion targets

`benchmarks/runtime/benches/runtime.rs` implements two targets with a 500 ms warmup, two-second measurement window, 10 samples, and no regression threshold.

| Target | Question | Timed in | Timed out |
|---|---|---|---|
| `e0_hosted_checked_prefill/4_tokens` | Did hosted checked-prefill submission-to-event cost regress for the deterministic four-token input? | `try_submit(Prefill)` through receipt and ticket matching of `PrefillCompleted`, including bounded transport, E0 dispatch/checking, Candle execution, event publication, and receive. | Fixture/source construction, worker start, model load, request/sequence and prompt creation, logits allocation, result validation, request completion, unload, shutdown, and join. |
| `e0_hosted_incremental_decode/1_token_after_2_token_prefill` | Did one hosted incremental decode regress after a prepared two-token prefix? | `try_submit(Decode)` through receipt and ticket matching of `DecodeCompleted`, with the same hosted transport/dispatch/event boundary. | Request/sequence creation, two-token setup prefill, token and logits setup, completion, and all lifecycle setup/teardown. |

`iter_custom` accumulates only the named boundary. The vocabulary-sized logits vector is allocated before measurement and returned from each event for reuse. Model load and worker start occur once per target. Candle tensor allocation or KV/cache growth that occurs inside the boundary contributes elapsed time, but the benchmark has no allocator instrumentation or RSS sampling and cannot attribute that work.

### Pinned real-product mode — implemented, not executed

The E1 normal-runner mode hardcodes and verifies:

- repository: `neubla/tiny-random-LlamaForCausalLM`;
- immutable revision: `1c81a3fba044af78df253edc66bdbab183184932`;
- Candle / Hugging Face Hub / CPU / Safetensors / F32;
- prompt `Hello`, deterministic top-k-1 generation, seed 39, eight generated tokens, and a four-token post-first public-usage window.

There are no repository, revision, or model-substitution flags. `--cache-dir PATH` is mandatory and must be an existing directory whose canonical location is under shared root `target/` or outside the repository; source-tree cache locations are rejected. Public E1 always performs immutable Hub metadata resolution, so `--allow-network` is mandatory and `HF_HUB_OFFLINE=1` is rejected as contradictory.

| Metric/question | Exact measured boundary |
|---|---|
| Application start | `ApplicationRuntime::start` through returned runtime. |
| E1 resolution/cache/download | Immediately before `resolve_model` through `ApplicationEvent::ModelResolved`, separate from model load. A network-enabled first warmup may include download and remains recorded rather than being mislabeled cached performance. |
| Candle model load | Immediately before `load_model` through `ApplicationEvent::ModelLoaded`, after resolution. |
| First decoded output and usage | Immediately before `start_generation("Hello", …)` through the first non-empty decoded fragment at public `pull_output`. Only byte count and public prompt/generated usage are recorded, never text or token IDs. |
| Post-first proxy | First decoded-output observation through the fixed four-token public usage window when observable; a shorter remainder is labeled if generation ends first. This is not steady state. |
| Normal generation/release | Eight-token top-k-1 completion requiring matching text-output `Terminal`, text-output `Released`, `GenerationFinished`, non-zero decoded bytes, exact usage, and no cleanup-pending event. |
| Unload | Immediately before `unload_model_with_behavior(RejectIfBusy)` through `ModelUnloaded` with zero cancellations. |
| Application shutdown | `ApplicationRuntime::shutdown` through successful bounded worker shutdown/join. |

RSS would be sampled before/after start, resolution, load, released generation, unload, and shutdown. **This mode was compiled but not run for the recorded baseline.** Any future run requires explicit network authorization and an allowed existing cache. Therefore this document reports no E1 real-model timing, memory result, output, or product baseline.

## Actually executed release baseline

### Environment and workload

The final corrected normal synthetic baseline and the four selected Criterion targets were executed after the harness hardening described in this change, with the identity below. The working tree was **dirty** at measurement time; the recorded `HEAD`/tree identify the committed base while the final report and diff identify the uncommitted Phase 10 source.

| Field | Recorded value |
|---|---|
| Git `HEAD` | `f61a0fadd2311a53e1bce55094f886e3465b0c95` |
| Git `HEAD^{tree}` | `1bee6fa25f8b4819ac68d02cc10324f0f1848e9e` |
| Working tree | dirty |
| Rust / Cargo / LLVM / Criterion | `rustc 1.96.1`; `cargo 1.96.1`; LLVM `22.1.2`; Criterion `0.8.2` |
| Target / profiles / features | `x86_64-unknown-linux-gnu`; synthetic runner `release`; Criterion targets Cargo `bench` (optimized); no package features (`[]`) |
| OS / kernel | Linux; `7.1.4-arch1-1` |
| CPU | AMD Ryzen 9 5950X; 16 physical cores; 32 logical CPUs |
| RAM | 33,556,652,032 bytes |
| Runtime cycles | warmup 1; samples 3 |
| Download-free E1 startup cycles | warmup 1; samples 3 |
| Synthetic generation | greedy; seed 17; prompt 2 tokens; generate 6 tokens |
| Checked prefill / post-first window | 4 prompt tokens; 4 generated tokens |

All recorded thread-control environment variables were unset: `CARGO_BUILD_JOBS`, `RAYON_NUM_THREADS`, `OMP_NUM_THREADS`, `OMP_THREAD_LIMIT`, `MKL_NUM_THREADS`, `OPENBLAS_NUM_THREADS`, `VECLIB_MAXIMUM_THREADS`, `NUMEXPR_NUM_THREADS`, `RUST_TEST_THREADS`, and `CANDLE_NUM_THREADS`.

### Synthetic normal-runner results

Intervals are the minimum and maximum of the three recorded samples; medians are the recorded three-sample medians.

| Measurement | Sample interval | Median / throughput evidence |
|---|---:|---:|
| E0 worker start | 33.571–41.041 µs | 35.651 µs; samples `41.041`, `33.571`, `35.651` µs |
| Model load | 0.087352–0.091073 ms | 0.088252 ms |
| Checked four-token prefill | 5.342635–5.534649 ms | 5.526169 ms |
| Checked-prefill throughput | 722.720–748.694 prompt tokens/s | 723.829 prompt tokens/s |
| Submission to first token at pull | 5.922168–5.983590 ms | 5.979990 ms |
| Four-token post-first proxy | 21.103904–21.171285 ms | 21.110794 ms |
| Post-first proxy throughput | 188.935–189.538 tokens/s | 189.477 tokens/s |
| Controlled no-pull hold | 100.060398–100.061698 ms | samples `100.060588`, `100.060398`, `100.061698` ms; controlled setup, no threshold |
| Backpressure recovery to next token | 1.056394–1.058274 ms | 1.057325 ms |
| Cancellation to observable `Terminal` | 1.058995–1.060235 ms | 1.060145 ms |
| Cancellation to observable `Released` | 1.059035–1.060275 ms | 1.060185 ms; samples `1.060185`, `1.059035`, `1.060275` ms |
| Model unload | 0.016280–0.016371 ms | 0.016330 ms |
| Clean E0 shutdown and join | 0.034441–0.036590 ms | 0.034721 ms |
| Download-free `ApplicationRuntime` start | 35.173612–37.534997 ms | 37.154378 ms |
| Download-free `ApplicationRuntime` shutdown | 0.056301–1.092955 ms | 1.082565 ms; samples `0.056301`, `1.082565`, `1.092955` ms |

The low first E1 shutdown sample is retained rather than hidden. The four-token post-first result is explicitly a short synthetic integration proxy, **not production steady-state throughput**. All nine generation operations across the three cycles reached observable matching `Terminal` and `Released` states with no pending or exhausted cleanup; every cancellation emitted two tokens.

### Deterministic accounting and sampled RSS

After load, public E0 accounting reported 4,800 host weight bytes, 4,800 host working bytes, and 64 cache bytes per token. During output backpressure it reported one active request and one generation workspace, 4,800 host weight bytes, 6,320 host working bytes, 128 cache bytes per token, and a 240-host-byte generation workspace. After every `Released` record, accounting returned to the model-only state. After every unload, all accounting fields were exactly zero with no model, request, generation workspace, pending/exhausted cleanup, or maintenance error.

Recorded `VmRSS` trends, in sample-cycle order, were:

| Checkpoint | Sample 1 | Sample 2 | Sample 3 |
|---|---:|---:|---:|
| E0 before load | 8,654,848 bytes | 8,671,232 bytes | 8,675,328 bytes |
| E0 during backpressure | 8,671,232 bytes | 8,675,328 bytes | 8,679,424 bytes |
| E0 after unload | 8,671,232 bytes | 8,675,328 bytes | 8,679,424 bytes |
| E1 before application start | 13,107,200 bytes | 13,164,544 bytes | 13,172,736 bytes |
| E1 after application shutdown | 13,164,544 bytes | 13,172,736 bytes | 13,176,832 bytes |

The non-decreasing RSS after exact runtime-accounting cleanup is consistent with allocator and operating-system page retention; it is not evidence that E0 still owns a model or request, and these three samples alone do not establish a leak.

Linux RSS comes from sampled `/proc/self/status` `VmRSS`; `VmHWM` is also recorded. Both are process-wide rather than per-model. They include executable pages, stacks, workers, allocator arenas/caches, mappings, libraries, filesystem effects, warmups, and prior cycles. Sampling can miss transient peaks, while `VmHWM` is monotonic for the process. Neither value identifies Candle/native allocations or device memory. On non-Linux systems these fields are `null`.

Public `RuntimeSnapshot` accounting is deterministic admission/ownership accounting and can include planned or reserved footprints. It is intentionally separate from OS residency: a change in one does not imply an equal change in the other. Neither runtime harness installs an allocation-counting global allocator.

### Actually executed Criterion results

| Target | Criterion-reported time interval | Throughput interval | Samples and outliers |
|---|---:|---:|---|
| Runtime hosted prefill, 4 tokens (`e0_hosted_checked_prefill/4_tokens`) | [5.0955, 5.1104] ms | [782.71, 785.00] elem/s | 10 samples; no outliers reported |
| Runtime hosted decode after two-token prefill (`e0_hosted_incremental_decode/1_token_after_2_token_prefill`) | [5.0474, 5.0890] ms | [196.50, 198.12] elem/s | 10 samples; 1 high severe outlier |
| Sampling `sample_only/default_top_k_top_p/32768` | [77.211, 77.228] µs | [424.30, 424.39] Melem/s | 100 samples; 6 outliers (3 low mild, 2 high mild, 1 high severe) |
| Sampling `restore_and_sample/default_top_k_top_p/32768` | [78.288, 78.302] µs | [418.48, 418.56] Melem/s | 100 samples; 9 outliers (3 low mild, 4 high mild, 2 high severe) |

Exactly these four Criterion targets were statistically executed. Every other sampling case/size/boundary and all three stop-matching targets compiled but were **not statistically executed** for this baseline. No result is inferred or invented for those targets.

## Historical sampling baseline — 2026-07-22

This is preserved as historical evidence from the pre-matrix, restoration-inclusive default-policy benchmark at 32,768 logits. It is not a current-tree Phase 10 matrix result.

Environment:

- CPU: AMD Ryzen 9 5950X, 16 cores and 32 hardware threads;
- target: `x86_64-unknown-linux-gnu`;
- compiler: Rust 1.96.1, LLVM 22.1.2;
- profile: Cargo `bench` optimized profile;
- Criterion: 0.8.2, 100 measured samples.

The prepared sampler reserved 128 KiB each for mutable F32 logits, U32 candidate indices, and U32 repetition epoch state: 384 KiB total, excluding history. Repetition penalty was one, so the epoch table was not mutated; active mutable storage was 256 KiB for logits and indices. A separate 128 KiB baseline slice restored overwritten logits inside the measured boundary. All vectors were allocated before measurement.

Observed interval:

```text
time:       80.726 µs to 82.028 µs per sample
throughput: 399.48 Melem/s to 405.92 Melem/s
```

Six measurements were classified as high outliers. No source optimization is justified from this historical baseline alone; a proposed change must be compared under equivalent conditions and should include profiler evidence identifying the cost.

## Metadata, output, and artifact policy

The normal runner serializes exactly one stable schema-versioned serde JSON document to **stdout** and writes progress plus a compact human summary to **stderr**. It never emits generated token IDs, decoded text, environment-wide values, access tokens, or other secrets. Only the fixed thread-control variables listed above are recorded.

Metadata includes Git `HEAD`, `HEAD^{tree}`, and a dirty boolean without dirty file names; Rust/Cargo/LLVM/Criterion details; target, profile, and features; OS/kernel/CPU/core/RAM facts; fixed thread environment; fixture or immutable product identity; workload and sampling settings; and effective cache/network policy.

The runner writes no result file itself. Download-free temporary redb/cache state is created below root `target/runtime-benchmarks` and deleted. Captured JSON, Criterion samples/HTML, reports, flamegraphs, profiler data, heap dumps, compiler intermediates, and model caches must remain under root `target`; generated output must never be written into source directories. Criterion uses `target/criterion`.

Representative commands, not executed while editing this document:

```text
mkdir -p target/runtime-benchmarks
cargo run --release --locked -p runtime-benchmarks --bin baseline -- \
  --mode synthetic --warmup 1 --cycles 3 \
  > target/runtime-benchmarks/synthetic.json
cargo bench --locked -p sampling --bench sampling_pipeline
cargo bench --locked -p runtime-benchmarks --bench runtime
```

For the pinned E1 mode, use an existing explicit cache under shared root `target/` or outside the repository, ensure `HF_HUB_OFFLINE` is not `1`, and opt into network access unmistakably:

```text
mkdir -p target/runtime-benchmarks/hf-cache
cargo run --release --locked -p runtime-benchmarks --bin baseline -- \
  --mode real-product --cache-dir target/runtime-benchmarks/hf-cache \
  --allow-network --warmup 1 --cycles 3
```

## Deferred candidates

These candidates remain deliberately unimplemented because no current evidence supports a separate benchmark or optimization decision:

- tokenizer encode: no identified implementation decision or representative profile;
- owned streaming decode: a stable public seam exists, but there is no measured bottleneck or current decision;
- context planner: no supported scale question or implementation decision;
- bounded output accumulator: token and text variants exist, and the system backpressure path answers the current question;
- isolated raw Candle prefill/decode: hosted E0 Criterion isolates the needed regression boundary without private access or duplicated fixture setup.

A future benchmark must name the decision it supports and explain why existing component or system evidence is insufficient. The recorded baseline, including its outliers, led to no optimization.
