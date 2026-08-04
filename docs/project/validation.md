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

## Phase 10 exact-tree acceptance

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

## Controlled Phase 10 measurements

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

## Download-free focused validation

Ordinary tests and the canonical gate do not resolve or download external models. Useful focused commands are:

```sh
cargo test --locked -p candle-backend
cargo test --locked -p inference-runtime --test native_backend_generation
cargo test --locked -p application-runtime
cargo test --locked -p desktop-slint
```

These tests exercise real-adapter fixture execution plus E0/E1/frontend lifecycle behavior, not external artifact availability, language quality, product performance, GPU execution, or allocation freedom inside upstream libraries.

Fixture regeneration is an explicit maintenance operation, not ordinary validation:

```text
cargo test --locked -p candle-backend --test generate_synthetic_fixture -- --ignored --exact regenerate_committed_candle_fixture
```

Run it only when intentionally replacing the fixture, then review generated files and update provenance hashes in the same change.

## Phase 11 lower-layer CUDA validation

The canonical gate remains CPU-only. CUDA validation is separate, sequential, Linux x86_64 only, and requires no model download. Before any CUDA Cargo command, require an NVIDIA driver that recognizes the intended device, CUDA Toolkit 12.8 or newer, and the Blackwell build capability:

```sh
nvidia-smi
nvcc --version
printf 'CUDA_COMPUTE_CAP=%s\n' "${CUDA_COMPUTE_CAP:-unset}"
```

For the first executed target, set `CUDA_COMPUTE_CAP=120` for every Cargo invocation. Compile the adapter feature without enabling workspace-wide features:

```sh
CUDA_COMPUTE_CAP=120 cargo check --locked \
    -p candle-backend \
    --all-targets \
    --features cuda

CUDA_COMPUTE_CAP=120 cargo test --locked \
    -p candle-backend \
    --features cuda \
    --no-run

CUDA_COMPUTE_CAP=120 cargo clippy --locked \
    -p candle-backend \
    --all-targets \
    --features cuda \
    -- -D warnings
```

Compile the E1 and Slint opt-in graphs independently. These commands exercise the exact `application-runtime/cuda -> candle-backend/cuda` and `desktop-slint/cuda -> application-runtime/cuda` forwarding paths without enabling workspace-wide features:

```sh
CUDA_COMPUTE_CAP=120 cargo check --locked \
    -p application-runtime \
    --all-targets \
    --features cuda

CUDA_COMPUTE_CAP=120 cargo test --locked \
    -p application-runtime \
    --features cuda \
    --no-run

CUDA_COMPUTE_CAP=120 cargo clippy --locked \
    -p application-runtime \
    --all-targets \
    --features cuda \
    -- -D warnings

CUDA_COMPUTE_CAP=120 cargo check --locked \
    -p desktop-slint \
    --all-targets \
    --features cuda

CUDA_COMPUTE_CAP=120 cargo clippy --locked \
    -p desktop-slint \
    --all-targets \
    --features cuda \
    -- -D warnings
```

A CUDA-enabled binary must also prove that explicit CPU selection remains usable:

```sh
CUDA_COMPUTE_CAP=120 cargo test --locked \
    -p candle-backend \
    --features cuda \
    --test llama_cuda
```

Hardware execution is ignored by default and additionally requires `MILKDRIFT_CUDA_TEST=1`. Run the adapter proofs separately so no CUDA fixture tests overlap. They establish that explicit CPU execution remains usable in a CUDA build, CUDA 0 matches CPU fixture logits, and BF16 source weights execute as BF16 on the required CUDA device:

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

CUDA_VISIBLE_DEVICES=0 \
MILKDRIFT_CUDA_TEST=1 \
CUDA_COMPUTE_CAP=120 \
cargo test --release --locked \
    -p candle-backend \
    --features cuda \
    --test llama_cuda \
    -- \
    --ignored \
    --exact cuda_ordinal_zero_executes_fixture_and_matches_cpu_logits \
    --nocapture \
    --test-threads=1

CUDA_VISIBLE_DEVICES=0 \
MILKDRIFT_CUDA_TEST=1 \
CUDA_COMPUTE_CAP=120 \
cargo test --release --locked \
    -p candle-backend \
    --features cuda \
    --test llama_cuda \
    -- \
    --ignored \
    --exact cuda_bf16_source_executes_as_bf16 \
    --nocapture \
    --test-threads=1
```

The E0 hardware target proves verified receipt/snapshot identity, scheduled prefill and incremental decode, CPU-side sampling from exact transferred logits, sequence cleanup, model unload, and zero post-unload accounting:

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
    --exact candle_cuda_fixture_covers_e0_generation_accounting_and_lifecycle \
    --nocapture \
    --test-threads=1
```

The ignored E1 fixture test is separately guarded by `MILKDRIFT_CUDA_TEST=1`. It exercises discovery, explicit E1 CUDA selection, fixture load, source/execution scalar truth, selected-versus-receipt-verified actual device reporting, unload, and bounded shutdown:

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
    --exact runtime::tests::devices::cuda_fixture_load_reports_the_selected_and_actual_e1_device \
    --nocapture \
    --test-threads=1
```

Do not run these tests in ordinary CPU CI, do not use `--all-features` for the workspace, and do not interpret compilation as hardware execution evidence. Executed evidence must name the exact commit, hardware, and local or Actions run rather than being inferred from this procedure.

## Self-hosted CUDA hardware correctness gate

[`.github/workflows/cuda-hardware.yml`](../../.github/workflows/cuda-hardware.yml) is a separate download-free correctness gate for the committed fixture. The maintained repository runner is named `hart-desk-rtx5070ti` and is selected with all registered labels: `self-hosted`, `Linux`, `X64`, and the dedicated `milkdrift-cuda-5070ti` label. If that runner is offline, removed, or under maintenance, restore its exact registration rather than weakening the job to generic `self-hosted` routing.

The security boundary is deliberate:

- triggers are path-filtered pushes to `main` and owner-dispatched runs of the `main` ref only;
- neither `pull_request` nor `pull_request_target` can schedule the machine, so fork or other untrusted PR code is never checked out there;
- workflow permissions are `contents: read`, checkout credentials are not persisted, and no repository secret or command input is used;
- one repository-wide concurrency group prevents overlapping Milkdrift GPU jobs;
- Cargo is offline after checkout, the target directory is isolated beneath `$RUNNER_TEMP`, and an `always()` final step removes it without `cargo clean`.

The runner administrator maintains `/var/tmp/milkdrift-cargo-home` as a dependency-only Cargo cache seeded out of band from a trusted locked checkout. Refresh that cache before a trusted dependency update reaches `main`; do not expose credentials in it or relax the workflow's offline setting. The job fails before compilation when the maintained cache is missing or inaccessible.

The job validates the exact RTX 5070 Ti / CUDA ordinal 0 / compute capability 12.0 / Toolkit 12.8+ / build-cap-120 matrix, then runs metadata, architecture, hygiene, the five-package CUDA check and Clippy graph, and the sequential adapter, hosted-E0, and E1 fixture tests listed above. It does not run TinyLlama, Hugging Face resolution, Criterion, elapsed-time thresholds, Slint interaction, or any arbitrary model.

## External CPU product baseline

The former CPU-only procedure is superseded by the device-parameterized [Phase 11 controlled CPU and CUDA product evidence](#phase-11-controlled-cpu-and-cuda-product-evidence) procedure below. Existing CPU evidence links retain this heading for historical continuity.

## Phase 11 controlled CPU and CUDA product evidence

`runtime-benchmarks` owns the single external E1 orchestration path for both devices. The external binary requires exactly `--device cpu` or `--device cuda:0`, never substitutes a device, and never falls back to CPU. It fixes `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable revision `fe8a4ea1ffedaf415f4da2f062534de366a451e6`; repository and revision overrides are rejected. The pinned revision's [model-card metadata](https://huggingface.co/TinyLlama/TinyLlama-1.1B-Chat-v1.0/raw/fe8a4ea1ffedaf415f4da2f062534de366a451e6/README.md) declares `apache-2.0`. Record that upstream declaration and source without making a broader legal conclusion.

This procedure is the only ordinary exception to the download-free rule. It requires explicit authorization to contact Hugging Face for that exact model/revision. Shared CI and ordinary tests compile the CPU path but never execute the external binary, contact the network, require its cache, load TinyLlama, or require CUDA hardware.

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
mkdir -p target/phase11-cpu
mkdir -p target/phase11-cuda
mkdir -p target/phase11-evidence

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
```

Execute the produced binaries directly and sequentially so no compiler process overlaps model residency:

```sh
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

The executable writes no result file itself: stdout is exactly one structured report, stderr carries progress and concise diagnostics, and the redirect owns the ignored raw artifact. Do not edit generated JSON.

The primary cycle on each device must prove the exact model/revision, non-empty compatible chat, one direct-completion warmup, three measured 32-token completions, matching request identities, exact terminal/released outcomes and usage, one cancellation after decoded progress, zero cleanup-pending/exhausted events, synchronized zero-cancellation unload, and bounded shutdown. CUDA additionally performs two reduced stability cycles containing load, direct generation/release, separate cancellation/release, unload, synchronization, shutdown, and owner drop. Together with the primary cycle this is three complete CUDA lifecycle cycles; warmup timing remains separate from measured samples.

Review both schema-3 reports programmatically without printing generated text or token identifiers. Require:

- the same clean code-under-test Git commit/tree and `dirty: false`;
- the same exact model, revision, prompt hashes, sampling settings, and primary workload;
- requested, selected E1, and actual loaded E0 identities all CPU in one report and all CUDA ordinal 0 in the other;
- `cuda_enabled: false` for the CPU build and `cuda_enabled: true` for the CUDA build;
- explicit BF16 `source_scalar` in both reports, CPU F32 `execution_scalar`, and CUDA BF16 `execution_scalar`;
- RTX 5070 Ti identity, driver/toolkit metadata, compute capability 12.0, and build target 120 only in the CUDA report;
- complete cancellation, unload, shutdown, workspace-removal, and three-cycle CUDA stability results.

CUDA total/free/used values are safe driver observations for the whole device, not process-attributed usage. Every cycle establishes a new immediately-pre-load baseline. Interpret post-unload and post-owner-drop retained deltas together with the absolute observations; desktop or other GPU activity can perturb either. The external report records `accounted_footprint` as an independent public adapter plan with validated E1 acceptance of the E0 load contract, but public E1 does not expose a same-worker E0 reservation or post-unload `RuntimeSnapshot`. Do not synthesize those fields or call the E1 state a direct accounting snapshot. Execute and record the direct opted-in E0 CUDA hardware snapshot test in the lower-layer section separately; that test owns exact zero model/request/workspace/cleanup accounting evidence.

Schema 3 also records the exact number of safe Candle `discover_device` calls. Each call initializes a temporary Candle CUDA device and cudarc context; these observations are bounded to cold identity/resource checkpoints and never occur per token. Review the count as context-churn audit evidence only, not as a timing or performance threshold.

Exact measured values belong only in [performance evidence](performance.md#external-product-evidence).

### Manual Slint acceptance

Use an isolated application data root that does not exist before the session:

```sh
test ! -e target/phase11-slint-data
mkdir -p target/phase11-slint-data

XDG_DATA_HOME="$PWD/target/phase11-slint-data" \
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

### Final Phase 11 validation

Run the ordinary CPU gates sequentially:

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

Run every explicitly opted-in CUDA hardware test listed in [Phase 11 lower-layer CUDA validation](#phase-11-lower-layer-cuda-validation), then run the controlled external CPU and CUDA baselines above. Confirm artifact hygiene:

```sh
test ! -e benchmarks/runtime/Cargo.lock
test ! -d benchmarks/runtime/target

find . \
    -path './.git' -prune -o \
    -type d -name target -print

git status --short --untracked-files=all
git status --short --ignored
```

Only root `target/` and its descendants may contain build artifacts, generated CUDA kernels, model cache, temporary application state, and raw evidence. After reviewing successful results, update canonical evidence/status/execution documents in a separate documentation-only evidence commit. Re-run the documentation and canonical gates on that commit. Push only when requested, then observe the actual GitHub Actions run; local success is not remote acceptance.

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
