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

Use one Cargo process at a time. Do not run `cargo clean`. Keep generated output under root `target/`, one explicitly named isolated `CARGO_TARGET_DIR`, or outside the repository. Clean acceptance must verify that it did not accidentally create both an isolated target and root `target/`.

## Phase 12 closure evidence

Phase 12 Segment 1 is commit `58490fe693fef7a2635956181088664cd90685e8`; Segment 2 is commit `12510695aa29be6a2665dbf3777cccbb8172c2d1`; Segment 3 is closure commit `181a069ce81525e9c144fe8de051ced8e3c0b9d7`, tree `310e437c0729f51fe6c0ba3dcb5fbf9f1935a80f`.

On 2026-08-08, these sequential download-free CPU commands passed on the closure tree:

```sh
cargo test --locked -p candle-backend --test llama_cpu
cargo test --locked -p inference-runtime --test native_backend_generation
cargo test --locked -p inference-runtime --test fault_injection
cargo test --locked -p runtime-benchmarks
```

The observed results were respectively 20, 3, 32, and 78 passing tests with no failures or ignored CPU tests. They cover the exact homogeneous/mixed adapter policy, mixed hosted-E0 lifecycle, loading-peak budget admission, retained/retry/exhausted fault paths, and synthetic-schema-3/external-schema-4 serialization contracts described below.

These CPU and canonical gates ran locally on Linux 7.1.5-arch1-2 x86_64 with an AMD Ryzen 9 5950X (16 cores/32 threads), Rust 1.96.1, and Cargo 1.96.1. The canonical `cargo xtask verify` gate passed from a previously absent Cargo target directory. Both `wasm32-unknown-unknown` and `thumbv7em-none-eabihf` checks passed for all five domain crates; locked cargo-deny policy passed; and offline Lychee checked 276 links with 0 errors. The exact CUDA compile chain passed with `CUDA_COMPUTE_CAP=120`.

The complete local deterministic hardware matrix passed on NVIDIA GeForce RTX 5070 Ti ordinal 0, driver/KMD 610.43.03, CUDA UMD/toolkit 13.3, `nvcc` 13.3.73, compute capability 12.0, and build cap 120. At commit-authoring time the GitHub workflow had not run; after push, self-hosted CUDA [run 31281013243](https://github.com/hartolit/milkdrift/actions/runs/31281013243) completed successfully on the exact closure commit. Hosted Quality [run 31281013257](https://github.com/hartolit/milkdrift/actions/runs/31281013257) passed its canonical native step and then exhausted disk before portable evidence could complete. That is infrastructure history, not a WASM/product failure. No suitable immutable, license-reviewed external mixed-dtype Llama checkpoint was established; the mixed-layout claim remains limited to deterministic project-authored fixtures.

## Pristine artifact-loading amendment evidence

On 2026-08-10, the uncommitted artifact-loading amendment passed these default-feature targeted commands sequentially:

```sh
cargo fmt --all -- --check
cargo check --locked \
    -p candle-backend -p hf-hub-adapter -p inference-runtime \
    -p application-runtime -p runtime-benchmarks --all-targets
cargo test --locked -p candle-backend
cargo test --locked -p hf-hub-adapter
cargo test --locked -p inference-runtime --test native_backend_generation
cargo test --locked -p application-runtime
cargo test --locked -p runtime-benchmarks
cargo clippy --locked \
    -p candle-backend -p hf-hub-adapter -p inference-runtime \
    -p application-runtime -p runtime-benchmarks --all-targets -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --locked \
    -p candle-backend -p hf-hub-adapter -p inference-runtime \
    -p application-runtime --no-deps
```

Observed passing counts were 24 Candle unit tests, 1 fixture-generator consistency test, 25 Candle CPU integration tests, 23 Hub tests, 3 native hosted-E0 tests, 79 application-runtime tests, and 78 runtime-benchmark tests. The fixture generator remained intentionally ignored. These commands did not run the complete workspace/canonical gate, portable-domain cross-targets, dependency policy, link checking, network resolution, an external checkpoint, or performance measurements.

The exact amended CUDA metadata/architecture/hygiene/check/test-compilation/Clippy graph also passed with `CUDA_COMPUTE_CAP=120` for `candle-backend`, `hf-hub-adapter`, `inference-runtime`, `application-runtime`, `desktop-slint`, and `runtime-benchmarks` as applicable. Local release-mode hardware execution then passed on NVIDIA GeForce RTX 5070 Ti ordinal 0, driver/KMD 610.43.03, CUDA UMD/toolkit 13.3, `nvcc` 13.3.73, compute capability 12.0, and build cap 120:

- explicit CPU execution from a CUDA-enabled Candle binary;
- exactly four guarded adapter tests: F32, homogeneous BF16, mixed F16/F32, and mixed BF16/F32;
- the guarded mixed F16/F32 hosted-E0 generation/accounting/lifecycle test;
- all 32 deterministic E0 fault-injection tests under the CUDA graph;
- E1 explicit unavailable-CUDA no-fallback;
- the guarded E1 CUDA fixture load/device/scalar/unload/shutdown lifecycle.

This is local hardware evidence for the amended working tree, not a GitHub self-hosted run, generic NVIDIA support, or external-checkpoint evidence. Re-run after the coherent commit if clean-commit provenance is required.

## Foundation closure 02 local evidence

On 2026-08-13, the Candle sequence-accounting closure passed its required focused
matrix sequentially with `CARGO_INCREMENTAL=0` in the previously absent isolated
target `/tmp/milkdrift-foundation-closure-02-target`:

```sh
cargo fmt --all -- --check
cargo check --locked -p domain-contracts -p candle-backend -p inference-runtime --all-targets
cargo test --locked -p domain-contracts
cargo test --locked -p candle-backend --test llama_cpu
cargo test --locked -p candle-backend
cargo test --locked -p inference-runtime --test runtime
cargo test --locked -p inference-runtime --test generation
cargo test --locked -p inference-runtime --test fault_injection
cargo test --locked -p inference-runtime --test native_backend_generation
cargo clippy --locked -p domain-contracts -p candle-backend -p inference-runtime --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps -p domain-contracts -p candle-backend -p inference-runtime
cargo xtask architecture
cargo xtask hygiene
git diff --check
```

Observed tests were 12 domain contract tests; 40 Candle unit tests; 1 passing
fixture-consistency test with its explicit source-maintenance generator ignored;
27 Candle CPU integration tests; and respectively 11, 25, 47, and 3 passing E0
runtime, generation, fault-injection, and native-backend integration tests. The
focused all-target check, strict Clippy, warning-denied rustdoc, architecture,
hygiene, formatting, and whitespace checks also passed.

Because the generic ownership field rename also updates the E1 evidence mapping,
supplemental locked `application-runtime` all-target check, complete package test,
and strict all-target Clippy commands passed. The package test observed 89 unit,
3 state-integration, and 1 doctest pass.

The dedicated CUDA test target is registered behind `cuda-hardware-tests` and
the architecture policy accepts its feature graph. An exact
`CUDA_COMPUTE_CAP=120 cargo check --locked -p candle-backend --features
cuda-hardware-tests --test cuda_hardware` attempt stopped in the `cudarc 0.19.8`
build script because `nvcc --version` could not execute. `nvcc`, `nvidia-smi`,
and NVIDIA device nodes are absent on this host, so no Rust CUDA compilation or
hardware execution is claimed. Prompt 6 owns remote CUDA acceptance. These are
source-tree results; the completion report records the resulting commit and tree.

## Foundation closure 05 local evidence

On 2026-08-13, clean commit
`b1f7e90b1ba67f1cf968d773052b5062ef8cbbb9`, tree
`fcb3ee6fa00243734abd74b64218aa0db2e340c1`, passed the complete local
CPU/portable closure matrix on the UM790 Pro. This is executable-source evidence;
the documentation-only closure commit cannot be named by the tracked document
that creates it and is reported separately with its post-commit checks.

The host was Linux `7.1.6-arch1-1` x86_64, Rust 1.96.1, Cargo 1.96.1, AMD Ryzen
9 7940HS (8 physical/16 logical cores), 60 GiB reported memory, 4 GiB zram swap,
and Btrfs with 782 GiB initially free. AMD Radeon 780M was the only display
adapter observed. `nvidia-smi` and `nvcc` were absent, so CUDA was not attempted.
Every clean evidence build set `CARGO_INCREMENTAL=0`; heavy Cargo builds ran
sequentially.

The fast matrix passed:

```sh
git status --short
git diff --check
cargo metadata --locked --format-version 1 --no-deps
cargo fmt --all -- --check
cargo xtask architecture
cargo xtask hygiene
```

The focused matrix passed in one fresh target, including three consecutive E1
runs:

```sh
cargo test --locked -p application-runtime
cargo test --locked -p redb-storage
cargo test --locked -p application-runtime
cargo test --locked -p application-runtime
cargo test --locked -p domain-contracts
cargo test --locked -p candle-backend --test llama_cpu
cargo test --locked -p candle-backend
cargo test --locked -p inference-runtime --test runtime
cargo test --locked -p inference-runtime --test generation
cargo test --locked -p inference-runtime --test fault_injection
cargo test --locked -p inference-runtime --test native_backend_generation
cargo test --locked -p task-graph
cargo test --locked -p corrective-workflow
cargo test --locked -p xtask
```

These commands reported 564 passing libtest/doctest cases, no failures, and one
ignored explicit fixture-regeneration maintenance test. The domain/task-graph
harness-free allocation executables also succeeded. The matrix covers the public
headless E1 boundary and redb migrations; homogeneous and mixed CPU fixtures;
independent scalar facts; sequence plan/create/prefill/decode/destroy and logical
reservation; output backpressure/cancellation; ordinary unload; failed-load
immediate cleanup, retry, and exhaustion; complete-model and mutable-owner
contradictions; unified E1 cleanup; and bounded shutdown/disconnection truth.
No process RSS observation or physical leak claim was made.

Every hosted native plan then passed from its own fresh target through the same
interface used by Quality:

```sh
cargo xtask verify-component structure
cargo xtask verify-component check
cargo xtask verify-component test
cargo xtask verify-component clippy
cargo xtask verify-component docs
cargo xtask verify-component benches
```

Fresh retained target observations were respectively 145,840 KiB, 2,027,432
KiB, 7,681,172 KiB, 2,027,432 KiB, 2,041,140 KiB, and 1,253,884 KiB. The
workspace test plan reported 525 passing libtest/doctest cases, no failures, and
the same one intentionally ignored fixture-maintenance operation. The exact two
maintained benchmark targets compiled without statistical execution.

The local developer composite passed from another fresh target:

```sh
cargo xtask verify
```

Its retained target was 10,405,360 KiB, separately demonstrating why the
multi-profile local command consumes more disk than any one hosted component.
These are fresh-target end observations with no in-command target cleanup, not
continuously sampled byte-exact peaks or product-performance measurements.

Both portable plans passed in separate fresh targets:

```sh
cargo xtask portable wasm32-unknown-unknown
cargo xtask portable thumbv7em-none-eabihf
```

Each retained 152,232 KiB and printed the exact owned package set:
`context-planner`, `domain-contracts`, `sampling`, `task-graph`, and
`tokenization`; only portable `libm` joined that local graph. No adapter, native
runtime, storage, UI, benchmark, or tooling package leaked into either target
check.

Finally, the repository-pinned tools passed:

```sh
cargo deny --workspace --locked check advisories bans licenses sources
lychee --config lychee.toml --offline '**/*.md'
```

Cargo-deny 0.20.2 reported advisories, bans, licenses, and sources all `ok`;
duplicate versions remained configured warnings. Lychee 0.24.2 checked 256
links: 235 OK, 21 excluded, and 0 errors. Across the six structural, fourteen
focused, six component, one composite, two portable, and two policy/link gates,
31 commands passed and none failed. Validation exposed no code, test, workflow,
or documentation repair. Remote Quality, current self-hosted RTX CUDA acceptance,
and a reviewed external mixed checkpoint remain pending.

## Canonical repository gate

The ordinary composite gate is:

```sh
cargo xtask verify
```

`tools/xtask` runs architecture and hygiene policy, formatting, every intended workspace target, ordinary tests/doctests, mandatory Clippy, API documentation with warnings denied, and exact maintained benchmark compilation. Package metadata is checked bidirectionally against Cargo bench targets before commands run. It does not execute statistical measurements, a hardware suite, or a network-dependent external model.

The composite iterates the same typed plans exposed to hosted CI. Useful direct
diagnostics are:

```sh
cargo xtask verify-component structure
cargo xtask verify-component check
cargo xtask verify-component test
cargo xtask verify-component clippy
cargo xtask verify-component docs
cargo xtask verify-component benches
git diff --check
```

`structure` runs architecture, hygiene, format, and locked metadata validation.
The other plans derive the exact sorted workspace package inventory from locked
Cargo metadata after validating every declared role. `benches` derives only
`runtime-benchmarks/runtime` and `sampling/sampling_pipeline` from the
bidirectionally checked benchmark registry. Every component prints the exact
Cargo command it executes and fails on an unknown role, target, or component.
The mandatory lint profile uses stable selected Clippy policy under `-D
warnings`. `verify-component nursery` is a separate exploratory plan.

## Shared CPU quality workflow

[`.github/workflows/quality.yml`](../../.github/workflows/quality.yml) is the normal shared-CI gate. It runs on every push and pull request, plus a weekly schedule, using GitHub-hosted Ubuntu 24.04 and the mandatory default CPU feature graph. It does not install a CUDA toolkit, require a driver, enable the `cuda` feature, execute CUDA hardware tests, download an external model, or run performance thresholds.

The hosted topology is deliberately wider than the local composite. A six-leg
native matrix gives structure, check, tests, Clippy, rustdoc, and exact benchmark
compilation a fresh standard runner and a unique target. A separate two-leg
matrix checks the metadata-owned portable domain packages on WebAssembly and
embedded targets without native UI/backend dependencies. Policy, scheduled
nursery, and scheduled external links use their own targets and tool roots. Only
the nursery lint-report step is non-blocking; checkout, setup, dependency
installation, shell execution, and cleanup failures remain real job failures.

Every Cargo-building job sets `CARGO_INCREMENTAL=0`, rejects an unexpected root
`target/`, prints commit/tree provenance where relevant, records `df`/`du`, and
removes its exact target/tool/shim resources under `if: always()`. The reviewed
`.github/scripts/ci-resource.sh` validates that cleanup paths are direct children
of `RUNNER_TEMP`, rejects checkout-local target artifacts, and verifies removal.
Policy compilation resets its target between cargo-deny and Lychee, so their
build trees never coexist.

Fresh local observations used to scope the standard-runner floors are
infrastructure evidence, not product performance or hosted success:

| Leg | Fresh local target/tool observation | Preflight floor |
|---|---:|---:|
| structure | 145,932 KiB | 1 GiB |
| check | 2,030,064 KiB | 4 GiB |
| tests | 7,686,032 KiB | 9 GiB |
| Clippy | 2,030,064 KiB | 4 GiB |
| rustdoc | 2,043,752 KiB | 4 GiB |
| exact benches | 1,254,908 KiB | 3 GiB |
| each portable target | 152,324 KiB | 1 GiB |
| policy/link tool compilation | 882,408 KiB sampled high-water | 2 GiB |
| nursery | 299,560 KiB before expected findings; bounded by the 2,028,492 KiB full Clippy graph | 4 GiB |

The standard Ubuntu 24.04 image is documented with 14 GB total SSD, so none of
these checks demands the whole runner or assumes a larger paid runner. Larger
runners are not selected by this topology. The first exact redesigned hosted run
must still be inspected because local target size and hosted filesystem
low-water are different observations. Diagnose an exhausted hosted filesystem as
CI infrastructure until a source-level product failure is independently shown.
See GitHub's [hosted-runner specification](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
and [larger-runner specification](https://docs.github.com/en/actions/reference/runners/larger-runners).

All checkout steps use immutable `actions/checkout` v7.0.1 commit `3d3c42e5aac5ba805825da76410c181273ba90b1` (Node 24) with credentials disabled and read-only repository permission. Self-hosted runners must meet checkout's documented minimum runner version 2.327.1.

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

Architecture validates locked typed Cargo metadata, explicit manifest roles, the generic layer DAG, actual domain acyclicity, exact exception/CUDA records, and maintained benchmark registration. Hygiene shares the role/benchmark source while validating tracked operational surfaces, manifests, selected dependencies, nested locks/targets, generated results, caches, and source-tree artifacts. See [dependency policy](dependency-policy.md).

## Download-free focused CPU validation

Ordinary tests and the canonical gate do not resolve or download external models. Run focused artifact-loading checks sequentially:

```sh
cargo test --locked -p domain-contracts
cargo test --locked -p candle-backend
cargo test --locked -p hf-hub-adapter
cargo test --locked -p inference-runtime --test native_backend_generation
cargo test --locked -p inference-runtime --test fault_injection
cargo test --locked -p inference-runtime
cargo test --locked -p application-runtime
cargo test --locked -p runtime-benchmarks
```

The portable contract tests prove that ordinary pre-attempt preparation and resource-bearing failed materialization are distinct associated typestates. The `FailedLoad` guard keeps the sole raw owner encapsulated, retries cleanup idempotently, permits ordinary drop only after explicit cleanup success, and deliberately retains an unresolved raw owner when direct error propagation abandons the guard.

The Candle package is the deterministic owner for required-versus-complete-observed scalar policy, all declaration states, every Safetensors 0.8 category, required unsupported-dtype rejection, ignored-extra non-materialization/transfer, exact required-only CPU/CUDA load footprints, whole-shard identity authorities, verified and mutable mutation handling, bounded structural metadata/allocation failure, malformed/duplicate/gapped/overlapping/truncated/reordered shards, and partial-load ownership at every stage. Homogeneous required layouts may accept an absent declaration; mixed required F16/F32 and BF16/F32 fixtures must reject absence and require their matching declaration. Sequence-plan tests separately prove that all-layer persistent KV ownership scales with depth while one block's transient forward peak does not, including the realistic 22-layer TinyLlama configuration. The host-budget test requires loading-peak rejection before materialization. The Hub package owns bounded immutable-commit discovery, strict declaration parsing, shard selection, exact LFS identity proof, and project-established fallback hashing.

The native E0 suite includes `mixed_f16_f32_fixture_covers_e0_generation_accounting_and_lifecycle`. It compares the independent preparation with the E0 receipt, exercises hosted prefill/decode and generation, restores model-only reserved ownership after release, unloads to exact empty model/request/workspace/cleanup accounting, and completes bounded shutdown and join.

The fault suite deterministically covers invalid preparations, loading-peak admission before materialization, immediate failed-load cleanup, retained ownership and full loading-peak accounting when cleanup fails, retry release exactly once, cleanup exhaustion through shutdown, and no model publication on contract failure. It also substitutes and mutates a failed owner's plan before and during cleanup, proving that E0 removes formerly exact accounting once, preserves monotonic conservative evidence, blocks admission under uncertainty, and releases only after explicit cleanup succeeds. The focused E1 checks keep a controlled mixed declaration distinct from actual F32 execution and prove that an unavailable selected CUDA target blocks load without CPU fallback. The benchmark package tests the report contracts without running a model download or statistical measurement.

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

Do not use workspace `--all-features`; the exact graph is `runtime-benchmarks/cuda -> application-runtime/cuda -> candle-backend/cuda`, `desktop-slint/cuda -> application-runtime/cuda`, plus the development-only `inference-runtime/cuda -> candle-backend/cuda` test edge. This exact compile chain passed locally on both the 2026-08-08 closure tree and the 2026-08-10 artifact-loading amendment with `CUDA_COMPUTE_CAP=120`.

Hardware execution is absent from ordinary CPU tests and requires both the package-local `cuda-hardware-tests` feature and `MILKDRIFT_CUDA_TEST=1`. Each owning package declares one explicit harness-free `cuda_hardware` target. The source macro requires one or more registered cases, the runner counts every attempted case, and absence of opt-in is a failure rather than a successful skip. Cargo fails when a target is missing; adding a case to a suite runs it without a workflow edit.

Run the complete adapter, E0, deterministic cleanup, and E1 boundaries sequentially:

```sh
export CUDA_VISIBLE_DEVICES=0
export CUDA_COMPUTE_CAP=120
export MILKDRIFT_CUDA_TEST=1

cargo test --release --locked \
    -p candle-backend \
    --features cuda-hardware-tests \
    --test cuda_hardware

cargo test --release --locked \
    -p inference-runtime \
    --features cuda-hardware-tests \
    --test cuda_hardware

cargo test --release --locked \
    -p inference-runtime \
    --features cuda \
    --test fault_injection \
    -- \
    --nocapture \
    --test-threads=1

cargo test --release --locked \
    -p application-runtime \
    --features cuda-hardware-tests \
    --test cuda_hardware
```

The adapter suite owns explicit CPU execution in a CUDA build, invalid-ordinal rejection, ordinal-0 F32 identity/logit comparison, homogeneous BF16, mixed F16/F32, and mixed BF16/F32. The E0 suite owns mixed execution, preparation/receipt identity, scheduled generation, sequence release, unload, and zero accounting. The complete fault target owns deterministic failed-load and cleanup behavior. The E1 suite owns explicit unavailable-CUDA no-fallback plus real discovery, selection, actual scalar/device, unload, and bounded shutdown.

Do not use workspace `--all-features`, parse test listings, or interpret compilation as device execution. The former ignored/name-enumerated matrix passed locally on 2026-08-08 and on the artifact-loading amendment on 2026-08-10. Phase 12 self-hosted run `31281013243` later passed that historical workflow on closure commit `181a069`; a dedicated-suite result must be recorded separately for the current tree.

## Self-hosted CUDA hardware correctness gate

[`.github/workflows/cuda-hardware.yml`](../../.github/workflows/cuda-hardware.yml) is a separate lockfile-constrained correctness gate for the committed fixture. The maintained repository runner is named `hart-desk-rtx5070ti` and is selected with all registered labels: `self-hosted`, `Linux`, `X64`, and the dedicated `milkdrift-cuda-5070ti` label. If that runner is offline, removed, or under maintenance, restore its exact registration rather than weakening the job to generic `self-hosted` routing.

The security boundary is deliberate:

- triggers are pushes to `main` filtered by `.github/workflows/**`, `.gitignore`, `.cargo/**`, root `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `crates/**`, `benchmarks/runtime/**`, and `tools/xtask/**`, plus owner-dispatched runs of the `main` ref only; documentation-only changes do not schedule push runs;
- neither `pull_request` nor `pull_request_target` can schedule the machine, so fork or other untrusted PR code is never checked out there;
- path-filtered pushes trust code already landed on `main`; anyone able to land matching code on `main` is inside the machine-execution boundary, so repository write and branch controls remain part of runner security;
- workflow permissions are `contents: read`, checkout credentials are not persisted, and no repository secret or command input is used;
- one repository-wide concurrency group prevents overlapping Milkdrift GPU jobs;
- after exact host/GPU/toolchain preflight, one dependency-only `cargo fetch --locked` step may synchronize the maintained Cargo cache; the job-wide default remains offline for metadata, policy, compilation, Clippy, and hardware execution;
- check and release-hardware targets are separately isolated beneath `$RUNNER_TEMP`, root-target creation is rejected, and an `always()` final step removes both without `cargo clean`.

The runner administrator maintains `/var/tmp/milkdrift-cargo-home` as a dependency-only Cargo cache without credentials. The trusted workflow synchronizes exact `Cargo.lock` packages after preflight; `cargo fetch` does not execute package build scripts, and the immediately following offline metadata step is the completeness check. The job fails before compilation when synchronization or that check fails, the cache is inaccessible, `RUNNER_TEMP` has less than 20 GiB free, or the Cargo-home filesystem has less than 4 GiB free. The 20 GiB threshold is a host-specific operational reserve for this self-hosted machine, not a standard-runner or product requirement: historical run 31281013243 observed its `/dev/mapper/root` filesystem at 1.9 TiB total with 139 GiB free before compilation. The redesigned job records both target sizes and filesystem availability so prompt-6 evidence can reassess that reserve. The runner must be Actions Runner 2.327.1 or newer for pinned checkout v7's Node 24 runtime.

The job validates the exact RTX 5070 Ti / CUDA ordinal 0 / compute capability 12.0 / Toolkit 12.8+ / build-cap-120 matrix. It compiles metadata/policy, the exact CUDA check/Clippy graph, and all dedicated suites in `${RUNNER_TEMP}/milkdrift-cuda-check-target`, reports its size, and removes it before release execution. It then runs the complete adapter, E0, fault-cleanup, and E1 suite boundaries in `${RUNNER_TEMP}/milkdrift-cuda-hardware-target`, reports target/Cargo-home/filesystem use, and always cleans both targets. No shell registry contains test function names.

The job does not run TinyLlama, network resolution, Criterion, elapsed-time thresholds, Slint interaction, or arbitrary models. Toolkit 12.8+ preflight does not broaden product support beyond an actually observed row. Historical Phase 12 run `31281013243` succeeded before this suite redesign; redesigned run `31696186329` passed on exact commit `6df699c`, and later trees require their own post-push run.

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
    > target/phase12-evidence/tinyllama-cpu-schema6.json

target/phase12-cuda/release/external-baseline \
    --allow-network \
    --cache-dir target/phase10-external-cache \
    --device cuda:0 \
    > target/phase12-evidence/tinyllama-cuda-schema6.json
```

The executable writes no result file itself: stdout is exactly one structured report, stderr carries progress and concise diagnostics, and the redirect owns the ignored raw artifact. Do not edit generated JSON.

The primary cycle on each device must prove the exact model/revision, non-empty compatible chat, one direct-completion warmup, three measured 32-token completions, matching request identities, exact terminal/released outcomes and usage, one cancellation after decoded progress, zero cleanup-pending/exhausted events, synchronized zero-cancellation unload, and bounded shutdown. CUDA additionally performs two reduced stability cycles containing load, direct generation/release, separate cancellation/release, unload, synchronization, shutdown, and owner drop. Together with the primary cycle this is three complete CUDA lifecycle cycles; warmup timing remains separate from measured samples.

Review both current external schema-6 reports programmatically without printing generated text or token identifiers. Require:

- the same clean code-under-test Git commit/tree and `dirty: false`;
- the same exact model, revision, fixed artifact layout, prompt hashes, sampling settings, and primary workload;
- configuration-declared `BF16` from public E1 resolution;
- actual E1/E0-verified execution scalar F32 on CPU and BF16 on CUDA;
- requested, selected E1, and actual loaded device all CPU in one report and CUDA ordinal 0 in the other;
- process RSS/HWM only under `process_memory`, and whole-device CUDA total/free/used only under `whole_device_cuda_memory`;
- CUDA build/device metadata absent from CPU-only evidence and exact RTX 5070 Ti identity, driver/toolkit, compute capability 12.0, and build target 120 in CUDA evidence;
- matching terminal/release identities, expected token/byte counts, cancellation after progress, unload, shutdown, and all three CUDA lifecycle cycles; and
- no independent `prepared_load`, observed-tensor, planned scalar/device, footprint, direct-E0-reservation, or tautological success payload.

Schema 6 observes the public E1 product boundary and deliberately performs no shadow adapter preparation before timed load. Exact final/loading-peak plans and zero E0 ownership accounting remain owned by synthetic E0 evidence and dedicated correctness suites. Process RSS and whole-device CUDA observations remain non-attributed resource samples.

Each CUDA cycle establishes a new whole-device pre-load baseline. Interpret post-unload and post-owner-drop retained deltas with absolute observations; desktop or other GPU activity can perturb either. Three cycles prove neither a leak nor non-leak result.

No schema-6 product report has been accepted on the infrastructure-truth tree, so no new measured values replace the historical Phase 10/11 evidence in [performance evidence](performance.md#external-product-evidence).

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

Run every explicitly opted-in CUDA hardware test listed in [download-free CUDA hardware validation](#download-free-cuda-hardware-validation). The homogeneous TinyLlama schema-6 CPU/CUDA baseline remains an optional current product regression and is not mixed evidence; run it only with explicit network authorization. Absence of a suitable reviewed external mixed profile remains a documented evidence gap rather than a reason to substitute TinyLlama. Confirm artifact hygiene:

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
