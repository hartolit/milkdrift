# Runtime benchmarks

`runtime-benchmarks` is a non-production observer for bounded runtime evidence. It has two surfaces:

- the normal `baseline` runner exercises deterministic hosted E0 scenarios plus separate download-free E1 startup/shutdown cycles;
- `benches/runtime.rs` uses Criterion for two hosted E0 submission-to-event boundaries.

Operational timeouts stop hangs; they are not performance thresholds. A lifecycle, fixture, output, cleanup, accounting, or join mismatch fails the run. A slow valid sample does not.

## Package boundary

The package is an outer workspace consumer of public production APIs. Production, application, tooling, and test packages do not depend on it. Benchmark support remains inside this unpublished package and is exposed only as the narrow `runtime_benchmarks::e0` seam required by the separate Cargo benchmark crate.

The normal runner and Criterion target use the same concrete Candle hosted-E0 worker owner, ticket allocation, event matching, fixture-load validation, direct request lifecycle, unload, shutdown, and bounded join implementation. No benchmark helper is exposed through `application-runtime` or `inference-runtime`.

The deterministic fixture is referenced from `crates/runtime/inference-runtime/tests/fixtures/candle-llama`; it is not copied. Verification checks regular files, exact byte sizes, recomputed SHA-256 values, parsed configuration, and the loaded public descriptor before measurements are accepted.

## Module map

```text
src/
├── cli.rs              bounded single-mode command line
├── fixture.rs          fixture identity, hashing, parsing, and source construction
├── e0/
│   ├── harness.rs      hosted worker, tickets/events, model tracking, cleanup, join
│   ├── lifecycle.rs    load, direct request, prefill/decode, completion, unload
│   ├── generation.rs   scheduled generation and public-output observation
│   ├── observation.rs  snapshots, accounting validation, report conversion
│   └── synthetic.rs    normal-runner scenario ordering and timing assembly
├── e1/
│   ├── mod.rs          typed bounded application-shutdown cleanup policy
│   └── lifecycle.rs    download-free application start/shutdown cycles
├── memory.rs           sampled process RSS and host-memory parsing
├── metadata.rs         allowlisted process/toolchain metadata
├── report.rs           JSON schema only
└── workspace.rs        temporary state under root target/

benches/runtime.rs       Criterion target selection and duration accumulation only
```

## Run the synthetic baseline

From the repository root:

```text
mkdir -p target/runtime-benchmarks
cargo run --release --locked \
  -p runtime-benchmarks \
  --bin baseline \
  -- \
  --mode synthetic \
  --warmup 1 \
  --cycles 3 \
  > target/runtime-benchmarks/synthetic.json
```

`--mode synthetic` is optional but is the only accepted mode. Warmup and sample counts must be non-zero and are bounded by the CLI.

The runner writes one typed JSON document to stdout and progress/a compact summary to stderr. It emits no generated text, token IDs, secrets, broad environment dump, or model-cache contents.

## Compile and run Criterion

Compile the package target without executing measurements:

```text
cargo check --locked -p runtime-benchmarks --all-targets
cargo bench --workspace --no-run --locked
```

Run only a focused target when measurement is intended:

```text
cargo bench --locked \
  -p runtime-benchmarks \
  --bench runtime \
  -- e0_hosted_checked_prefill/4_tokens

cargo bench --locked \
  -p runtime-benchmarks \
  --bench runtime \
  -- e0_hosted_incremental_decode/1_token_after_2_token_prefill
```

Criterion has no regression threshold. Its raw samples and reports are local generated artifacts.

## Measurement boundaries

| Surface | Timed | Excluded and still required |
|---|---|---|
| E0 worker start | `start_hosted_runtime` through returned endpoint/thread handles. | Fixture verification and the first readiness snapshot. |
| Model load | `LoadModel` submission through matching `ModelLoaded`. | Source construction; descriptor and accounting validation. |
| Checked prefill | `Prefill` submission through matching `PrefillCompleted`. | Model/request setup, prompt/logits allocation, outcome/usage validation, request completion. |
| First public token | `Generate` submission through admission receipt/validation and the first token observed at `pull_token_output`. | Request construction. |
| Post-first proxy | First public-token observation through four additional public tokens. | Completion through matching `Terminal` and `Released`; this is a short synthetic proxy, not steady state. |
| Backpressure | A separately recorded fixed no-pull hold, then the freeing pull and its output-backpressure validation through observation of the next token. | Active snapshot validation plus eventual terminal/release and clean-accounting validation. |
| Cancellation | `CancelRequest` submission to separate acknowledgement, `Terminal`, and `Released` observations. | Progress precondition, outcome validation, and exact release accounting. |
| Model unload | `UnloadModel(RejectIfBusy)` submission through matching successful unload. | All request releases and the empty post-unload snapshot. |
| E0 shutdown | `Shutdown` submission through matching event and successful bounded join; event and join portions remain separate. | A clean receipt and no retained worker handle. |
| E1 lifecycle | Separate `ApplicationRuntime::start` and `shutdown` calls through their successful returns. | Clean initial/terminal state checks and temporary-workspace removal. No resolution, load, or generation occurs. |
| Criterion prefill | `Prefill` submission through matching `PrefillCompleted`. | All setup, semantic validation, completion, unload, shutdown, and join. |
| Criterion decode | `Decode` submission through matching `DecodeCompleted` after an untimed two-token setup prefill. | Request/setup prefill, semantic validation, completion, unload, shutdown, and join. |

Criterion accumulates only the returned submission-to-event duration. The vocabulary-sized logits vector is allocated once per target and moved back from each completion event for reuse.

## Artifacts and temporary state

Use the root `Cargo.lock` and root `target/` only. Do not create `benchmarks/runtime/Cargo.lock`, `benchmarks/runtime/target`, or a source-tree results/cache directory.

Temporary redb files and the empty cache directory needed for download-free E1 startup are created under `target/runtime-benchmarks` and removed where possible. Criterion uses Cargo/Criterion output under root `target`. Generated JSON, Criterion data, profiles, heap dumps, and caches are not committed.

## Evidence limits

Synthetic E0 results are deterministic integration evidence for the tiny project-authored fixture. They do not establish model quality, product-model performance, representative vocabulary/context scale, production serving throughput, GPU/device behavior, or allocation freedom.

E1 lifecycle cycles prove only repeated frontend-neutral worker startup and bounded shutdown without Hub resolution. The former real-product benchmark mode was removed because it was network-dependent, large, and never executed; future product evidence should be added only as a narrow opt-in path that is actually exercised and does not duplicate the existing external smoke.

`RuntimeSnapshot` accounting is deterministic runtime ownership/admission data. `/proc/self/status` RSS and high-water marks are sampled process-wide observations. RSS includes the executable, workers, stacks, libraries, allocator retention, mappings, and earlier cycles; sampling can miss transient peaks and cannot attribute Candle/native or device resources. Runtime accounting and RSS remain separate in the report.
