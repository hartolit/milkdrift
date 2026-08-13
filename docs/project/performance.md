# Performance evidence

This document is the canonical owner of evidence classes, benchmark methodology, measured boundaries, controlled environments, curated timing and memory results, limitations, and interpretation. Repeatable commands live in [validation](validation.md); package-local usage lives in [`benchmarks/runtime/README.md`](../../benchmarks/runtime/README.md); chronology lives in [execution history](../agent/execution/history.md).

## Evidence policy

Phase 10 created measurement and regression infrastructure; it made no production optimization and defines no portable wall-clock pass/fail threshold. Phase 11 adds controlled product evidence for the exact CPU and CUDA compositions documented below; it likewise defines no portable threshold or generic device-family claim. Absolute timing varies with CPU frequency, thermals, scheduler activity, compiler version, and background work. Comparisons require the same host, toolchain, profile, fixture, workload, and benchmark configuration.

Hard harness timeouts stop hangs only. Lifecycle, fixture, identity, output, cleanup, accounting, or join mismatches fail a run; elapsed time alone does not.

| Registry ID | Public boundary and evidence class | Required environment/artifact | Output/schema owner | Does not establish |
|---|---|---|---|---|
| `correctness.allocation` | Harness-free domain/sampling allocator correctness | Named preallocated regions; default host tests | Owning crate tests | Candle/native/driver/OS/device allocation behavior |
| `sampling.pipeline` | Public `Sampler::sample` and prepared restore boundary; synthetic component performance | Deterministic caller-owned fixture; host bench profile | Criterion target `sampling/sampling_pipeline` | E0/E1 latency, product throughput, or allocation attribution |
| `sampling.stop-suffix` | Public `match_stop_suffix`; synthetic component performance | Deterministic caller-owned inputs; host bench profile | Criterion target `sampling/sampling_pipeline` | Generation/product latency |
| `e0.checked-prefill` | Public E0 submission through matching checked-prefill completion; synthetic hosted-E0 performance | Committed Candle fixture; CPU bench profile | Criterion target `runtime-benchmarks/runtime` | Raw Candle kernel timing, E1 latency, RSS, or generation throughput |
| `e0.incremental-decode` | Public E0 decode after untimed two-token prefill; synthetic hosted-E0 performance | Committed Candle fixture; CPU bench profile | Criterion target `runtime-benchmarks/runtime` | Product latency or full-generation throughput |
| `e0.lifecycle-process` | Hosted E0 load/generate/backpressure/cancel/unload/shutdown, bounded actual-transaction loader observation, direct accounting, and RSS/HWM; synthetic/process evidence | Release CPU binary; committed fixture; no network | `benchmarks/runtime/src/report.rs`, synthetic schema 6 | Product-model speed/quality, representative scale, or physical device residency |
| `e1.cold-start-process` | Fresh E1 start/shutdown and process RSS; process sampling | Empty temporary redb state; no network | Synthetic schema 6 application-lifecycle records | Resolution, load, or generation behavior |
| `e1.tinyllama-product` | Public E1 resolve/load/chat/direct/cancel/unload/shutdown plus actual scalar/device; external product/process/device evidence | Clean commit, explicit cache/network, fixed TinyLlama revision, CPU or exact reviewed CUDA row | `benchmarks/runtime/src/external/report.rs`, external schema 6 | Mixed-checkpoint evidence, model quality, generic NVIDIA support, process-attributed VRAM, leak proof, or a portable threshold |
| `cuda.fixture-correctness` | Dedicated adapter/E0/E1 CUDA suites; hardware correctness | Download-free fixture; exact self-hosted RTX 5070 Ti matrix | GitHub/local test logs | Product performance or external-model compatibility |
| `compile.maintained-benches` | Exact registered Cargo bench targets; compile-only | Locked graph; bench profile | `xtask` manifest registry | Runtime correctness or performance |

CI target size/free-space observations are infrastructure evidence and are recorded in [implementation status](implementation-status.md) and [execution history](../agent/execution/history.md), not interpreted as product performance. The Quality topology compiles `compile.maintained-benches` in its own fresh target and gives check, tests, Clippy, and rustdoc separate standard-runner filesystems; none of their artifact sizes is a runtime-memory, throughput, or latency measurement. Current local resource observations and the corresponding per-leg preflights are documented in [validation](validation.md#shared-cpu-quality-workflow). Hosted low-water observations remain pending an exact redesigned run.

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
| `config.json` | 382 bytes | `e30225f7b8cbeb18c6fe2e9f623e87bd5d7cec3e28dd7e23a3f36ee107c69c4d` |
| `model.safetensors` | 4,800 bytes | `cc4798af93488b4fb2ae0548c2b28ace600521732b52023a7786c3227d72d672` |

The fixture is Candle / Llama / Safetensors / unquantized F32 with vocabulary and context capacity 16. It contains no trained or externally sourced weights and proves integration only.

The runner writes one schema-versioned JSON document to stdout and compact progress/summary information to stderr. It records allowlisted Git, toolchain, host, workload, fixture, lifecycle, accounting, and process-memory data; it excludes generated text/token IDs, credentials, secrets, and broad environment dumps.

Raw JSON, Criterion reports, profiles, caches, and compiler output remain beneath root `target` or outside the repository. The runner writes no result file itself.

### Current synthetic schema 6 contract

The artifact-accelerator amendment advanced the normal synthetic report from schema 4 to schema 5 by attaching a bounded, feature-gated recorder to the one production Candle loader transaction that produces the hosted `ModelLoaded` event. Synthetic schema 6 removes only the recorder's duplicate plan and accepted-ownership projections: `prepared` is the sole serialized plan projection, `receipt` is the sole accepted E0 ownership record, and `loader` owns transaction-stage observations. No new controlled synthetic report is curated here, and no historical timing or RSS value is reinterpreted.

Each E0 cycle records three related but distinct evidence groups:

| Evidence | Meaning |
|---|---|
| `prepared.configuration_declared_scalar` | Optional configuration metadata describing producer intent; not proof of tensor homogeneity. |
| `prepared.observed_tensor_scalars` | Stable compact set observed from all selected Safetensors tensor headers. |
| `prepared.planned_execution_scalar` / `planned_execution_device` | Device-aware execution facts selected by the exact preparation inside the actual hosted transaction. |
| `prepared.exact_final_footprint` | Exact deterministic tensor ownership expected after successful load. |
| `prepared.loading_peak_footprint` | Separate component-wise deterministic admission peak while materializing/converting; it is not post-load ownership. |
| `receipt.actual_execution_scalar` / `actual_execution_device` | Actual loaded facts verified by E0 against its accepted transaction. |
| `receipt.reserved_footprint` | Direct E0 post-load ownership reservation, required to equal the exact final footprint. |
| `loader.preparation_duration_ns` / `materialization_duration_ns` | Adapter-internal phase durations within the actual hosted load; they are nested observations, not additions to public model-load time. |
| `loader.required_bytes_read` / `whole_file_verification_bytes_read` | Required tensor payload bytes and all Safetensors shard bytes read for whole-file identity verification by that transaction. |
| `loader.transfer_batches` / `loading_device_synchronizations` | Actual accelerator transfer batches and load-time device synchronization calls; the maintained CPU fixture records zero for both. |
| snapshot `process_memory` | Sampled whole-process RSS/HWM, independent of plan and ownership accounting. |

The successful-only normal writer cannot populate loader outcome/cleanup variants nontrivially, so schema 6 omits them; a failed lifecycle aborts instead of becoming a performance sample. The committed synthetic artifact remains homogeneous F32, so its truthful schema-6 facts are declared `F32`, observed `{F32}`, planned F32/CPU, and actual F32/CPU. Loader observation retains fixed-size state rather than a tensor/event log. Its instrumentation overhead is present in this benchmark build; it does not establish a production optimization or a speedup. Synthetic schemas 1–5 retain their historical meanings, including schema 4's removal of `cache_bytes_per_token` and schema 5's duplicate loader ownership projections; no legacy report parser was introduced.

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

### Current external schema-6 contract

The infrastructure-truth amendment advances the external report from schema 5 to **external schema 6**. It removes the observer's independent adapter preparation, which duplicated production policy and could warm page-cache/CUDA state before the timed public E1 load. It also removes invariant prose, derivable duplicate arrays, and success booleans whose false state already aborts the runner. Unit tests validate the schema, but no schema-6 CPU or CUDA product run is accepted here. Every Phase 10/11 timing and memory table below remains attributed to its original code, schema, and environment.

Schema 6 records:

- fixed repository/revision/license and Git/toolchain/system/cache provenance;
- optional configuration declaration from public E1 resolution;
- requested device, selected E1 device, actual loaded device, and actual execution scalar;
- raw startup, resolution, load, generation, cancellation, unload, shutdown, token/byte, and terminal measurements;
- sampled process RSS/HWM under `process_memory`;
- qualified whole-device CUDA total/free/used observations under `whole_device_cuda_memory`, never as process-attributed ownership; and
- retained-growth summaries computed from raw cycle-local observations, without claiming leak/non-leak proof.

External schema 6 deliberately does **not** serialize observed tensor sets, planned scalar/device, exact final/loading-peak footprints, or direct E0 reserved ownership. Those facts require an adapter preparation or direct E0 receipt/snapshot and are owned by synthetic E0 evidence and correctness suites. The external runner observes the public E1 product boundary rather than fabricating same-worker lower-layer access.

The pinned `TinyLlama/TinyLlama-1.1B-Chat-v1.0` revision remains the established homogeneous BF16 lifecycle/chat profile and is **not mixed-checkpoint evidence**. No suitable immutable, license-reviewed external mixed-dtype Llama profile with an auditable direct-completion procedure has been established. A missing network path or credential is an acquisition failure, not model incompatibility; only an acquired artifact that fails the reviewed product path can support an incompatibility finding.

External schemas 1–5 retain their historical field meanings. No legacy parser was added merely to reinterpret old reports.

### Historical post-Phase 11 external schema-3 observation contract

The sole device-parameterized external runner emitted schema version 3 on the recorded post-Phase 11 regression tree. That structural change did not revise the fixed model, revision, workload, lifecycle timing boundaries, cancellation boundary, or canonical Commit E measurements below. Schema version 2 remains historical evidence with its original field meanings.

Schema 3 records:

- `source_scalar` from explicit resolved and loaded E1 facts;
- `execution_scalar` from the independent public Candle `LoadPlan`, marked accepted only after the loaded E1 execution scalar matches;
- requested, selected-E1, and actual-loaded-E0 device identities separately;
- `accounted_footprint` as the independent accepted plan, explicitly not a physical-memory observation or same-worker E0 reservation snapshot;
- process RSS and whole-device CUDA total/free/used observations only in separate resource checkpoints;
- the exact count of safe Candle `discover_device` calls used by benchmark observation.

Each counted CUDA discovery constructs a temporary Candle CUDA device and cudarc context. Calls are bounded to cold environment, identity, and resource checkpoints and never occur per generated token. The runner retains this behavior because safe context reuse would require production ownership exposure or a new lower-level benchmark dependency; neither is justified for cold observation. This count is context-churn audit evidence, not a performance threshold. Whole-device memory remains non-process-attributed, and direct E0 fixture validation remains the separate exact-zero accounting owner.

The clean schema-3 CUDA regression on commit `7dd7a72565cfb976bf123ed664296e9332af0e70`, tree `766682d96b89a3e6fb4b0d14282e44e318244a56`, recorded exactly 51 safe discovery calls: two before lifecycle execution, 19 in the primary cycle, and 15 in each reduced stability cycle. The same report classified retained whole-device growth as not strictly monotonic. These facts validate the observation schedule and finite stability classifier only; they do not revise canonical performance numbers or establish a leak/non-leak conclusion.

### Historical Phase 11 controlled CPU-vs-CUDA product evidence

The schema-2 Commit E reports and exact curated tables below remain the canonical executed Phase 11 CPU/CUDA product evidence. The CPU and CUDA primary runs use the same BF16-source model and application workload, but they are product-path observations rather than precision-matched hardware microbenchmarks: CPU uses F32 execution, supported CUDA uses BF16 execution, and CUDA generation includes full-vocabulary-logit transfer for host F32 sampling.

#### Exact code, model, and report identity

| Field | Recorded value |
|---|---|
| Commit E | `411945e0fd53363f98609db21a43d757c4d9b506` |
| Commit E tree | `7099dcb5c9879190543d3afa5fde399a84d799df` |
| Repository state | Clean Commit E; both reports record `dirty: false` |
| CPU report | `target/phase11-evidence/cpu.json`; schema version 2 |
| CUDA report | `target/phase11-evidence/cuda.json`; schema version 2 |
| Model | `TinyLlama/TinyLlama-1.1B-Chat-v1.0` |
| Exact model revision | `fe8a4ea1ffedaf415f4da2f062534de366a451e6` |

No raw report body, generated model text, or generated token ID is reproduced here.

#### Controlled host and execution composition

| Field | Recorded value |
|---|---|
| CPU | AMD Ryzen 9 5950X; 16 physical cores; 32 logical CPUs |
| OS / kernel | Linux `7.1.4-arch1-1` |
| Rust / Cargo / LLVM | `rustc 1.96.1`; `cargo 1.96.1`; LLVM `22.1.2` |
| RAM | 33,556,647,936 B (31.25 GiB) |
| CUDA device | NVIDIA GeForce RTX 5070 Ti, ordinal 0 |
| Driver | `610.43.03` |
| CUDA toolkit / compiler | CUDA `13.3`; `nvcc V13.3.73` |
| Compute capability / build cap | CC 12.0; `CUDA_COMPUTE_CAP=120` |
| Device VRAM | 16,648,896,512 B |
| CPU product composition | Default build; explicit `cpu` device; BF16 source → F32 execution |
| CUDA product composition | Non-default `runtime-benchmarks/cuda -> application-runtime/cuda -> candle-backend/cuda`; explicit `cuda:0` device; BF16 source → BF16 execution |
| Sampling boundary | Host sampling; CUDA full-vocabulary logits transferred to host F32; no GPU sampling |

#### Controlled workload and boundaries

The CPU primary cycle and CUDA primary cycle each ran the same chat proof, one direct-completion warmup, three measured direct completions of exactly 32 generated tokens each, and user-requested cancellation after the first generation progress. The warmup is excluded from the measured sample table. CUDA then ran two reduced stability cycles, for three CUDA cycles total; the reduced cycles contribute the lifecycle and memory checkpoints curated below rather than another direct-sample set.

Operational deadlines were hang bounds only. No elapsed-time pass/fail threshold was applied. Cancellation, unload, and shutdown completed cleanly.

#### Lifecycle timing results

All values are seconds. The primary rows cover the common full workload; CUDA cycles 2 and 3 are the reduced stability cycles.

| Run | `ApplicationRuntime::start` | Resolve | Load | Unload | Shutdown |
|---|---:|---:|---:|---:|---:|
| CPU primary | 0.043196614 s | 0.350453311 s | 6.276900458 s | 0.191068334 s | 0.000180964 s |
| CUDA cycle 1 (primary) | 0.042087709 s | 0.283494348 s | 2.313752303 s | 0.010063483 s | 0.000116972 s |
| CUDA cycle 2 (reduced stability) | 0.042084268 s | 0.255933587 s | 1.740622140 s | 0.010064204 s | 0.000219314 s |
| CUDA cycle 3 (reduced stability) | 0.039309835 s | 0.246895773 s | 1.418942616 s | 0.010061673 s | 0.001800384 s |

#### Chat compatibility timing results

All intervals begin at chat submission and end at the named public E1 observation.

| Device | `GenerationStarted` | First decoded output | Terminal application event | Observable release |
|---|---:|---:|---:|---:|
| CPU | 0.016440457 s | 0.480356773 s | 1.859431995 s | 1.859433145 s |
| CUDA | 0.096552685 s | 0.247440335 s | 0.268494479 s | 0.268494949 s |

#### Direct 32-token completion results

“First output” is submission to the first non-empty decoded output at the public E1 boundary. “Release” is submission to observable release. Effective rate is exactly 32 generated tokens divided by release duration.

| Device | Sample | First output | Release | Effective generated rate |
|---|---:|---:|---:|---:|
| CPU | 1 | 0.427336524 s | 8.545338298 s | 3.744732 tokens/s |
| CPU | 2 | 0.426757627 s | 8.575551839 s | 3.731538 tokens/s |
| CPU | 3 | 0.416893491 s | 8.605083176 s | 3.718732 tokens/s |
| CPU | **Median** | **0.426757627 s** | **8.575551839 s** | **3.731538 tokens/s** |
| CUDA | 1 | 0.014379286 s | 0.156057848 s | 205.052168 tokens/s |
| CUDA | 2 | 0.014050340 s | 0.165905288 s | 192.881133 tokens/s |
| CUDA | 3 | 0.013828106 s | 0.155647781 s | 205.592395 tokens/s |
| CUDA | **Median** | **0.014050340 s** | **0.156057848 s** | **205.052168 tokens/s** |

On this exact run, CUDA's median first-output interval was about 30.4 times shorter and its median effective generated-token rate about 55.0 times higher than CPU. Those are same-workload local observations, not a portable speedup claim; execution dtype and transfer boundaries differ as documented above.

#### Cancellation results

Progress was established before cancellation. Both primary runs generated exactly one token and ended for user-requested cancellation.

| Device | Generation submission to progress | Cancel submission to acknowledgement | Cancel submission to terminal event | Cancel submission to release | Generated tokens | Terminal reason |
|---|---:|---:|---:|---:|---:|---|
| CPU | 0.426740446 s | 0.251451575 s | 0.262429696 s | 0.252368862 s | 1 | User-requested cancellation |
| CUDA | 0.013790885 s | 0.010059043 s | 0.020916542 s | 0.010860189 s | 1 | User-requested cancellation |

#### Accounted E0 footprint and E1 load contract

| Composition | Accounted weight bytes | Accounted host working bytes | Accounted cache bytes per token |
|---|---:|---:|---:|
| CPU / BF16 source → F32 execution | 4,400,239,728 B host weights | 2,200,119,864 B | 45,056 B |
| CUDA / BF16 source → BF16 execution | 2,200,119,864 B device weights | 2,200,119,864 B | 22,528 B |

This is an independent E0 plan plus the E1 accepted load contract, not a reservation snapshot from the same worker that ran the product workload. Exact zero-accounting evidence is owned by the direct E0 snapshot test.

#### Process RSS observations

The key CPU primary checkpoints were:

| Checkpoint | CPU `VmRSS` |
|---|---:|
| Pre-load | 36,708,352 B |
| Post-load | 4,469,198,848 B |
| Peak sampled | 4,489,629,696 B |
| Post-unload | 901,120,000 B |
| Owner-drop | 896,720,896 B |

The key CUDA checkpoints were:

| CUDA cycle | Pre-load `VmRSS` | Post-load `VmRSS` | Post-unload `VmRSS` | Owner-drop `VmRSS` |
|---|---:|---:|---:|---:|
| 1 (primary) | 340,312,064 B | 358,006,784 B | 821,284,864 B | 821,211,136 B |
| 2 (reduced stability) | 821,272,576 B | 822,132,736 B | 859,877,376 B | 859,811,840 B |
| 3 (reduced stability) | 859,815,936 B | 860,663,808 B | 898,453,504 B | 863,121,408 B |

These RSS values describe the whole process, including executable and library mappings, allocator retention, workers, stacks, driver state, and other host allocations. They are sampled residency observations, not deterministic owner attribution.

#### Whole-device CUDA used-memory observations

Each delta is relative to that cycle's pre-load whole-device used-memory value.

| CUDA cycle | Checkpoint | Whole-device used memory | Delta from cycle pre-load |
|---|---|---:|---:|
| 1 (primary) | Pre-load | 1,529,151,488 B | 0 B |
| 1 (primary) | Post-load | 3,812,950,016 B | +2,283,798,528 B |
| 1 (primary) | Post-unload / owner-drop | 1,577,385,984 B | +48,234,496 B |
| 2 (reduced stability) | Pre-load | 1,577,385,984 B | 0 B |
| 2 (reduced stability) | Post-load | 3,859,087,360 B | +2,281,701,376 B |
| 2 (reduced stability) | Post-unload / owner-drop | 1,577,385,984 B | 0 B |
| 3 (reduced stability) | Pre-load | 1,577,385,984 B | 0 B |
| 3 (reduced stability) | Post-load | 3,859,087,360 B | +2,281,701,376 B |
| 3 (reduced stability) | Post-unload / owner-drop | 1,577,385,984 B | 0 B |

The maximum retained delta was 48,234,496 B (46 MiB), in cycle 1. There was no strict monotonic retained growth across the three cycles. These are whole-device values, not process-attributed CUDA allocations.

#### Focused validation and manual product evidence

The two adapter tests, the direct E0 snapshot test, and the E1 device test all passed. The direct E0 snapshot test is the exact-zero-accounting owner; the external E1 run establishes that the selected device and planned footprint were accepted by the product load contract. All recorded cancellation, unload, and shutdown paths were clean.

A manual Slint user check confirmed CPU and CUDA behavior, described CUDA prompt output as near instant, reported no issues, and accepted the implementation. The automation launch reached its 20-minute bound while the window remained open. No screenshot or automated UI assertion is claimed.

#### Interpretation and limitations

This evidence establishes controlled local product-path behavior for the exact Commit E tree, model revision, host, RTX 5070 Ti ordinal, driver, toolkit, build cap, scalar choices, and workload above. The lower CUDA durations are observations for that exact composition, not a hard threshold or a hardware-only speedup claim: the shared BF16 source executed as F32 on CPU and BF16 on CUDA, and CUDA transferred full-vocabulary logits to host F32 for host sampling. GPU sampling was not measured.

Nothing here generalizes to NVIDIA devices as a family, another device/model/revision/scalar, concurrent or steady-state serving, model quality, or production capacity. Whole-device CUDA values cannot attribute memory to this process. The three CUDA cycles and absence of strict monotonic retained growth prove neither a leak nor non-leak; sampled RSS likewise cannot do so. Clean lifecycle outcomes, the accepted E1 contract, and exact direct-E0 zero accounting establish their named ownership boundaries without claiming immediate OS, allocator, or driver reclamation.

### Historical Phase 10 CPU product evidence

The following Commit C CPU evidence is retained as the historical Phase 10 product baseline; it is not the current Phase 11 CPU/CUDA comparison.

#### Exact code-under-test and model identity

The authoritative external run executed the release `external-baseline` binary directly after building it on clean Commit C. The source/index remained clean before and after execution; generated state existed only beneath ignored root `target/`.

| Field | Recorded value |
|---|---|
| Commit C | `771c0de4d72565a6302ca60f3b6bafd8c807962b` |
| Commit C tree | `3d5b6ccc5ecc959de7cb370c1147f76e4cd32e3f` |
| Raw report | `target/phase10-evidence/external.json`; schema version 1; `dirty: false` |
| Repository | `TinyLlama/TinyLlama-1.1B-Chat-v1.0` |
| Requested revision | `fe8a4ea1ffedaf415f4da2f062534de366a451e6` |
| Resolved immutable commit | `fe8a4ea1ffedaf415f4da2f062534de366a451e6` |
| Composition | Candle / Hugging Face Hub / CPU / Safetensors / Llama; BF16 source with F32 CPU execution under the then-current policy |
| Vocabulary / context / prefill capacities | 32,000 / 2,048 / 2,048 tokens |
| Upstream-declared license metadata | `apache-2.0`, from the pinned revision's [model card](https://huggingface.co/TinyLlama/TinyLlama-1.1B-Chat-v1.0/raw/fe8a4ea1ffedaf415f4da2f062534de366a451e6/README.md); this records the upstream declaration, not a broader legal conclusion |
| Explicit cache | `target/phase10-external-cache`; empty before resolution; populated only as an ignored generated artifact |

The initially empty cache means the 21.031913121-second resolution interval included first acquisition and local cache population. It is still the complete public E1 resolve interval—not a pure network-transfer benchmark—because it also includes Hub metadata handling plus artifact/tokenizer/configuration validation and local I/O.

#### Controlled environment and workload

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

#### Compatibility and lifecycle results

The compatible chat submission was accepted through the public conversation API. It observed matching request identity, `GenerationStarted`, non-empty decoded output, terminal and released states, and the matching terminal application event. It completed by end-of-sequence with 30 prompt tokens, 6 generated tokens, and 27 decoded bytes. The user record and completed active assistant attempt were validated after release, then public conversation clear left conversation and context diagnostics empty.

The direct warmup reached token limit with 17 prompt tokens, exactly 32 generated tokens, 150 decoded bytes, matching terminal/released/event outcomes, and clean release. All three measured requests then produced the same usage and decoded-byte counts sequentially against the already loaded model. No request entered cleanup-pending or cleanup-exhausted state, no worker disconnected, and no active generation remained after release.

`ModelUnloadBehavior::RejectIfBusy` completed with zero cancelled requests and left no public loaded model or active generation while both workers remained available. Explicit bounded shutdown returned successfully, left Hub and inference unavailable with no loaded model or active generation, and the temporary redb workspace was removed. Successful E1 lifecycle events and public state establish application-visible ownership cleanup; process RSS is a separate coarse observation.

#### External timing results

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

#### Process RSS observations

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

The sampled RSS dropped by 3,167,993,856 bytes from the final measured release to post-unload and by another 410,099,712 bytes after shutdown. Remaining RSS is not treated as retained model/runtime ownership: it includes the whole process, executable/library mappings, allocator arenas/caches, stacks, and other host state. `/proc` RSS/HWM accounting and checkpoint sampling are coarse and can miss or approximate transient residency; the highest observed reported field was 5,666,680,832 bytes. That historical run used CPU and recorded no device-memory evidence.

#### Interpretation and limitations

This closes the missing current-tree CPU product baseline and is sufficient to make Phase 11 ready for a separate activation decision. It does not implement or evidence GPU capability. It is one local run on one desktop host, one model/revision and one BF16-source/F32-execution policy, one initially empty cache, one chat proof, one warmup, and three short direct completions. Background desktop and editor services remained present, so these values are controlled local evidence rather than isolated laboratory results.

The result supports product-path compatibility, deterministic workload comparison, and lifecycle regression investigation on a comparable host. It does not establish model quality, concurrent or steady-state serving throughput, another model/format/engine/device, allocator attribution, cross-host portability, a memory leak/non-leak conclusion, or a production optimization target. No production optimization is justified by this baseline alone.

## Interpretation and deferred work

- No production optimization or portable performance threshold is justified by these controlled local baselines alone.
- Tokenizer encode/streaming decode, context planning, output accumulation, and isolated Candle kernels remain deferred until a named decision and profile show that current surfaces are insufficient.
- Synthetic timings are useful for regression investigation, lifecycle confidence, and harness comparison—not product claims.
- A proposed optimization should compare equivalent before/after trees on this controlled host and include a profile or other evidence identifying the cost.
