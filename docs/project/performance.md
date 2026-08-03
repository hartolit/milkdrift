# Performance evidence

This document is the canonical owner of evidence classes, benchmark methodology, measured boundaries, controlled environments, curated timing and memory results, limitations, and interpretation. Repeatable commands live in [validation](validation.md); package-local usage lives in [`benchmarks/runtime/README.md`](../../benchmarks/runtime/README.md); chronology lives in [execution history](../agent/execution/history.md).

## Evidence policy

Phase 10 created measurement and regression infrastructure; it made no production optimization and defines no portable wall-clock pass/fail threshold. Absolute timing varies with CPU frequency, thermals, scheduler activity, compiler version, and background work. Comparisons require the same host, toolchain, profile, fixture, workload, and benchmark configuration.

Hard harness timeouts stop hangs only. Lifecycle, fixture, identity, output, cleanup, accounting, or join mismatches fail a run; elapsed time alone does not.

| Evidence class | Current surface | What it establishes | What it does not establish |
|---|---|---|---|
| Deterministic allocation | Harness-free `domain-contracts` allocation executable and the sampling allocator test | Project-global allocator behavior within the named preallocated regions | Candle/native/driver/OS/device allocation behavior |
| Sampling component | `crates/domain/sampling/benches/sampling_pipeline.rs` | Comparative timing of public sampling and stop matching with caller-owned prepared storage | E0/E1 latency, product throughput, or allocation attribution |
| Hosted E0 component-like | `benchmarks/runtime/benches/runtime.rs` | Public command submission through matching completion event, including bounded transport and dispatch | Raw Candle kernel timing, E1/product latency, RSS, or full-generation throughput |
| Synthetic system/integration | `benchmarks/runtime` normal `baseline` binary | Download-free hosted-E0 lifecycle/output/accounting/RSS observations and fresh E1 start/shutdown cycles | Product-model speed or quality, representative scale, steady-state serving, or device memory |
| Compile-only | Workspace benchmark compilation | Target/API compatibility | Runtime correctness or performance |
| External real-product | `runtime-benchmarks` `external-baseline` binary | One exact local Candle/Hub/Safetensors/BF16/CPU product lifecycle and controlled direct-completion timing on Commit C | Model quality, other hosts/models/scalars, serving capacity, GPU behavior, or a regression threshold |

## Sampling methodology

### Matrix and correctness coverage

The implemented Cartesian product is:

```text
{sample_only, restore_and_sample}
  × {eight policy/history cases}
  × {8,192, 32,768, 131,072 logits}
```

That produces 48 statistical sampling targets plus three stop-matching targets. `crates/domain/sampling/tests/benchmark_matrix.rs` shares the case definitions and fixture builders with Criterion and executes every combination once for correctness without running statistics.

The eight sampling cases are:

- `greedy`;
- `default_top_k_top_p`;
- `min_p_0_05_full_vocabulary`;
- `repetition_disabled_history_256`;
- `repetition_enabled_empty`;
- `repetition_short_unique_8`;
- `repetition_medium_unique_64`;
- `repetition_repeated_heavy_256`.

The three stop targets are token hit, a four-token last-pattern hit among eight patterns, and an eight-pattern miss, each with a 128-token generated history.

Each sampling fixture uses deterministic pseudo-random logits bounded to `[-8, 8]`, seed 29, one prepared `Sampler`, and caller-owned logits, indices, repetition state, and history. Fixture construction, allocation, sampler construction, and capacity reservation occur before measurement. `Throughput::Elements` means one vocabulary-sized logit set, not generated tokens.

### Sampling boundaries

| Prefix | Exact timed boundary |
|---|---|
| `sample_only/<case>/<vocabulary>` | Baseline-to-working-logit restoration finishes before timing. The public workspace view, `Sampler::sample`, result checking, and `black_box` are timed. |
| `restore_and_sample/<case>/<vocabulary>` | Timing begins before baseline-to-working-logit restoration and includes the complete `sample_only` boundary. |
| `stop_matching/...` | Only public `match_stop_suffix` plus `black_box` is timed; inputs are prepared before timing. |

The measured loops invoke no allocation API and reuse capacities, but Criterion is not an allocator counter. The sampling allocation test remains the deterministic gate for 64 preallocated calls with repetition enabled.

## Runtime methodology

### Synthetic normal runner

The recorded workload uses one warmup cycle and three sample cycles. Warmups remain in the JSON report for auditability but are excluded from the curated intervals below.

Each hosted-E0 cycle starts a fresh worker, verifies and loads the deterministic fixture, runs checked prefill and three generation scenarios, unloads, performs ticketed shutdown, waits for completion, and joins. Any pending or exhausted cleanup, degraded model, maintenance error, residual request/workspace, unexpected shutdown cleanup, or failed join invalidates the cycle.

| Measurement | Exact boundary and required interpretation |
|---|---|
| E0 worker start | `start_hosted_runtime` through returned client/thread handles; the readiness snapshot follows outside timing. |
| Model load | `LoadModel` submission through matching `ModelLoaded`; source construction precedes timing and validation follows it. |
| Checked prefill | `Prefill` submission through matching `PrefillCompleted` for four prompt tokens; model/request setup and logits allocation are excluded. |
| Submission to first token | `Generate` submission through first token observed at public `pull_token_output`; request construction is excluded. |
| Post-first proxy | First public-token observation through four additional public tokens; this short synthetic window is not steady-state throughput. |
| Backpressure | A separately recorded fixed 100 ms no-pull hold, then the freeing pull through observation of the next token; eventual terminal/release and clean accounting are mandatory. |
| Cancellation | `CancelRequest` submission to separate acknowledgement, observable cancellation terminal state, and observable release; progress is established first. |
| Model unload | `UnloadModel(RejectIfBusy)` submission through successful unload after all requests release; the post-unload snapshot must be empty. |
| E0 shutdown | `Shutdown` submission through matching event and successful join; event, join, and total durations are recorded. |
| E1 lifecycle | Separate `ApplicationRuntime::start` and bounded `shutdown` calls on fresh download-free instances; no Hub resolution, model load, or generation occurs. |

Each E0 cycle records public runtime/model accounting and Linux process memory at eight checkpoints. Each E1 cycle records process memory before start, after start, and after shutdown.

### Hosted E0 Criterion targets

The runtime Criterion group uses a 500 ms warmup, a two-second measurement window, 10 samples, and no regression threshold.

| Target | Timed | Excluded |
|---|---|---|
| `e0_hosted_checked_prefill/4_tokens` | Public `Prefill` submission through matching `PrefillCompleted`, including bounded transport, E0 dispatch/checking, Candle execution, event publication, and receive | Fixture/source construction, worker start, load, request/sequence/prompt/logits setup, semantic validation, completion, unload, shutdown, and join |
| `e0_hosted_incremental_decode/1_token_after_2_token_prefill` | Public `Decode` submission through matching `DecodeCompleted` after a prepared two-token prefix | Request/sequence creation, setup prefill, token/logits setup, semantic validation, completion, and lifecycle setup/teardown |

`iter_custom` accumulates only the named boundary. The vocabulary-sized logits vector is allocated before measurement and returned from each event for reuse. Internal Candle tensor/cache work contributes elapsed time but is not attributed.

### Fixture, output, and artifact contract

The runtime harness references the project-authored deterministic fixture under `crates/runtime/inference-runtime/tests/fixtures/candle-llama` without copying it. Before setup it verifies regular files, exact byte sizes, recomputed SHA-256 values, parsed configuration, and the loaded public descriptor.

| File | Size | Required SHA-256 |
|---|---:|---|
| `config.json` | 360 bytes | `052b5c325859dc723ed0825f711950cbff112a140239953273cebacdb36afdd0` |
| `model.safetensors` | 4,800 bytes | `cc4798af93488b4fb2ae0548c2b28ace600521732b52023a7786c3227d72d672` |

The fixture is Candle / Llama / Safetensors / unquantized F32 with vocabulary and context capacity 16. It contains no trained or externally sourced weights and proves integration only.

The runner writes one schema-versioned JSON document to stdout and compact progress/summary information to stderr. It records allowlisted Git, toolchain, host, workload, fixture, lifecycle, accounting, and process-memory data; it excludes generated text/token IDs, credentials, secrets, and broad environment dumps.

Raw JSON, Criterion reports, profiles, caches, and compiler output remain beneath root `target` or outside the repository. The runner writes no result file itself.

## Commit A controlled baseline

### Exact code-under-test identity

| Field | Value |
|---|---|
| Commit A | `efcd36e320a97d61d3f982619fee182410c514df` |
| Commit A tree | `f80c5d6c746376df81d7ac8e7281ac9736e44d88` |
| Working tree during validation and measurement | Clean; generated output existed only beneath ignored root `target/` |
| Dedicated Cargo target | `target/phase10-final` |
| Captured synthetic report | `target/phase10-evidence/synthetic.json` |

The follow-on evidence commit (Commit B) changes Markdown only. These measurements therefore apply directly to Commit A’s executable tree; they are not represented as a fresh measurement of Commit B. Commit B’s post-commit repository gate is separate validation evidence.

### Controlled environment and workload

| Field | Recorded value |
|---|---|
| Rust / Cargo / LLVM / Criterion | `rustc 1.96.1`; `cargo 1.96.1`; LLVM `22.1.2`; Criterion `0.8.2` |
| Host target | `x86_64-unknown-linux-gnu` |
| Profiles | Synthetic runner `release`; Criterion Cargo `bench` optimized profile |
| OS / kernel | Linux / `7.1.4-arch1-1` |
| CPU | AMD Ryzen 9 5950X; 16 physical cores; 32 logical CPUs |
| RAM | 33,556,660,224 bytes |
| Thread controls | All eight allowlisted runtime thread-control variables were unset |
| Synthetic cycles | 1 warmup; 3 recorded samples |
| Fixture | Project-authored deterministic Candle/Llama/Safetensors/F32 fixture; vocabulary/context 16 |
| Generation workload | Greedy; two-token prompt; six-token first-token scenario; four-token post-first window |
| Checked prefill | Four prompt tokens |
| Backpressure | Four-token generation limit; fixed 100 ms no-pull hold |
| Cancellation | Twelve-token limit; fixed 25 ms pre-cancel hold; two generated tokens observed in every sample |

### Synthetic timing results

Intervals are the minimum and maximum of the three recorded sample cycles. Medians are the middle recorded values after ordering.

| Measurement | Sample interval | Median / derived evidence |
|---|---:|---:|
| E0 worker start | 24.270–26.901 µs | 26.161 µs |
| Model load | 0.095222–0.103892 ms | 0.098103 ms |
| Checked four-token prefill | 5.123713–5.233256 ms | 5.138244 ms |
| Checked-prefill throughput | 764.343–780.684 prompt tokens/s | 778.476 prompt tokens/s |
| Submission to first token at pull | 5.900052–5.911632 ms | 5.904922 ms |
| Four-token post-first proxy | 20.005891–20.056412 ms | 20.052802 ms |
| Post-first proxy throughput | 199.437–199.941 tokens/s | 199.473 tokens/s |
| Controlled no-pull hold | 100.044964–100.059085 ms | 100.058516 ms; controlled setup, not a threshold |
| Backpressure recovery to next token | 1.057125–1.058826 ms | 1.057425 ms |
| Cancellation acknowledgement | 1.058355–1.058845 ms | 1.058476 ms |
| Cancellation to observable terminal state | 1.058545–1.058975 ms | 1.058616 ms |
| Cancellation to observable release | 1.058585–1.059015 ms | 1.058646 ms |
| Model unload | 12.870–17.551 µs | 17.360 µs |
| Clean E0 shutdown and join | 32.041–49.351 µs | 35.251 µs |
| Download-free `ApplicationRuntime` start | 35.612696–53.880535 ms | 47.204385 ms |
| Download-free `ApplicationRuntime` shutdown | 0.048151–1.089336 ms | 1.081216 ms |

The low first E1 shutdown sample is retained rather than hidden. The post-first result remains a short synthetic integration proxy, not representative steady state.

All nine generation operations across the three sample cycles reached matching terminal and released states. Every cancellation produced two tokens. No sample snapshot contained pending/exhausted cleanup or a maintenance error.

### Deterministic accounting and sampled RSS

After load, public E0 accounting reported 4,800 host weight bytes, 4,800 host working bytes, and 64 cache bytes per token. During output backpressure it reported one active request, one generation workspace, 4,800 host weight bytes, 6,320 host working bytes, 128 cache bytes per token, and a 240-host-byte generation workspace.

After each checked-prefill, first-token/proxy, backpressure, and cancellation release, accounting exactly matched the model-only after-load state. After every unload, model/request/workspace/cleanup/maintenance accounting and all reserved footprints were exactly zero, with no loaded model.

Recorded Linux `VmRSS` values, in sample-cycle order, were:

| Checkpoint | Sample 1 | Sample 2 | Sample 3 |
|---|---:|---:|---:|
| E0 before load | 7,864,320 bytes | 7,876,608 bytes | 7,888,896 bytes |
| E0 during backpressure | 7,876,608 bytes | 7,880,704 bytes | 7,892,992 bytes |
| E0 after unload | 7,876,608 bytes | 7,888,896 bytes | 7,892,992 bytes |
| E1 before application start | 11,988,992 bytes | 12,046,336 bytes | 12,058,624 bytes |
| E1 after application shutdown | 12,046,336 bytes | 12,058,624 bytes | 12,062,720 bytes |

RSS comes from sampled Linux `/proc/self/status` `VmRSS`; the report also records `VmHWM`. These values are process-wide and include executable pages, workers, stacks, libraries, mappings, allocator arenas/caches, warmups, and prior cycles. Sampling can miss transient peaks, and `VmHWM` is monotonic. Neither value attributes Candle/native resources or device memory.

The small non-decreasing RSS trend after exact accounting cleanup is consistent with allocator and operating-system page retention. Three samples do not establish a leak. Public runtime accounting is deterministic ownership/admission evidence and is intentionally distinct from OS residency.

### Focused Criterion results

Exactly these four targets were statistically executed on Commit A. Every other sampling matrix target and all stop-matching targets were compile-only for this baseline; correctness was covered by the one-shot matrix test.

| Target | Criterion time interval | Throughput interval | Samples and outliers |
|---|---:|---:|---|
| `e0_hosted_checked_prefill/4_tokens` | [5.1632, 5.1720] ms | [773.40, 774.71] elem/s | 10 samples; no outliers reported |
| `e0_hosted_incremental_decode/1_token_after_2_token_prefill` | [5.0015, 5.0872] ms | [196.57, 199.94] elem/s | 10 samples; no outliers reported |
| `sample_only/default_top_k_top_p/32768` | [78.075, 78.313] µs | [418.42, 419.70] Melem/s | 100 samples; 12 outliers: 2 low mild, 2 high mild, 8 high severe |
| `restore_and_sample/default_top_k_top_p/32768` | [81.265, 81.287] µs | [403.11, 403.22] Melem/s | 100 samples; 5 outliers: 1 low mild, 4 high mild |

These intervals are local comparative evidence, not shared-CI thresholds or product-model latency/throughput.

## Historical Phase 4 external smoke

This section preserves curated product-path observations from the 2026-07-25 Phase 4 baseline. It is historical evidence for that older tree, not a current-tree Phase 10 external baseline.

| Field | Historical value |
|---|---|
| Repository | `neubla/tiny-random-LlamaForCausalLM` |
| Revision | `39ca1f8a1fc940377c5cb49a21aff73bb99b52f5` |
| Expected architecture | `LlamaForCausalLM` / runtime `Llama` |
| Model SHA-256 | `49c20f32c6c597480fcaec5df2f86c645eabea765cbea1e67886dbae45e5c992` |
| Observed generation | Eight tokens through the pinned E0 path |

| Measurement | Historical result |
|---|---:|
| Model load duration | 0.005661 s |
| Time to first generated token | 0.060969 s |
| Decode throughput | 21.954 tokens/s |
| Cancellation latency | 0.045297 s |
| Model unload duration | 0.000380 s |
| RSS before load | 4,636 KiB |
| RSS after load | 11,116 KiB |
| RSS during generation | 14,088 KiB |
| RSS after unload | 10,412 KiB |

Elevated post-unload RSS was not treated as retained model ownership because allocator page retention can outlive resource release. The historical ownership evidence was released records, empty accounting, an empty post-unload snapshot, successful worker shutdown, and clean process exit.

## External product evidence

### Exact code-under-test and model identity

The authoritative external run executed the release `external-baseline` binary directly after building it on clean Commit C. The source/index remained clean before and after execution; generated state existed only beneath ignored root `target/`.

| Field | Recorded value |
|---|---|
| Commit C | `771c0de4d72565a6302ca60f3b6bafd8c807962b` |
| Commit C tree | `3d5b6ccc5ecc959de7cb370c1147f76e4cd32e3f` |
| Raw report | `target/phase10-evidence/external.json`; schema version 1; `dirty: false` |
| Repository | `TinyLlama/TinyLlama-1.1B-Chat-v1.0` |
| Requested revision | `fe8a4ea1ffedaf415f4da2f062534de366a451e6` |
| Resolved immutable commit | `fe8a4ea1ffedaf415f4da2f062534de366a451e6` |
| Composition | Candle / Hugging Face Hub / CPU / Safetensors / Llama / BF16 |
| Vocabulary / context / prefill capacities | 32,000 / 2,048 / 2,048 tokens |
| Upstream-declared license metadata | `apache-2.0`, from the pinned revision's [model card](https://huggingface.co/TinyLlama/TinyLlama-1.1B-Chat-v1.0/raw/fe8a4ea1ffedaf415f4da2f062534de366a451e6/README.md); this records the upstream declaration, not a broader legal conclusion |
| Explicit cache | `target/phase10-external-cache`; empty before resolution; populated only as an ignored generated artifact |

The initially empty cache means the 21.031913121-second resolution interval included first acquisition and local cache population. It is still the complete public E1 resolve interval—not a pure network-transfer benchmark—because it also includes Hub metadata handling plus artifact/tokenizer/configuration validation and local I/O.

### Controlled environment and workload

| Field | Recorded value |
|---|---|
| Rust / Cargo / LLVM | `rustc 1.96.1`; `cargo 1.96.1`; LLVM `22.1.2` |
| Host target / profile | `x86_64-unknown-linux-gnu`; `release` |
| OS / kernel | Linux / `7.1.4-arch1-1` |
| CPU | AMD Ryzen 9 5950X; 16 physical cores; 32 logical CPUs |
| RAM | 33,556,660,224 bytes |
| Thread controls | All eight allowlisted runtime thread-control variables were unset |
| Resource preflight | Approximately 19 GiB available memory and 187 GiB free disk; no `cargo`, `rustc`, or external-model process active before model execution |
| Runtime ownership | One `ApplicationRuntime`, one resolved/loaded model, and sequential requests only |
| Chat proof | Message identifier `tinyllama-local-inference-chat-proof-v1`; SHA-256 `85edb99e63b9fcedf242043a55ba131e0ca9a2bfa7349ce6a1da81f90dbecd0e`; 73 bytes; maximum 24 generated tokens; exact profile-owned EOS policy |
| Direct completion | Prompt identifier `deterministic-resource-cleanup-completion-v1`; SHA-256 `a4b6e5d148e9f1b2b2cf962ec4ce5a0e4f40e21866b7b602cdeb80b1f3a6f5a4`; 93 bytes; 1 warmup then 3 measured samples; exactly 32 generated tokens each |
| Sampling | Temperature 1.0; top-k 1; top-p 1.0; min-p 0.0; repetition penalty 1.0; repetition window 0; fixed seed 39 |
| Direct termination controls | No EOS tokens and no textual stop sequences; token-limit completion required |

No decoded model text or generated token IDs were retained in the report or curated evidence.

### Compatibility and lifecycle results

The compatible chat submission was accepted through the public conversation API. It observed matching request identity, `GenerationStarted`, non-empty decoded output, terminal and released states, and the matching terminal application event. It completed by end-of-sequence with 30 prompt tokens, 6 generated tokens, and 27 decoded bytes. The user record and completed active assistant attempt were validated after release, then public conversation clear left conversation and context diagnostics empty.

The direct warmup reached token limit with 17 prompt tokens, exactly 32 generated tokens, 150 decoded bytes, matching terminal/released/event outcomes, and clean release. All three measured requests then produced the same usage and decoded-byte counts sequentially against the already loaded model. No request entered cleanup-pending or cleanup-exhausted state, no worker disconnected, and no active generation remained after release.

`ModelUnloadBehavior::RejectIfBusy` completed with zero cancelled requests and left no public loaded model or active generation while both workers remained available. Explicit bounded shutdown returned successfully, left Hub and inference unavailable with no loaded model or active generation, and the temporary redb workspace was removed. Successful E1 lifecycle events and public state establish application-visible ownership cleanup; process RSS is a separate coarse observation.

### External timing results

Durations are submission/call-to-observation intervals from the one recorded run. Operational deadlines were hang bounds only and no wall-clock acceptance threshold was applied. The warmup is excluded from measured timing summaries.

| Lifecycle measurement | Recorded duration |
|---|---:|
| `ApplicationRuntime::start` | 0.053289329 s |
| Resolve plus first acquisition/cache population | 21.031913121 s |
| Load submission through matching `ModelLoaded` | 4.977881531 s |
| `RejectIfBusy` unload submission through matching successful unload | 0.201129165 s |
| Explicit bounded shutdown call through successful return | 0.021589180 s |

| Direct sample | To `GenerationStarted` | To first non-empty decoded output | To terminal application event | To observable release | Effective generated throughput |
|---:|---:|---:|---:|---:|---:|
| 1 | 0.014357342 s | 0.427737818 s | 8.554832290 s | 8.554831990 s | 3.740576 generated tokens/s |
| 2 | 0.014343812 s | 0.416876410 s | 8.332556224 s | 8.332555974 s | 3.840358 generated tokens/s |
| 3 | 0.013663945 s | 0.415999918 s | 8.352163915 s | 8.352163495 s | 3.831343 generated tokens/s |
| **Median** | **0.014343812 s** | **0.416876410 s** | **8.352163915 s** | **8.352163495 s** | **3.831343 generated tokens/s** |

Here “first output” means the first non-empty decoded UTF-8 fragment observed at the public E1 output boundary. Effective throughput is exactly 32 generated tokens divided by submission-to-release duration. No post-first-output throughput is reported because the public output record does not identify how many generated tokens had been consumed at the first decoded fragment.

### Process-memory observations

The runner sampled Linux `/proc/self/status` at lifecycle checkpoints. Each measured sample's memory was captured after release.

| Checkpoint | `VmRSS` | Observed `VmHWM` field |
|---|---:|---:|
| Before application start | 5,332,992 bytes | 5,332,992 bytes |
| After application start | 9,842,688 bytes | 9,842,688 bytes |
| After resolution | 602,669,056 bytes | 1,505,689,600 bytes |
| After load | 5,649,702,912 bytes | 5,649,702,912 bytes |
| After warmup release | 5,666,664,448 bytes | 5,666,664,448 bytes |
| After measured sample 1 release | 5,666,672,640 bytes | 5,666,672,640 bytes |
| After measured sample 2 release | 5,666,676,736 bytes | 5,666,676,736 bytes |
| After measured sample 3 release | 5,666,680,832 bytes | 5,666,680,832 bytes |
| After unload | 2,498,686,976 bytes | 5,661,573,120 bytes |
| After shutdown | 2,088,587,264 bytes | 5,661,573,120 bytes |

The sampled RSS dropped by 3,167,993,856 bytes from the final measured release to post-unload and by another 410,099,712 bytes after shutdown. Remaining RSS is not treated as retained model/runtime ownership: it includes the whole process, executable/library mappings, allocator arenas/caches, stacks, and other host state. `/proc` RSS/HWM accounting and checkpoint sampling are coarse and can miss or approximate transient residency; the highest observed reported field was 5,666,680,832 bytes. There is no device-memory evidence because CPU is the only supported device.

### Interpretation and limitations

This closes the missing current-tree CPU product baseline and is sufficient to make Phase 11 ready for a separate activation decision. It does not implement or evidence GPU capability. It is one local run on one desktop host, one model/revision/scalar, one initially empty cache, one chat proof, one warmup, and three short direct completions. Background desktop and editor services remained present, so these values are controlled local evidence rather than isolated laboratory results.

The result supports product-path compatibility, deterministic workload comparison, and lifecycle regression investigation on a comparable host. It does not establish model quality, concurrent or steady-state serving throughput, another model/format/engine/device, allocator attribution, cross-host portability, a memory leak/non-leak conclusion, or a production optimization target. No production optimization is justified by this baseline alone.

## Interpretation and deferred work

- No production optimization is justified by this single local baseline.
- Tokenizer encode/streaming decode, context planning, output accumulation, and isolated Candle kernels remain deferred until a named decision and profile show that current surfaces are insufficient.
- Synthetic timings are useful for regression investigation, lifecycle confidence, and harness comparison—not product claims.
- A proposed optimization should compare equivalent before/after trees on this controlled host and include a profile or other evidence identifying the cost.
