# Runtime benchmarks

`runtime-benchmarks` is the sole non-production cross-crate measurement package. It observes reviewed public production APIs and has three current surfaces:

- the normal `baseline` binary runs bounded, download-free synthetic hosted-E0 scenarios plus fresh E1 startup/shutdown cycles;
- the `external-baseline` binary is the sole opt-in external E1 product-baseline path, parameterized by mandatory `--device cpu|cuda:0` selection;
- `benches/runtime.rs` runs two hosted-E0 Criterion submission-to-event measurements.

Operational timeouts stop hangs; they are not performance thresholds. Fixture, identity, output, lifecycle, cleanup, accounting, or join mismatches fail. A slow valid sample does not.

Canonical methodology, environments, preserved measurements, RSS/device-memory interpretation, and limitations live in [performance evidence](../../docs/project/performance.md). Repository-wide procedures live in [validation](../../docs/project/validation.md). Historical reports retain the semantics of the code that emitted them; they are not rewritten as measurements of the Phase 12 tree.

## Package contract

- Run Cargo from the repository root with the committed root `Cargo.lock`.
- Use only the root `target/`; never create `benchmarks/runtime/Cargo.lock`, `benchmarks/runtime/target`, or a source-tree results/cache directory.
- Production, application, tooling, and test packages do not depend on this package.
- Benchmark support remains private to this package; no helper is added to a production public API solely for measurement.
- The non-default `runtime-benchmarks/cuda` feature forwards only to `application-runtime/cuda`; CPU remains selectable in that binary.
- The committed Candle fixture is referenced in place. Its byte sizes and hashes identify the files; scalar layout and memory accounting come from `prepare_load` plan data, never from scaling the whole Safetensors file length.
- Reports are serialize-only. This package has no legacy report parser, so preserving historical schema meaning does not justify inventing one.

The sole external runner is split by evidence responsibility, not by device:

```text
external/
├── generation/   # fixed workload, one request observer, validation, summaries
├── model/        # exact identity, resolution, observer preparation, E1 load lifecycle
├── observation/  # requested/selected/actual devices, process RSS, whole-device CUDA
├── lifecycle.rs  # start -> select -> resolve -> prepare -> load -> workload -> unload -> shutdown -> owner drop
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
mkdir -p target/runtime-evidence
cargo run --release --locked \
  -p runtime-benchmarks \
  --bin baseline \
  -- \
  --mode synthetic \
  --warmup 1 \
  --cycles 3 \
  > target/runtime-evidence/synthetic-schema3.json
```

The runner writes one schema-versioned JSON document to stdout and progress plus a compact summary to stderr. It excludes generated text, token IDs, credentials, secrets, and broad environment dumps.

Each schema-3 E0 cycle creates an observer-owned, unmaterialized `prepare_load` transaction before the timed E0 load. The opaque preparation is dropped without materialization after copying its public plan. The report then keeps these facts separate:

- `prepared.configuration_declared_scalar`: optional producer-intent metadata (`null` when absent);
- `prepared.observed_tensor_scalars`: deterministic labels derived from the plan descriptor's observed `ScalarTypeSet` in stable category-bit order;
- `prepared.planned_execution_scalar` and `prepared.planned_execution_device`: execution selected by the exact preparation;
- `prepared.exact_final_footprint`: exact final deterministic tensor ownership from `LoadPlan::expected_footprint`;
- `prepared.loading_peak_footprint`: exact component-wise deterministic loading peak from `LoadPlan::loading_peak_footprint`;
- `receipt.actual_execution_scalar` and `receipt.actual_execution_device`: actual facts verified by E0 against its accepted plan and loaded model;
- `receipt.reserved_footprint`: direct E0 post-load reserved ownership, required to equal the exact final footprint;
- snapshot `process_memory`: sampled process RSS/HWM, separate from every E0 reserved footprint.

The loading peak is an admission-phase deterministic tensor quantity, not post-load reserved ownership or physical RSS. The fixture remains homogeneous F32; schema 3 makes that truth explicit as declared `F32`, observed `{F32}`, planned F32 CPU execution, and actual F32 CPU execution.

### Synthetic schema history

**Synthetic schema 1 (historical):** the original normal-runner document combined synthetic and real-product modes. Its singular `model.scalar_type` and derived summaries/generation fields retain only their then-current meanings; they are not parsed or reinterpreted as Phase 12 declared, observed, planned, or actual facts.

**Synthetic schema 2 (historical):** the split download-free report introduced the current synthetic-E0 and E1-lifecycle result organization. `fixture.scalar_type: "F32"` was the reviewed homogeneous fixture label, while E0 snapshots recorded reserved ownership and process memory. It did not record an observer preparation or loading peak.

**Synthetic schema 3 (current):** replaces the singular fixture scalar with per-cycle prepared declaration/layout/planned facts and actual E0 receipt facts, including exact final and loading-peak footprints. Existing timing and snapshot units retain their schema-2 meanings.

Synthetic and external reports have independent version sequences.

## Run the controlled CPU and CUDA product baseline

The external binary fixes `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable revision `fe8a4ea1ffedaf415f4da2f062534de366a451e6`; callers cannot substitute either identity. The fixed artifact profile is `config.json`, `tokenizer.json`, and one unindexed `model.safetensors`. It requires explicit network authorization, explicit device selection, a clean committed tree, and an already-existing canonical cache beneath repository-root `target/` or outside the repository. It never reads a default global Hub cache implicitly and never falls back from CUDA to CPU.

The profile's configuration declaration is optional in the schema but required to equal `Some(BF16)` for this exact revision. An accepted run also requires the observer `prepare_load` descriptor to report homogeneous observed `{BF16}`. CPU planning selects F32 execution; supported CUDA planning selects BF16 execution. E1's loaded model supplies the actual execution scalar and actual device after E0 receipt validation.

External schema 4 records:

- repository, immutable revision/commit, license-metadata provenance, and the fixed artifact layout;
- `configuration_declared_scalar` separately from `observed_tensor_scalars`;
- `planned_execution_scalar` separately from `actual_execution_scalar`;
- requested device, each cycle's planned device, selected E1 device, and actual E0-verified loaded device;
- `prepared_load.exact_final_footprint` and `prepared_load.loading_peak_footprint` copied from an unmaterialized `prepare_load` plan;
- public E1 load acceptance without claiming a direct same-worker E0 reservation (`e0_reserved_ownership_observed` is required to be `false`);
- `process_memory` checkpoints containing sampled process RSS/HWM;
- `whole_device_cuda_memory` checkpoints containing qualified driver total/free/used values for the complete device, never process-attributed CUDA ownership.

The observer does not derive tensor counts, execution bytes, final accounting, or loading peaks from the complete Safetensors file length. File length is used only to reject missing/empty fixed artifacts. Exact accounting comes from the adapter's checked per-tensor preparation plan.

Build separate release artifacts, then execute them directly and sequentially so no compiler process overlaps model residency:

```text
mkdir -p target/phase12-cpu target/phase12-cuda target/phase12-evidence

CARGO_TARGET_DIR="$PWD/target/phase12-cpu" \
cargo build --release --locked \
  -p runtime-benchmarks \
  --bin external-baseline

CUDA_COMPUTE_CAP=120 \
CARGO_TARGET_DIR="$PWD/target/phase12-cuda" \
cargo build --release --locked \
  -p runtime-benchmarks \
  --features cuda \
  --bin external-baseline

target/phase12-cpu/release/external-baseline \
  --allow-network \
  --cache-dir target/phase10-external-cache \
  --device cpu \
  > target/phase12-evidence/tinyllama-cpu-schema4.json

target/phase12-cuda/release/external-baseline \
  --allow-network \
  --cache-dir target/phase10-external-cache \
  --device cuda:0 \
  > target/phase12-evidence/tinyllama-cuda-schema4.json
```

The primary cycle on each device runs one compatible-chat proof, one direct-completion warmup, three measured 32-token completions, and one progress-triggered cancellation. The CUDA invocation then runs two additional load/generate/release/cancel/unload/shutdown stability cycles, yielding three complete CUDA lifecycle cycles total. Stdout contains one schema-versioned report; progress and the compact summary use stderr.

CUDA total/free/used observations are safe driver observations for the whole device, not process-attributed memory. Each cycle establishes its own pre-load baseline after the observer preparation; retained-delta stability uses post-unload and post-owner-drop deltas while preserving absolute observations. Public E1 does not expose the product worker's E0 `RuntimeSnapshot`, so the external report does not fabricate E0 reserved ownership or a post-unload accounting snapshot. Direct E0 zero-accounting evidence remains a separate download-free hardware test.

Every safe Candle `discover_device` observation constructs a temporary Candle CUDA device and cudarc context. Schema 4 records the exact number of those discovery calls. The separate observer `prepare_load` call also initializes its requested device but is not mislabeled as a discovery call. All are cold lifecycle operations and never per token. Safe reuse would require exposing production context ownership or adding a lower-level benchmark dependency, neither of which is justified by this inward observer.

Ordinary tests and shared CI compile this path but never execute it, access the network, or require CUDA hardware. Resource preflight, hardware tests, report review, and the bounded manual Slint procedure are in [controlled CPU and CUDA external product evidence](../../docs/project/validation.md#controlled-cpu-and-cuda-external-product-evidence); curated historical results live only in [performance evidence](../../docs/project/performance.md#external-product-evidence).

### External schema history

**External schema 1 (historical):** the CPU-only report serialized the then-current loaded-model `scalar_type` label and had no separate planned/actual execution scalar or multi-device evidence. Its Commit C measurements remain historical schema-1 evidence.

**External schema 2 (historical):** the first shared CPU/CUDA report split `source_scalar` from `execution_dtype`, recorded requested/selected/actual device identities, and used the then-current `e0_footprint` evidence shape. The canonical Commit E CPU/CUDA measurements retain exactly those meanings.

**External schema 3 (historical):** renamed `execution_dtype` to `execution_scalar`, renamed the plan evidence to `accounted_footprint`, and added bounded CUDA discovery/context-call evidence. Its schema-3 regression remains attributed to its recorded commit/tree and does not replace the schema-2 timing tables.

**External schema 4 (current):** replaces singular source-scalar evidence with optional configuration declaration plus observed `ScalarTypeSet`, splits planned from actual execution scalar, records exact final and loading-peak preparation quantities, and names process RSS, absent direct E0 reserved ownership, and whole-device CUDA observation independently.

No legacy parser has been added for schemas 1–3. Their descriptions and preserved measurements remain provenance records, not migration inputs.

### External mixed-checkpoint evidence gap

The pinned TinyLlama revision is homogeneous BF16 and is retained only as the established product/lifecycle regression profile. It is not mixed-dtype checkpoint evidence and must not be used as a substitute for one.

No suitable immutable, license-reviewable external mixed-dtype Llama-compatible checkpoint has been responsibly pinned for this runner on the current tree. Therefore external CPU/CUDA mixed-checkpoint evidence remains absent. Download-free deterministic adapter/E0/E1 fixtures establish the reviewed compatibility behavior, while the external evidence claim remains explicitly open until a genuinely mixed checkpoint, immutable revision, provenance, direct-completion profile, and CPU/CUDA executions can be reviewed. Missing credentials or network access would be acquisition failures, not product incompatibility.

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

The normal `baseline` CLI remains synthetic-only and rejects product/network options. The external binary has a separate opt-in report schema and evidence contract. Executed CUDA evidence applies only to its documented Linux/NVIDIA/GPU/toolkit matrix; it does not establish generic NVIDIA compatibility, model quality, serving capacity, cross-host performance, allocation freedom, mixed-checkpoint external evidence, or an optimization threshold. CUDA sampling remains on the host after vocabulary-logit transfer. Canonical interpretation is in [performance evidence](../../docs/project/performance.md#external-product-evidence).
