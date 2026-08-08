# Validation

This document owns repeatable validation commands and procedures. It does not claim that a command passed on a tree unless a baseline-specific record says so. Current support lives in [implementation status](implementation-status.md), exact performance results in [performance evidence](performance.md), and closed-tree outcomes in [execution history](../agent/execution/history.md).

## Evidence provenance

Run acceptance from a clean committed tree. Record:

```sh
git status --short --untracked-files=all
git rev-parse HEAD
git rev-parse 'HEAD^{tree}'
```

A command that passed on another commit or an earlier dirty tree is not evidence for the current tree. Local validation and GitHub CI are separate facts; do not claim remote CI passed without observing its actual run.

Use one Cargo process at a time. Do not run `cargo clean`. Keep generated output under the root `target/` or outside the repository.

## Phase 12 closure evidence

Phase 12 Segment 1 is commit `58490fe693fef7a2635956181088664cd90685e8`; Segment 2 is commit `12510695aa29be6a2665dbf3777cccbb8172c2d1`; Segment 3 is this coherent validation/project-truth closure commit.

On 2026-08-08, these sequential download-free CPU commands passed on the closure tree:

```sh
cargo test --locked -p candle-backend --test llama_cpu
cargo test --locked -p inference-runtime --test native_backend_generation
cargo test --locked -p inference-runtime --test fault_injection
cargo test --locked -p runtime-benchmarks
```

The observed results were respectively 20, 3, 32, and 78 passing tests with no failures or ignored CPU tests. They cover the exact homogeneous/mixed adapter policy, mixed hosted-E0 lifecycle, loading-peak budget admission, retained/retry/exhausted fault paths, and synthetic-schema-3/external-schema-4 serialization contracts described below.

These CPU and canonical gates ran locally on Linux 7.1.5-arch1-2 x86_64 with an AMD Ryzen 9 5950X (16 cores/32 threads), Rust 1.96.1, and Cargo 1.96.1. The canonical `cargo xtask verify` gate passed from a previously absent Cargo target directory. Both `wasm32-unknown-unknown` and `thumbv7em-none-eabihf` checks passed for all five domain crates; locked cargo-deny policy passed; and offline Lychee checked 276 links with 0 errors. The exact CUDA compile chain passed with `CUDA_COMPUTE_CAP=120`.

The complete local deterministic hardware matrix passed on NVIDIA GeForce RTX 5070 Ti ordinal 0, driver/KMD 610.43.03, CUDA UMD/toolkit 13.3, `nvcc` 13.3.73, compute capability 12.0, and build cap 120. The Phase 12 GitHub self-hosted workflow has not run, so no remote workflow provenance is claimed. No suitable immutable, license-reviewed external mixed-dtype Llama checkpoint was established; the mixed-layout claim remains limited to deterministic project-authored fixtures.

## Canonical repository gate

The ordinary composite gate is:

```sh
cargo xtask verify
```

`tools/xtask` runs architecture and hygiene policy, formatting, every workspace target, ordinary tests/doctests, mandatory Clippy, API documentation with warnings denied, and benchmark compilation. It does not run statistical measurements or a network-dependent external model.

Useful direct diagnostics are:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo bench --workspace --no-run --locked
git diff --check
```

The mandatory lint profile uses stable selected Clippy policy under `-D warnings`. Nursery remains a separate non-blocking report.

## Shared CPU quality workflow

[`.github/workflows/quality.yml`](../../.github/workflows/quality.yml) is the normal shared-CI gate. It runs on every push and pull request, plus a weekly schedule, using GitHub-hosted Ubuntu 24.04 and the mandatory default CPU feature graph. It does not install a CUDA toolkit, require a driver, enable the `cuda` feature, execute CUDA hardware tests, download an external model, or run performance thresholds.

The `Rust and architecture` job prints the commit and tree, runs the canonical gate from a fresh target with forbidden-tool shims, checks the two portable domain targets, and reports duplicate dependencies. The separate blocking policy job runs locked dependency policy and offline repository-local Markdown links. Scheduled-only nursery and external-link jobs remain non-blocking or outside pull-request determinism as configured in the workflow.

Untrusted pull-request code runs only on GitHub-hosted infrastructure. It cannot schedule the self-hosted CUDA machine because that workflow has no pull-request trigger. Observed run acceptance belongs in [implementation status](implementation-status.md); this document owns the repeatable workflow boundary.

## Historical Phase 10 exact-tree acceptance

Use a fresh dedicated directory beneath the root target without deleting the normal target. The absence check must succeed; if it does not, choose another new child path rather than reusing prior build output:

```sh
test ! -e target/phase10-final
export CARGO_TARGET_DIR="$(git rev-parse --show-toplevel)/target/phase10-final"
```

Run the following sequentially on the clean code-under-test commit:

```sh
cargo metadata --locked --format-version 1 --no-deps

cargo test --locked -p domain-contracts --test allocation
cargo test --locked -p domain-contracts
cargo test --locked -p sampling
cargo test --locked -p inference-runtime
cargo test --locked -p application-runtime
cargo test --locked -p runtime-benchmarks
cargo test --locked -p xtask

cargo clippy --workspace --all-targets --locked -- -D warnings
cargo bench --workspace --no-run --locked
cargo xtask verify
cargo deny --workspace --locked check advisories bans licenses sources
lychee --config lychee.toml --offline '**/*.md'
git diff --check
```

The `domain-contracts` and `sampling` allocation targets are harness-free executables so libtest/process activity cannot overlap their process-global allocator regions. The sampling package’s ordinary matrix test executes every benchmark case once at every vocabulary size and every stop case once; statistical execution is not required for correctness coverage.

### Portable domain checks

Run both targets because allocation-test configuration and portable package manifests are part of the acceptance boundary:

```sh
cargo check --locked \
    --target wasm32-unknown-unknown \
    --lib \
    -p domain-contracts \
    -p tokenization \
    -p context-planner \
    -p sampling \
    -p task-graph

cargo check --locked \
    --target thumbv7em-none-eabihf \
    --lib \
    -p domain-contracts \
    -p tokenization \
    -p context-planner \
    -p sampling \
    -p task-graph
```

### Target and artifact checks

```sh
find . \
    -path './.git' -prune -o \
    -type d -name target -print

test ! -e benchmarks/runtime/Cargo.lock
test ! -d benchmarks/runtime/target

git status --short --untracked-files=all
git status --short --ignored
```

Directories named `target` beneath the root `target/` may be generated documentation artifacts and are valid. A package-local target elsewhere is not. The untracked status must be clean; ignored generated output should be confined to the root target.

## Historical controlled Phase 10 measurements

Run measurements only after exact-tree acceptance on the same clean code-under-test commit. Raw output remains ignored beneath root `target`; only curated reviewed values enter [performance evidence](performance.md).

### Synthetic baseline

```sh
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

The runner writes one JSON report to stdout and progress/summary to stderr. A successful process exit means its fixture, lifecycle, output, cleanup, accounting, unload, shutdown, and join invariants passed; it does not convert synthetic timing into product evidence.

### Focused runtime Criterion measurements

```sh
cargo bench --locked \
    -p runtime-benchmarks \
    --bench runtime \
    -- e0_hosted_checked_prefill/4_tokens

cargo bench --locked \
    -p runtime-benchmarks \
    --bench runtime \
    -- e0_hosted_incremental_decode/1_token_after_2_token_prefill
```

### Focused sampling Criterion measurements

```sh
cargo bench --locked \
    -p sampling \
    --bench sampling_pipeline \
    -- sample_only/default_top_k_top_p/32768

cargo bench --locked \
    -p sampling \
    --bench sampling_pipeline \
    -- restore_and_sample/default_top_k_top_p/32768
```

Do not run the full statistical sampling matrix merely for closure; the ordinary one-shot test owns correctness coverage.

After measurement, verify the commit/tree and clean status again. The JSON’s Git identity and dirty flag should agree with the surrounding Git commands.

## Documentation evidence commit

After measuring Commit A, update only canonical documentation/evidence files and commit them separately as Commit B. State that performance values apply to Commit A and its tree. If Commit B changes only non-executable documentation, Commit A’s measurements remain applicable to the executable tree, but Commit B still requires its own post-commit documentation/canonical gate:

```sh
cargo xtask verify
cargo deny --workspace --locked check advisories bans licenses sources
lychee --config lychee.toml --offline '**/*.md'
git diff --check
```

Record Commit B and its tree in the closure report. A tracked document should not try to contain its own resulting tree hash.

The external closure uses the same discipline with Commit C (runner code under test) and Commit D (documentation-only curated evidence). Build and execute the external binary only from clean Commit C; the raw report must identify Commit C with `dirty: false`. Exact external timing remains attributable to Commit C after Commit D because Commit D changes documentation only. Record Commit D and its post-commit gate in the closure report rather than predicting its identity in tracked documentation.

## Repository architecture and hygiene

Run the policy halves independently when diagnosing:

```sh
cargo xtask architecture
cargo xtask hygiene
```

Architecture validates the typed locked workspace graph and exact role/dependency registries. Hygiene validates tracked operational surfaces, manifests, selected dependencies, benchmark layout, nested locks/targets, generated results, caches, and source-tree artifacts. See [dependency policy](dependency-policy.md).

## Download-free focused CPU validation

Ordinary tests and the canonical gate do not resolve or download external models. Run focused Phase 12 checks sequentially:

```sh
cargo test --locked -p candle-backend --test llama_cpu
cargo test --locked -p inference-runtime --test native_backend_generation
cargo test --locked -p inference-runtime --test fault_injection
cargo test --locked -p application-runtime --lib \
    controlled_mixed_dtype_receipt_allows_bf16_declaration_with_f32_execution
cargo test --locked -p application-runtime --lib \
    unavailable_selected_cuda_blocks_load_without_fallback
cargo test --locked -p runtime-benchmarks
```

The adapter suite is the deterministic owner for homogeneous `{F32}`, `{F16}`, and `{BF16}` plus mixed `{F16, F32}` and `{BF16, F32}` inspection, conversion, execution, exact final/loading-peak planning, supported auxiliary F32 headroom, declaration handling, and rejection of F16/BF16 mixtures, unsupported dtypes, malformed headers, duplicate tensors, invalid bounds/shapes, and overflow. Its host-budget test requires aggregate loading-peak rejection before materialization.

The native E0 suite includes `mixed_f16_f32_fixture_covers_e0_generation_accounting_and_lifecycle`. It compares the independent preparation with the E0 receipt, exercises hosted prefill/decode and generation, restores model-only reserved ownership after release, unloads to exact empty model/request/workspace/cleanup accounting, and completes bounded shutdown and join.

The fault suite deterministically covers invalid preparations, loading-peak admission before materialization, immediate failed-load cleanup, retained ownership and full loading-peak accounting when cleanup fails, retry release exactly once, cleanup exhaustion through shutdown, and no model publication on contract failure. The focused E1 checks keep a controlled mixed declaration distinct from actual F32 execution and prove that an unavailable selected CUDA target blocks load without CPU fallback. The benchmark package tests the report contracts without running a model download or statistical measurement.

These commands establish only their named download-free CPU contracts. They do not establish external artifact availability, language quality, product performance, CUDA compilation or hardware execution, allocation freedom inside upstream libraries, or a full repository gate.

Fixture regeneration is an explicit maintenance operation, not ordinary validation:

```text
cargo test --locked -p candle-backend --test generate_synthetic_fixture -- --ignored --exact regenerate_committed_candle_fixture
```

Run it only when intentionally replacing the fixture, then review generated files and update provenance hashes in the same change.

## Download-free CUDA hardware validation

The canonical repository gate uses the mandatory default CPU feature graph. CUDA validation is a separate, sequential, download-free evidence class. **Compilation proves only feature/API compatibility; it does not prove device execution.** Hardware claims require the later opted-in tests on the exact accepted Linux x86_64 row.

Before the CUDA chain, require an NVIDIA driver that recognizes the intended device, CUDA Toolkit 12.8 or newer, and build capability 120:

```sh
nvidia-smi
nvcc --version
printf 'CUDA_COMPUTE_CAP=%s\n' "${CUDA_COMPUTE_CAP:-unset}"
export CUDA_COMPUTE_CAP=120
```

The exact current workflow compile chain is:

```sh
cargo metadata --locked --format-version 1 --no-deps > /dev/null
cargo xtask architecture
cargo xtask hygiene

cargo check --locked \
    -p candle-backend \
    -p inference-runtime \
    -p application-runtime \
    -p desktop-slint \
    -p runtime-benchmarks \
    --all-targets \
    --features cuda

cargo test --locked \
    -p candle-backend \
    -p inference-runtime \
    -p application-runtime \
    -p runtime-benchmarks \
    --features cuda \
    --no-run

cargo clippy --locked \
    -p candle-backend \
    -p inference-runtime \
    -p application-runtime \
    -p desktop-slint \
    -p runtime-benchmarks \
    --all-targets \
    --features cuda \
    -- -D warnings
```

Do not use workspace `--all-features`; the exact graph is `runtime-benchmarks/cuda -> application-runtime/cuda -> candle-backend/cuda`, `desktop-slint/cuda -> application-runtime/cuda`, plus the development-only `inference-runtime/cuda -> candle-backend/cuda` test edge. This exact compile chain passed locally on 2026-08-08 with `CUDA_COMPUTE_CAP=120`.

Hardware execution is ignored by default and additionally requires `MILKDRIFT_CUDA_TEST=1`. First run the non-ignored explicit-CPU proof in the CUDA build:

```sh
CUDA_VISIBLE_DEVICES=0 \
CUDA_COMPUTE_CAP=120 \
cargo test --release --locked \
    -p candle-backend \
    --features cuda \
    --test llama_cuda \
    -- \
    --exact cuda_enabled_binary_can_explicitly_execute_cpu \
    --nocapture \
    --test-threads=1
```

Then verify and execute the **complete all-ignored adapter matrix**. The expected ignored tests are exactly:

- `cuda_ordinal_zero_executes_fixture_and_matches_cpu_logits`;
- `cuda_homogeneous_bf16_source_executes_as_bf16`;
- `cuda_mixed_f16_f32_executes_as_f16`;
- `cuda_mixed_bf16_f32_executes_as_bf16`.

```sh
CUDA_VISIBLE_DEVICES=0 \
MILKDRIFT_CUDA_TEST=1 \
CUDA_COMPUTE_CAP=120 \
cargo test --release --locked \
    -p candle-backend \
    --features cuda \
    --test llama_cuda \
    -- \
    --ignored \
    --list

for test_name in \
    cuda_ordinal_zero_executes_fixture_and_matches_cpu_logits \
    cuda_homogeneous_bf16_source_executes_as_bf16 \
    cuda_mixed_f16_f32_executes_as_f16 \
    cuda_mixed_bf16_f32_executes_as_bf16
do
    CUDA_VISIBLE_DEVICES=0 \
    MILKDRIFT_CUDA_TEST=1 \
    CUDA_COMPUTE_CAP=120 \
    cargo test --release --locked \
        -p candle-backend \
        --features cuda \
        --test llama_cuda \
        -- \
        --include-ignored \
        --exact "${test_name}" \
        --nocapture \
        --test-threads=1
done
```

The workflow requires the ignored-test list to contain exactly these four names, then runs each name separately with `--include-ignored --exact`. A rename, accidental de-ignoring, or extra ignored test therefore cannot silently reduce or expand the trusted runner workload. The matrix checks actual device/scalar facts and exact final/loading-peak footprints for the mixed fixtures. The complete guarded four-test adapter matrix passed locally on 2026-08-08 on the exact RTX 5070 Ti row.

The E0 hardware target proves verified preparation/receipt/snapshot identity, mixed F16/F32 execution, scheduled prefill and incremental decode, host-side sampling from transferred logits, sequence cleanup, model unload, and zero post-unload ownership accounting:

```sh
CUDA_VISIBLE_DEVICES=0 \
MILKDRIFT_CUDA_TEST=1 \
CUDA_COMPUTE_CAP=120 \
cargo test --release --locked \
    -p inference-runtime \
    --features cuda \
    --test native_backend_generation \
    -- \
    --ignored \
    --list

CUDA_VISIBLE_DEVICES=0 \
MILKDRIFT_CUDA_TEST=1 \
CUDA_COMPUTE_CAP=120 \
cargo test --release --locked \
    -p inference-runtime \
    --features cuda \
    --test native_backend_generation \
    -- \
    --include-ignored \
    --exact candle_mixed_cuda_fixture_covers_e0_generation_accounting_and_lifecycle \
    --nocapture \
    --test-threads=1
```

Run deterministic failed-load ownership/accounting faults and the explicit application no-fallback policy separately. These tests are download-free; their use in the CUDA job proves the exact feature graph still preserves the same cleanup and selection contracts:

```sh
CUDA_VISIBLE_DEVICES=0 \
MILKDRIFT_CUDA_TEST=1 \
CUDA_COMPUTE_CAP=120 \
cargo test --release --locked \
    -p inference-runtime \
    --features cuda \
    --test fault_injection \
    -- \
    --nocapture \
    --test-threads=1

CUDA_VISIBLE_DEVICES=0 \
MILKDRIFT_CUDA_TEST=1 \
CUDA_COMPUTE_CAP=120 \
cargo test --release --locked \
    -p application-runtime \
    --features cuda \
    --lib \
    -- \
    --exact runtime::tests::devices::unavailable_selected_cuda_blocks_load_without_fallback \
    --nocapture \
    --test-threads=1
```

The ignored E1 fixture test is separately guarded by `MILKDRIFT_CUDA_TEST=1`. It exercises discovery, explicit E1 CUDA selection, fixture load, configuration-declared versus actual execution scalar truth, selected-versus-receipt-verified actual device reporting, unload, and bounded shutdown:

```sh
CUDA_VISIBLE_DEVICES=0 \
MILKDRIFT_CUDA_TEST=1 \
CUDA_COMPUTE_CAP=120 \
cargo test --release --locked \
    -p application-runtime \
    --features cuda \
    --lib \
    -- \
    --ignored \
    --list

CUDA_VISIBLE_DEVICES=0 \
MILKDRIFT_CUDA_TEST=1 \
CUDA_COMPUTE_CAP=120 \
cargo test --release --locked \
    -p application-runtime \
    --features cuda \
    --lib \
    -- \
    --include-ignored \
    --exact runtime::tests::devices::cuda_fixture_load_reports_the_selected_and_actual_e1_device \
    --nocapture \
    --test-threads=1
```

Do not run the ignored hardware tests in ordinary CPU CI, do not use workspace `--all-features`, and do not interpret source presence, test listing, or compilation as hardware execution. On 2026-08-08, the complete local Phase 12 matrix passed on NVIDIA GeForce RTX 5070 Ti ordinal 0, driver/KMD 610.43.03, CUDA UMD/toolkit 13.3, `nvcc` 13.3.73, compute capability 12.0, and build cap 120. The explicit CPU-in-CUDA adapter test passed; all four ignored adapter tests passed; the mixed F16/F32 hosted-E0 lifecycle passed; all 32 fault tests passed under the CUDA feature graph; E1 explicit no-fallback passed; and the guarded E1 CUDA fixture lifecycle passed. These are local deterministic fixture results. The Phase 12 GitHub self-hosted workflow has not run and remains unclaimed.

## Self-hosted CUDA hardware correctness gate

[`.github/workflows/cuda-hardware.yml`](../../.github/workflows/cuda-hardware.yml) is a separate download-free correctness gate for the committed fixture. The maintained repository runner is named `hart-desk-rtx5070ti` and is selected with all registered labels: `self-hosted`, `Linux`, `X64`, and the dedicated `milkdrift-cuda-5070ti` label. If that runner is offline, removed, or under maintenance, restore its exact registration rather than weakening the job to generic `self-hosted` routing.

The security boundary is deliberate:

- triggers are pushes to `main` filtered by `.github/workflows/**`, `.gitignore`, `.cargo/**`, root `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `crates/**`, `benchmarks/runtime/**`, and `tools/xtask/**`, plus owner-dispatched runs of the `main` ref only; documentation-only changes do not schedule push runs;
- neither `pull_request` nor `pull_request_target` can schedule the machine, so fork or other untrusted PR code is never checked out there;
- path-filtered pushes trust code already landed on `main`; anyone able to land matching code on `main` is inside the machine-execution boundary, so repository write and branch controls remain part of runner security;
- workflow permissions are `contents: read`, checkout credentials are not persisted, and no repository secret or command input is used;
- one repository-wide concurrency group prevents overlapping Milkdrift GPU jobs;
- Cargo is offline after checkout, the target directory is isolated beneath `$RUNNER_TEMP`, and an `always()` final step removes it without `cargo clean`.

The runner administrator maintains `/var/tmp/milkdrift-cargo-home` as a dependency-only Cargo cache seeded out of band from a trusted locked checkout. Refresh that cache before a trusted dependency update reaches `main`; do not expose credentials in it or relax the workflow's offline setting. The job fails before compilation when the maintained cache is missing or inaccessible.

The job validates the exact RTX 5070 Ti / CUDA ordinal 0 / compute capability 12.0 / Toolkit 12.8+ / build-cap-120 matrix, then runs metadata, architecture, hygiene, the five-package CUDA check/Clippy graph, four-package test compilation, guarded explicit CPU-in-CUDA execution, the guarded exact four-test homogeneous/mixed adapter matrix, the guarded renamed mixed hosted-E0 lifecycle, deterministic cleanup faults, guarded exact no-fallback, and guarded E1 device/scalar lifecycle. It does not run TinyLlama, Hugging Face resolution, Criterion, elapsed-time thresholds, Slint interaction, or any arbitrary model. Its Toolkit 12.8+ fixture preflight range does not broaden product support beyond the exact locally observed Phase 12 Toolkit 13.3 row. The local command matrix passed, but the updated Phase 12 GitHub workflow has not run; workflow-source presence is not remote execution evidence.

## Controlled CPU and CUDA external product evidence

`runtime-benchmarks` owns the single external E1 orchestration path for both devices. The external binary requires exactly `--device cpu` or `--device cuda:0`, never substitutes a device, and never falls back to CPU. It fixes `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable revision `fe8a4ea1ffedaf415f4da2f062534de366a451e6`; repository and revision overrides are rejected. The pinned revision's [model-card metadata](https://huggingface.co/TinyLlama/TinyLlama-1.1B-Chat-v1.0/raw/fe8a4ea1ffedaf415f4da2f062534de366a451e6/README.md) declares `apache-2.0`. Record that upstream declaration and source without making a broader legal conclusion.

That exact TinyLlama profile is configuration-declared BF16 and observed homogeneous `{BF16}`. It remains useful for the established product lifecycle, chat, timing, and CPU/CUDA comparison, but it is **not mixed-layout checkpoint evidence** and cannot substitute for an external F16/F32 or BF16/F32 checkpoint.

This procedure is the only ordinary exception to the download-free rule. It requires explicit authorization to contact Hugging Face for that exact model/revision. Shared CI and ordinary tests compile the CPU path but never execute the external binary, contact the network, require its cache, load TinyLlama, or require CUDA hardware. No current-tree external execution result is claimed.

### Preflight and code-under-test identity

Commit the code and procedure changes as the clean code-under-test commit before measuring. Record its commit and tree, then run the ordinary gate:

```sh
git status --short --untracked-files=all
git rev-parse HEAD
git rev-parse 'HEAD^{tree}'

export CARGO_TARGET_DIR="$(git rev-parse --show-toplevel)/target"

cargo xtask verify
```

Require physical CUDA ordinal 0 to be the intended RTX 5070 Ti, CUDA Toolkit 12.8 or newer, `CUDA_COMPUTE_CAP=120`, sufficient free host/device memory and disk, and no competing compiler or model process:

```sh
nvidia-smi
nvcc --version
printf 'CUDA_COMPUTE_CAP=%s\n' "${CUDA_COMPUTE_CAP:-unset}"
free -h
df -h target
ps -eo pid,comm,rss --sort=-rss | head -20
```

Stop rather than measuring when the matrix or resources do not match. Use one Cargo process and one model process at a time. Do not run `cargo clean`.

### Separate release builds and controlled runs

Use separate root-target children so the CPU and CUDA executables cannot be confused. The explicit cache must already exist. A cache outside the repository is also allowed; a cache inside the source tree but outside root `target/` is rejected.

```sh
mkdir -p target/phase12-cpu
mkdir -p target/phase12-cuda
mkdir -p target/phase12-evidence

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
```

Execute the produced binaries directly and sequentially so no compiler process overlaps loaded-model ownership:

```sh
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

The executable writes no result file itself: stdout is exactly one structured report, stderr carries progress and concise diagnostics, and the redirect owns the ignored raw artifact. Do not edit generated JSON.

The primary cycle on each device must prove the exact model/revision, non-empty compatible chat, one direct-completion warmup, three measured 32-token completions, matching request identities, exact terminal/released outcomes and usage, one cancellation after decoded progress, zero cleanup-pending/exhausted events, synchronized zero-cancellation unload, and bounded shutdown. CUDA additionally performs two reduced stability cycles containing load, direct generation/release, separate cancellation/release, unload, synchronization, shutdown, and owner drop. Together with the primary cycle this is three complete CUDA lifecycle cycles; warmup timing remains separate from measured samples.

Review both current external schema-4 reports programmatically without printing generated text or token identifiers. Require:

- the same clean code-under-test Git commit/tree and `dirty: false`;
- the same exact model, revision, fixed artifact layout, prompt hashes, sampling settings, and primary workload;
- configuration-declared `BF16` and observed tensor scalars exactly `["BF16"]`, proving this profile is homogeneous;
- planned CPU F32 versus planned CUDA BF16 execution, with actual E1/E0-verified execution matching each plan;
- requested device, prepared-load device, selected E1 device, and actual loaded E0 device all CPU in one report and all CUDA ordinal 0 in the other;
- `prepared_load.exact_final_footprint` and `prepared_load.loading_peak_footprint` as separate deterministic plan quantities;
- `e1_load_accepted: true` and `e0_reserved_ownership_observed: false`, because this independent observer preparation is not the product worker's direct E0 snapshot;
- process RSS/HWM only under `process_memory`, and whole-device CUDA total/free/used only under `whole_device_cuda_memory`;
- `cuda_enabled: false` for the CPU build and `cuda_enabled: true` for the CUDA build;
- RTX 5070 Ti identity, driver/toolkit metadata, compute capability 12.0, and build target 120 only in the CUDA report;
- complete cancellation, unload, shutdown, workspace-removal, and three-cycle CUDA stability results.

The exact final footprint is the prepared transaction's deterministic post-load tensor ownership. The loading peak is the separate aggregate admission requirement during materialization. Neither is process RSS or physical whole-device memory. Public E1 accepts the E0 load contract but does not expose a same-worker E0 `RuntimeSnapshot`, so external schema 4 deliberately does not claim direct E0 reserved ownership or post-unload zero accounting. The opted-in E0 fixture test remains the owner for exact zero model/request/workspace/cleanup accounting.

Each CUDA cycle establishes a new whole-device pre-load baseline. Interpret post-unload and post-owner-drop retained deltas with absolute observations; desktop or other GPU activity can perturb either. Safe Candle `discover_device` calls and their temporary contexts remain bounded cold observations recorded as audit evidence, never per-token work or a threshold.

No schema-4 product report has been accepted on the Phase 12 closure tree, so no new measured values replace the historical Phase 10/11 evidence in [performance evidence](performance.md#external-product-evidence).

### External mixed-checkpoint evidence gap

No immutable, license-reviewed, Llama-compatible mixed-dtype checkpoint with a suitable direct-completion profile has been established for current Phase 12 evidence. Therefore no external mixed CPU or CUDA compatibility result is claimed. The deterministic project-authored adapter/E0/E1 fixtures own the current reviewed layout evidence.

A missing network route, unavailable Hugging Face service, gated-repository credential, or absent local cache is an acquisition/precondition failure. It must be reported separately and must not be classified as model incompatibility. Conversely, only an acquired immutable artifact that reaches inspection/preparation and fails the reviewed product contract can provide incompatibility evidence.

### Manual Slint acceptance

Use an isolated application data root that does not exist before the session:

```sh
test ! -e target/phase12-slint-data
mkdir -p target/phase12-slint-data

XDG_DATA_HOME="$PWD/target/phase12-slint-data" \
CUDA_COMPUTE_CAP=120 \
cargo run --release --locked \
    -p desktop-slint \
    --features cuda
```

A human must visibly verify all of the following; compilation or process launch is not graphical acceptance:

1. CPU and CUDA 0 are shown.
2. CUDA 0 can be selected explicitly.
3. The exact TinyLlama revision resolves and loads.
4. The UI reports actual CUDA execution.
5. One chat message produces streamed output.
6. A second generation can be cancelled after progress.
7. Unload returns controls to idle.
8. Closing the window completes bounded shutdown.

Record only the behavioral result. Do not retain screenshots containing generated private text unless they were deliberately reviewed.

### Final CPU and CUDA acceptance procedure

The following commands remain the canonical reproducibility procedure for the locally passed Phase 12 closure classes. Run the ordinary CPU gates sequentially:

```sh
cargo xtask verify

cargo deny --workspace --locked check \
    advisories bans licenses sources

lychee --config lychee.toml --offline '**/*.md'

git diff --check
```

Run the exact CUDA feature matrix without `--all-features`:

```sh
export CUDA_COMPUTE_CAP=120

cargo check --locked \
    -p candle-backend \
    -p inference-runtime \
    -p application-runtime \
    -p desktop-slint \
    -p runtime-benchmarks \
    --all-targets \
    --features cuda

cargo test --locked \
    -p candle-backend \
    -p inference-runtime \
    -p application-runtime \
    -p runtime-benchmarks \
    --features cuda \
    --no-run

cargo clippy --locked \
    -p candle-backend \
    -p inference-runtime \
    -p application-runtime \
    -p desktop-slint \
    -p runtime-benchmarks \
    --all-targets \
    --features cuda \
    -- -D warnings
```

Run every explicitly opted-in CUDA hardware test listed in [download-free CUDA hardware validation](#download-free-cuda-hardware-validation). The homogeneous TinyLlama schema-4 CPU/CUDA baseline remains an optional current product regression and is not mixed evidence; run it only with explicit network authorization. Absence of a suitable reviewed external mixed profile remains a documented evidence gap rather than a reason to substitute TinyLlama. Confirm artifact hygiene:

```sh
test ! -e benchmarks/runtime/Cargo.lock
test ! -d benchmarks/runtime/target

find . \
    -path './.git' -prune -o \
    -type d -name target -print

git status --short --untracked-files=all
git status --short --ignored
```

Only root `target/` and its descendants may contain build artifacts, generated CUDA kernels, model cache, temporary application state, and raw evidence. After reviewing successful results, update canonical evidence/status/execution documents in the same coherent closure commit and re-run the documentation and canonical gates before creating it. Push only when requested; local success and an observed run on an earlier executable tree must remain separately identified.

## Dependency, link, and graph audits

```sh
cargo deny --workspace --locked check advisories bans licenses sources
lychee --config lychee.toml --offline '**/*.md'
```

Useful audit reports are:

```sh
cargo metadata --locked --format-version 1
cargo tree --workspace --locked
cargo tree -d --locked
cargo tree -e features --locked
```

Duplicate versions are audit inputs rather than automatic failures. Interpret them against [dependency policy](dependency-policy.md).
