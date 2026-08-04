# Runtime benchmarks

`runtime-benchmarks` is the sole non-production cross-crate measurement package. It observes reviewed public production APIs and has three current surfaces:

- the normal `baseline` binary runs bounded, download-free synthetic hosted-E0 scenarios plus fresh E1 startup/shutdown cycles;
- the `external-baseline` binary is the sole opt-in external E1 product-baseline path, parameterized by mandatory `--device cpu|cuda:0` selection;
- `benches/runtime.rs` runs two hosted-E0 Criterion submission-to-event measurements.

Operational timeouts stop hangs; they are not performance thresholds. Fixture, identity, output, lifecycle, cleanup, accounting, or join mismatches fail. A slow valid sample does not.

Canonical methodology, environment, curated results, RSS/device-memory interpretation, and limitations live in [performance evidence](../../docs/project/performance.md). Repository-wide procedures live in [validation](../../docs/project/validation.md).

## Package contract

- Run Cargo from the repository root with the committed root `Cargo.lock`.
- Use only the root `target/`; never create `benchmarks/runtime/Cargo.lock`, `benchmarks/runtime/target`, or a source-tree results/cache directory.
- Production, application, tooling, and test packages do not depend on this package.
- Benchmark support remains private to this package; no helper is added to a production public API solely for measurement.
- The non-default `runtime-benchmarks/cuda` feature forwards only to `application-runtime/cuda`; CPU remains selectable in that binary.
- The committed Candle fixture is referenced in place and its byte sizes, hashes, parsed configuration, and loaded descriptor are verified before measurement.

The sole external runner is split by evidence responsibility, not by device:

```text
external/
├── generation/   # fixed workload, one request observer, validation, summaries
├── model/        # exact identity, resolution, independent plan, load lifecycle
├── observation/  # device matrix, resources, environment
├── lifecycle.rs  # start -> select -> resolve -> plan -> load -> workload -> unload -> shutdown -> owner drop
└── report.rs     # one versioned CPU/CUDA report contract
```

CPU and CUDA use the same CLI, binary, E1 lifecycle, generation observer, cleanup path, and report schema.

## Test and compile

```text
cargo check --locked -p runtime-benchmarks --all-targets
cargo test --locked -p runtime-benchmarks
cargo clippy --locked -p runtime-benchmarks --all-targets -- -D warnings
cargo bench --workspace --no-run --locked

CUDA_COMPUTE_CAP=120 cargo check --locked \
  -p runtime-benchmarks \
  --all-targets \
  --features cuda
```

These commands compile the current benchmark targets without claiming CUDA hardware execution or a product baseline. Ordinary tests remain network-free.

The separate [`cuda-hardware` workflow](../../.github/workflows/cuda-hardware.yml) runs the exact CUDA feature graph and committed adapter/E0/E1 fixtures on the dedicated `milkdrift-cuda-5070ti` self-hosted label. It accepts only trusted `main` pushes or owner dispatches of `main`, uses read-only permissions and offline Cargo, and never runs this package's external model binary or a performance threshold. The full security and maintenance procedure is in [validation](../../docs/project/validation.md#self-hosted-cuda-hardware-correctness-gate).

## Run the synthetic baseline

```text
mkdir -p target/phase10-evidence
cargo run --release --locked \
  -p runtime-benchmarks \
  --bin baseline \
  -- \
  --mode synthetic \
  --warmup 1 \
  --cycles 3 \
  > target/phase10-evidence/synthetic.json
```

The runner writes one schema-versioned JSON document to stdout and progress plus a compact summary to stderr. It excludes generated text, token IDs, credentials, secrets, and broad environment dumps.

## Run the controlled CPU and CUDA product baseline

The external binary fixes `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable revision `fe8a4ea1ffedaf415f4da2f062534de366a451e6`; callers cannot substitute either identity. It requires explicit network authorization, explicit device selection, a clean committed tree, and an already-existing canonical cache beneath repository-root `target/` or outside the repository. It never reads a default global Hub cache implicitly and never falls back from CUDA to CPU.

External schema version 3 records `source_scalar` from explicit resolved/loaded E1 facts, `execution_scalar` from the independent public Candle plan only after matching E1 load acceptance, and separate requested, selected-E1, and actual-loaded-E0 device identities. `accounted_footprint` is the accepted independent plan, not physical residency or a same-worker E0 reservation snapshot. Process RSS and qualified whole-device CUDA total/free/used observations remain separate resource checkpoints. Schema version 2 retains its historical meaning and is not reinterpreted.

Build separate release artifacts, then execute them directly and sequentially so no compiler process overlaps model residency:

```text
mkdir -p target/phase11-cpu target/phase11-cuda target/phase11-evidence

CARGO_TARGET_DIR="$PWD/target/phase11-cpu" \
cargo build --release --locked \
  -p runtime-benchmarks \
  --bin external-baseline

CUDA_COMPUTE_CAP=120 \
CARGO_TARGET_DIR="$PWD/target/phase11-cuda" \
cargo build --release --locked \
  -p runtime-benchmarks \
  --features cuda \
  --bin external-baseline

target/phase11-cpu/release/external-baseline \
  --allow-network \
  --cache-dir target/phase10-external-cache \
  --device cpu \
  > target/phase11-evidence/cpu.json

target/phase11-cuda/release/external-baseline \
  --allow-network \
  --cache-dir target/phase10-external-cache \
  --device cuda:0 \
  > target/phase11-evidence/cuda.json
```

The primary cycle on each device runs one compatible-chat proof, one direct-completion warmup, three measured 32-token completions, and one progress-triggered cancellation. The CUDA invocation then runs two additional load/generate/release/cancel/unload/shutdown stability cycles, yielding three complete CUDA lifecycle cycles total. Stdout contains one schema-versioned report; progress and the compact summary use stderr.

CUDA total/free/used observations are safe driver observations for the whole device, not process-attributed memory. Each cycle establishes its own pre-load baseline; retained-delta stability uses post-unload and post-owner-drop deltas while preserving absolute observations. The report records an independent public adapter plan plus validated E1 acceptance of the E0 load contract; it does not fabricate a same-worker E0 reservation or post-unload accounting snapshot. Direct E0 zero-accounting evidence is an explicit separate hardware test.

Every safe Candle `discover_device` observation constructs a temporary Candle CUDA device and cudarc context. Schema 3 records the exact number of these calls. They occur only at cold identity/resource checkpoints and never per token. The runner intentionally keeps this behavior: safe reuse would require exposing context ownership through production APIs or adding a lower-level benchmark dependency, neither of which is justified by this observation-only path.

Ordinary tests and shared CI compile this path but never execute it, access the network, or require CUDA hardware. Resource preflight, hardware tests, report review, and the bounded manual Slint procedure are in [Phase 11 validation](../../docs/project/validation.md#phase-11-controlled-cpu-and-cuda-product-evidence); curated results live only in [performance evidence](../../docs/project/performance.md#external-product-evidence).

## Run focused Criterion targets

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

Criterion has no repository regression threshold. Raw samples and reports are local generated artifacts; only curated values belong in [performance evidence](../../docs/project/performance.md).

## Artifacts and temporary state

Temporary redb state for E1 lifecycle checks is created beneath root `target` and removed where possible. Captured JSON, Criterion data/HTML, profiles, heap dumps, and caches remain beneath root `target` or outside the repository and are not committed.

## Evidence limits

The synthetic fixture proves deterministic integration and lifecycle behavior, not model quality, product-model speed, representative scale, production serving throughput, allocation freedom, or product-model GPU behavior. Synthetic E1 cycles cover startup and bounded shutdown without Hub resolution, model loading, or generation.

The normal `baseline` CLI remains synthetic-only and rejects product/network options. The external binary has a separate opt-in report schema and evidence contract. Executed CUDA evidence applies only to its documented Linux/NVIDIA/GPU/toolkit matrix; it does not establish generic NVIDIA compatibility, model quality, serving capacity, cross-host performance, allocation freedom, or an optimization threshold. CUDA sampling remains on the host after vocabulary-logit transfer. Canonical interpretation is in [performance evidence](../../docs/project/performance.md#external-product-evidence).
