# Runtime benchmarks

The Phase 10 `runtime-benchmarks` package owns two deliberately different harnesses:

- `src/bin/baseline.rs` is a bounded **system/integration baseline runner**. Its default synthetic mode exercises public hosted E0 lifecycle APIs without downloads; its opt-in real-product mode independently exercises public E1 `ApplicationRuntime` APIs.
- `benches/runtime.rs` is a **Criterion component-like harness** for repeatable hosted E0 checked prefill and incremental decode boundaries.

Neither harness sets pass/fail timing thresholds. Hard timeouts exist only to stop hangs. A lifecycle, identity, cleanup, accounting, or output mismatch fails the run; a slow but otherwise valid measurement does not.

## Package boundary

`benchmarks/runtime` is the registered root-workspace package `runtime-benchmarks`. Its role is a non-production measurement observer: it consumes public production APIs but is not part of product execution or composition. It uses the committed root `Cargo.lock` and shared root `target`, declares `publish = false`, and has no nested workspace or lockfile, build script, Cargo custom-build target, or build dependencies. No production, tooling, test, or application package depends on it through any dependency kind.

Its complete dependency set is:

- workspace-local normal: `application-runtime`, `candle-backend`, `domain-contracts`, `host-runtime`, and `inference-runtime`;
- external normal: `serde`, `serde_json`, and `sha2` 0.11;
- external development: `criterion`.

These are observer edges outside the production graph even where Cargo classifies them as `normal`.

## Evidence classes

| Harness | Evidence supplied by a successful run | Evidence not supplied |
|---|---|---|
| Normal runner, synthetic mode | Download-free cross-crate integration evidence for Candle plus hosted E0 scheduling, public pull output, lifecycle/accounting, shutdown/join, and sampled process RSS. | Product model performance, model quality, production steady-state throughput, isolated allocator behavior, or device-memory attribution. |
| Normal runner, real-product mode | Opt-in public E1 product-path evidence for one immutable Hugging Face model selection, including resolution/cache/download, Candle load, decoded output and usage, unload, and clean application shutdown. | A broad model survey, production serving load, long-running steady state, quality evaluation, or isolated allocation evidence. |
| Criterion target | Comparative statistical component-regression evidence for two precise hosted E0 command/event boundaries against the deterministic fixture. | E1/product latency, full generation throughput, RSS, allocation counts, native-resource attribution, or CI pass/fail gates. |

The synthetic fixture is integration evidence, **not product performance**.

## Deterministic synthetic fixture

The synthetic baseline and Criterion target reference, and never copy, the existing fixture using a path anchored at `CARGO_MANIFEST_DIR`:

`../../crates/runtime/inference-runtime/tests/fixtures/candle-llama`

It is project-authored deterministic synthetic test data documented by the fixture's `PROVENANCE.md`. It contains no trained or externally sourced model weights.

| File | Exact size | Required SHA-256 |
|---|---:|---|
| `config.json` | 360 bytes | `052b5c325859dc723ed0825f711950cbff112a140239953273cebacdb36afdd0` |
| `model.safetensors` | 4,800 bytes | `cc4798af93488b4fb2ae0548c2b28ace600521732b52023a7786c3227d72d672` |

Before either the normal runner or Criterion setup starts, shared fixture verification checks file presence, regular-file status, exact sizes, recomputed SHA-256 using `sha2` 0.11, and parsed configuration fields. The loaded public E0 descriptor must then report Candle backend, Llama architecture, Safetensors source, unquantized F32, vocabulary 16, and context 16.

## Normal synthetic baseline

Default CLI settings are one warmup cycle followed by three recorded cycles. Both counts must be non-zero; `--warmup` is capped at 10 and `--cycles` at 20. Warmup records remain in JSON for lifecycle auditability but are excluded from summary statistics.

Each E0 cycle starts a fresh hosted worker, loads once, performs the operations below, unloads, requests ticketed shutdown, waits to a hard bound for worker completion, and joins. Any `CleanupPending`, `CleanupExhausted`, degraded model, maintenance error, residual request/workspace, unexpected shutdown cleanup, or failed join invalidates the cycle.

| Target/question | Exact measured boundary | Setup and validation outside the boundary |
|---|---|---|
| E0 worker start | Call to `start_hosted_runtime` through returned client/thread handles. | The immediate `before-load` snapshot is an untimed readiness/accounting handshake. |
| Model load | Immediately before public `RuntimeCommand::LoadModel` submission through matching `ModelLoaded`. | Source construction and fixture checks occur first; descriptor and accounting validation occur after timing. |
| Hosted checked prompt prefill | Immediately before public `RuntimeCommand::Prefill` submission through matching `PrefillCompleted`. Throughput uses four consumed prompt tokens. | Model load, request/sequence creation, prompt ownership, and vocabulary-sized logits allocation are outside timing. Returned logits, checked outcome/usage, request completion, and release accounting are validated after timing. |
| Generation submission to first token | Immediately before public `RuntimeCommand::Generate` submission through the first generated token observed at `HostedRuntime::pull_token_output`. | Request construction is outside timing. Admission and all token/output identities are validated. A one-token pull capacity makes the first public observation unambiguous. |
| Post-first generated-token throughput | From observation of the first token at the public pull boundary through four additional observed tokens. | This is explicitly labeled a **synthetic short-window integration proxy; not representative production steady state**. Completion through matching `Terminal` and `Released` is outside the fixed window but mandatory. |
| Output-backpressure hold/recovery | A fixed 100 ms no-pull hold is recorded separately. Recovery starts immediately before the pull that frees the full one-token accumulator and ends when the next token is observed at a later public pull. | The run requires an explicit `Yielded(OutputBackpressure)` record, a during-backpressure snapshot, eventual token-limit `Terminal` and `Released`, and clean accounting. The hold is control setup, not a threshold. |
| Cancellation | Immediately before public `CancelRequest` submission. Independent durations end at matching control-plane acknowledgement, observable cancellation `Terminal` at the pull boundary, and observable `Released`. | An untimed first-token pull proves progress, then a short no-pull hold keeps the bounded request active. The run requires a non-zero partial generation, `Cancelled(UserRequested)` for both terminal states, no cleanup failure, and exact release accounting. |
| Model unload | Immediately before `UnloadModel(RejectIfBusy)` submission through matching `ModelUnload(Unloaded)`. | All requests must already be released. Cancellation count must be zero; the post-unload snapshot must be exactly empty. |
| E0 shutdown/join | Total starts immediately before `Shutdown` submission and ends after successful worker join; event and post-event join portions are also recorded. | A clean cycle requires the shutdown receipt to unload zero models and cancel zero requests. |

### Required snapshots and RSS checkpoints

Every synthetic cycle records public `RuntimeSnapshot` data plus sampled `/proc/self/status` process memory at:

- before load;
- after load;
- after the separately checked prefill request is completed;
- after the first-token/proxy generation is released;
- during controlled generation backpressure;
- after backpressure generation release;
- after cancellation release;
- after unload.

The required lifecycle checkpoints are therefore present directly, with extra release checkpoints that make per-request cleanup auditable. Runtime accounting and RSS are separate JSON objects and are never treated as interchangeable.

Synthetic mode also performs the same number of fresh, download-free `ApplicationRuntime::start`/`shutdown` cycles. These open unique temporary redb files, start both production workers, submit no Hub resolution, validate clean initial and terminal application state, and delete their workspace.

## Real-product mode

Real-product mode has no repository, revision, or substitution flags. It hardcodes and verifies:

- repository: `neubla/tiny-random-LlamaForCausalLM`
- immutable revision/commit: `1c81a3fba044af78df253edc66bdbab183184932`
- engine/source/device/format/scalar: Candle / Hugging Face Hub / CPU / Safetensors / F32

An explicit `--cache-dir PATH` is mandatory and must name an existing directory whose canonical location is under the shared repository-root `target/` or outside the repository. Source-tree cache locations are rejected. The current public E1 resolver always performs immutable Hub metadata resolution, so real-product mode also requires the unmistakable `--allow-network` flag and rejects `HF_HUB_OFFLINE=1` as contradictory.

The runner never accepts a replacement model and never invokes, includes, or parses the `candle_hub_smoke` example. It independently uses public `ApplicationRuntime` methods. Real-product mode is opt-in local evidence and is never run by ordinary CI. It compiled for the current Phase 10 evidence but was not executed, so no product baseline is recorded.

Each real cycle measures these separate boundaries:

| Target/question | Exact measured boundary |
|---|---|
| Application start | `ApplicationRuntime::start` call through returned runtime. |
| E1 resolve/cache/download | Immediately before `resolve_model` through `ApplicationEvent::ModelResolved`. This is separate from model load. In a network-enabled first warmup it can include download; warmup records are retained so that cost is not silently described as a normal cached sample. |
| Candle model load | Immediately before `load_model` through `ApplicationEvent::ModelLoaded`. Resolution has already completed. |
| First decoded output and usage | Immediately before `start_generation("Hello", …)` through the first non-empty decoded text fragment observed at public `pull_output`; only byte count and public prompt/generated usage are recorded, never text or tokens. |
| Post-first generated-token proxy | From first decoded-output observation through the fixed four-token public usage window when observable. A shorter observable remainder is labeled and recorded if generation ends first. This is an integration proxy, not steady state. |
| Normal generation/release | Eight-token, top-k-1 deterministic generation. Matching text-output `Terminal`, text-output `Released`, `GenerationFinished`, non-zero decoded bytes, exact usage, and no cleanup-pending event are mandatory. |
| Unload | Immediately before `unload_model_with_behavior(RejectIfBusy)` through matching `ModelUnloaded` with zero cancellations. |
| Application shutdown | `ApplicationRuntime::shutdown` call through its successful return after bounded worker shutdown/join. |

The real path uses normal token-limit completion rather than cancellation. Process RSS is sampled before/after start, resolution, load, released generation, unload, and shutdown.

## Criterion component target

`benches/runtime.rs` contains only two hosted public-E0 component-like targets. Criterion defaults are bounded to a 500 ms warmup, 2 second measurement window, and 10 samples per target. There are no regression thresholds.

| Criterion target | Exact question and decision/regression use | Timed in | Timed out |
|---|---|---|---|
| `e0_hosted_checked_prefill/4_tokens` | Did hosted checked-prefill submission-to-event cost regress for the deterministic four-token input? Use comparisons to investigate queue/dispatch/backend-prefill changes. | `try_submit(Prefill)` through reception and ticket matching of `PrefillCompleted`, including bounded transport, E0 dispatch/checking, Candle execution, event publication, and receive. | Fixture/source construction, worker start, model load, request/sequence creation, prompt creation, logits allocation, result validation, request completion, unload, shutdown, join. |
| `e0_hosted_incremental_decode/1_token_after_2_token_prefill` | Did one hosted incremental-decode submission-to-event boundary regress after a prepared deterministic prefix? Use comparisons to investigate queue/dispatch/backend-decode changes. | `try_submit(Decode)` through reception and ticket matching of `DecodeCompleted`, with the same transport/dispatch/event boundary. | Request/sequence creation, two-token checked setup prefill, token construction, logits allocation, completion, and all lifecycle setup/teardown. |

`iter_custom` accumulates only the named boundary. The vocabulary-sized logits vector is allocated before measurement and moved back from each completion event for reuse. Request and sequence creation happen per iteration, but outside accumulated duration. Model source construction, model load/unload, and worker start/shutdown happen once per target and remain outside Criterion measurement.

Candle's current CPU Llama implementation may allocate tensors or grow KV/cache resources during prefill/decode. Such work contributes to elapsed time when it occurs inside the boundary, but Criterion does not count, identify, or attribute Rust-global, Candle-native, BLAS, operating-system, or device allocations. The target has no allocator instrumentation and records no RSS.

## Metadata and output

The normal runner serializes one stable serde JSON record to **stdout** and writes progress plus a compact human summary to **stderr**. It never emits generated token IDs, decoded text, environment-wide values, access tokens, or other secrets. Fixed allowlisted build/runtime parallelism variables are the only environment values recorded.

Metadata includes:

- Git `HEAD`, `HEAD^{tree}`, and a boolean dirty state (not dirty file names);
- the Rust and Cargo version strings, the LLVM version reported by `rustc --version --verbose` when present, the native target triple, build profile, enabled features (`[]`), and the declared Criterion harness version (`0.8.2`);
- OS, kernel, CPU model, safely discoverable physical-core and logical-CPU counts, and total memory;
- the recorded values or absence of `CARGO_BUILD_JOBS`, `RAYON_NUM_THREADS`, `OMP_NUM_THREADS`, `OMP_THREAD_LIMIT`, `MKL_NUM_THREADS`, `OPENBLAS_NUM_THREADS`, `VECLIB_MAXIMUM_THREADS`, `NUMEXPR_NUM_THREADS`, `RUST_TEST_THREADS`, and `CANDLE_NUM_THREADS`; `CARGO_BUILD_JOBS` is build-parallelism metadata, not a claim about runtime worker count or effective measured parallelism;
- exact fixture or immutable product identity, revision, architecture, format, scalar type, vocabulary, and context capacity;
- prompt/generation counts, sampling policy and seed, mode, warmup/sample counts, and effective cache/network policy.

The runner writes no result files itself. Temporary redb/cache state used by download-free lifecycle checks is created below the repository root `target/runtime-benchmarks` and deleted. Run from the repository root and do not create a package-local `target`. For a captured result, create the root target directory before shell redirection (redirection happens before the process starts):

```text
mkdir -p target/runtime-benchmarks
cargo run --release --locked -p runtime-benchmarks --bin baseline -- \
  --mode synthetic --warmup 1 --cycles 3 \
  > target/runtime-benchmarks/synthetic.json
```

Criterion uses Cargo/Criterion's ordinary root `target/criterion` output. Raw JSON and Criterion output are generated local evidence and are not committed; curated result summaries belong in `docs/project/performance.md`.

## Commands

Default bounded download-free development smoke:

```text
cargo run --locked -p runtime-benchmarks --bin baseline
```

Release-profile synthetic baseline with explicit counts:

```text
cargo run --release --locked -p runtime-benchmarks --bin baseline -- \
  --mode synthetic --warmup 1 --cycles 3
```

Pinned product path with mandatory network opt-in and an allowed existing cache:

```text
mkdir -p target/runtime-benchmarks/hf-cache
cargo run --release --locked -p runtime-benchmarks --bin baseline -- \
  --mode real-product --cache-dir target/runtime-benchmarks/hf-cache \
  --allow-network --warmup 1 --cycles 3
```

Ensure `HF_HUB_OFFLINE` is not set to the exact value `1` before this invocation. An existing cache outside the repository may be supplied instead; a cache inside the repository is accepted only under the shared root `target/`.

Criterion components:

```text
cargo bench --locked -p runtime-benchmarks --bench runtime
```

Selected Criterion component:

```text
cargo bench --locked -p runtime-benchmarks --bench runtime -- \
  e0_hosted_checked_prefill/4_tokens
cargo bench --locked -p runtime-benchmarks --bench runtime -- \
  e0_hosted_incremental_decode/1_token_after_2_token_prefill
```

Shared CI compiles benchmark targets with `cargo bench --workspace --no-run --locked`; it never executes measurements or applies timing gates.

Unit tests cover CLI/network/cache policy, exact fixture hashing and parsing, metadata, CPU/RSS parsing, and synthetic-only behavior; no real-product mode is executed in tests. Current focused evidence records 15 passing tests:

```text
cargo test --locked -p runtime-benchmarks
```

## RSS, allocation, and native-resource limitations

Linux RSS comes from a safe parser for `/proc/self/status` `VmRSS`; `VmHWM` is also recorded. Values are process-wide sampled observations, not per-model ownership. They include the executable, stacks, workers, allocator arenas/caches, mapped pages, libraries, filesystem effects, and any unrelated process-resident state. Sampling can miss transient peaks; `VmHWM` is monotonic for the process and is affected by warmups and earlier cycles. RSS does not distinguish useful resident pages from retained allocator capacity, does not identify Candle/native allocations, and is not device-memory accounting. On non-Linux targets these fields are unavailable (`null`).

Public `RuntimeSnapshot` accounting is deterministic E0 admission/ownership accounting. It may include planned or reserved footprints and generation workspaces, while RSS reports operating-system process residency. A change in one does not imply the same change in the other; the report intentionally keeps them distinct.

Neither harness installs an allocation-counting global allocator. Setup vector allocations are explicitly outside Criterion timing, but elapsed measurements can still include allocations performed internally by production Candle/E0 code within the named boundary. Native libraries and device resources cannot be attributed by these harnesses.
