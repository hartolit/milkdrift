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

The `domain-contracts` allocation target is a harness-free executable so libtest/process activity cannot overlap its isolated allocator regions. The sampling package’s ordinary matrix test executes every benchmark case once at every vocabulary size and every stop case once; statistical execution is not required for correctness coverage.

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

## External CPU product baseline

`runtime-benchmarks` owns the sole current external E1 orchestration path. The external binary fixes `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable revision `fe8a4ea1ffedaf415f4da2f062534de366a451e6`; repository and revision overrides are rejected. The pinned revision's [model-card metadata](https://huggingface.co/TinyLlama/TinyLlama-1.1B-Chat-v1.0/raw/fe8a4ea1ffedaf415f4da2f062534de366a451e6/README.md) declares `apache-2.0`. Record that upstream declaration and source without making a broader legal conclusion.

This procedure is the only ordinary exception to the download-free rule, and it requires explicit authorization to contact Hugging Face for that exact model/revision. Shared CI and ordinary tests compile the binary but never execute it, contact the network, require its cache, or load TinyLlama.

Before building or acquiring artifacts, confirm no compiler, benchmark, or model process is consuming substantial resources and require approximately 12 GiB available host memory plus 8 GiB free disk for the root target/cache:

```sh
export CARGO_TARGET_DIR="$(git rev-parse --show-toplevel)/target"
free -h
df -h "$(git rev-parse --show-toplevel)/target"
ps -eo pid,comm,rss --sort=-rss | head -20
```

Stop rather than running when either capacity bound is not met. Use one Cargo process and one model process at a time. Do not run `cargo clean`.

Create an explicit cache and evidence directory beneath the ignored repository-root target. The cache must already exist when the binary starts. It may instead be an existing canonical directory outside the repository, but any cache inside the repository and outside its root `target/` is rejected. The runner configures this exact cache and does not implicitly use a default global Hugging Face cache.

```sh
mkdir -p target/phase10-external-cache
mkdir -p target/phase10-evidence

cargo build --release --locked \
    -p runtime-benchmarks \
    --bin external-baseline

target/release/external-baseline \
    --allow-network \
    --cache-dir target/phase10-external-cache \
    > target/phase10-evidence/external.json
```

When `CARGO_TARGET_DIR` differs, execute `external-baseline` from that configured root target rather than creating a nested package target. Build first and execute the binary directly so no compiler overlaps model residency. The executable writes no result file itself: stdout is exactly one structured report, stderr carries progress/concise diagnostics, and the redirect above owns the ignored raw artifact.

Before treating a run as evidence, verify the report and surrounding Git commands agree on a clean commit/tree, exact model/commit, compatible-chat success, one warmup plus three sequential 32-token direct completions, clean release, zero-cancellation unload, and successful bounded shutdown. Record whether the explicit cache was empty or populated before resolution; do not call the interval pure download or pure cache lookup without that observation. Exact measured values belong only in [performance evidence](performance.md#external-product-evidence).

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
