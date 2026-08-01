# Validation

This document owns repeatable current validation procedures. [Implementation status](implementation-status.md) records which commands have evidence on the current working tree; [execution history](../agent/execution/history.md) preserves older, baseline-specific results.

## Canonical repository gate

Run from the repository root on the exact tree being evaluated:

```sh
cargo xtask verify
```

The `xtask` package under `tools/xtask` validates architecture and repository hygiene, then checks formatting, every workspace target, ordinary tests/doctests, mandatory Clippy, API documentation with warnings denied, and benchmark compilation. It does not run the network-dependent external smoke. The root is a virtual workspace; there is no root runner package.

Use focused commands to diagnose failures without treating them as a substitute for the canonical gate:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo bench --workspace --no-run --locked
git diff --check
```

These are direct Cargo/Git operations, not commands forwarded by `xtask`.

The mandatory lint profile enables stable `clippy::all`, `clippy::pedantic`, and the workspace's explicit lints, then applies `-D warnings` to every workspace target. Clippy nursery is deliberately separate and exploratory: scheduled CI runs `cargo clippy --workspace --all-targets --locked -- -W clippy::nursery` without `-D warnings` and reports findings without making that job step blocking.

Record the evaluated commit and dirty state with local results. Required CI prints the checked-out commit ID and Git tree ID immediately before invoking `cargo xtask verify`; that runtime log is the run's provenance. A tracked document is not required to contain its own resulting tree hash, because adding that hash changes the tree. A command that passed on another commit or earlier working tree is not evidence for the current tree.

Required Linux CI also uses a fresh target directory and failing shims for CMake executables and the prohibited Python and Hugging Face command-line families. The non-FIPS `aws-lc` build is configured for its CC builder. A successful clean-target run therefore demonstrates that those external tools were not invoked; this procedure is not itself a claim about an unrun tree.

## Repository hygiene

Run the Rust-owned tooling and selected-graph hygiene policy independently with:

```sh
cargo xtask hygiene
```

Run architecture independently with:

```sh
cargo xtask architecture
```

The architecture command does not implicitly run hygiene; the canonical verify command invokes both. Hygiene examines tracked project artifacts, maintained operational surfaces, direct manifests, and locked Cargo metadata without filename, directory, document-status, or whole-file bypasses. It also rejects tracked nested `target` components, benchmark lockfiles/build scripts/generated output, model caches, unregistered `benchmarks/runtime`, and unknown benchmark manifests. Any future exception must be exact, reviewed, and tested. [Dependency policy](dependency-policy.md#rust-owned-operational-hygiene) defines the boundary.

## Download-free focused validation

Ordinary tests and the canonical gate do not resolve or download external models. The repository commits a project-generated deterministic Candle/Safetensors fixture for real-adapter E0 coverage and download-free synthetic runtime measurement; deterministic loaders cover backend-independent failure paths. The E0 integration test remains the fixture's primary owner. Existing `application-runtime` tests already load it for E1 coverage, while `runtime-benchmarks` is an additional in-place consumer and recomputes both exact SHA-256 values before runner or Criterion setup. The fixture's generator, tensor construction, licensing, exact sizes/hashes, replacement audit, and integration-only scope are recorded in its [provenance document](../../crates/runtime/inference-runtime/tests/fixtures/candle-llama/PROVENANCE.md).

Useful focused commands are:

```sh
cargo test --locked -p candle-backend
cargo test --locked -p inference-runtime --test native_backend_generation
cargo test --locked -p application-runtime
cargo test --locked -p desktop-slint
```

The Candle E0 target covers load, descriptor validation, prompt prefill, incremental decode, greedy and seeded sampling, EOS and token limits, output backpressure, cancellation, explicit sequence cleanup/release, unload, empty post-unload accounting, shutdown, and worker join. E1 tests additionally cover immutable resolution evidence, direct completion, exact TinyLlama chat compatibility, context/regeneration behavior, decoded output, unload policies, private incompatible-model cleanup retention through retry/exhaustion, successful transactional rollback after Hub startup failure, retained/reaped inference ownership after rollback timeout, persistence, worker disconnection, and retryable bounded shutdown with worker-handle retention. Slint tests cover E1-only selection/state mapping and presentation behavior.

The correctness tests prove integration and lifecycle behavior. The runtime harness also uses the fixture for controlled synthetic integration/lifecycle timing proxies, but neither use establishes model language quality, real-product performance, GPU execution, or strict allocation-free behavior inside upstream libraries.

Fixture regeneration is an explicit source-maintenance operation, not an ordinary test:

```text
cargo test --locked -p candle-backend --test generate_synthetic_fixture -- --ignored --exact regenerate_committed_candle_fixture
```

Run it only when intentionally replacing the committed fixture, then review both generated files and update the provenance hashes in the same change. The generator performs no network access and uses no external model or tokenizer assets.

## Runtime benchmark validation

A crate-local `benches/` directory is added only with a real component benchmark that states its question and runs production code. Cross-crate E0/E1 measurement is owned by the registered root-workspace package `benchmarks/runtime` (`runtime-benchmarks`), a non-production observer of reviewed public APIs. Run every command below from the repository root with the committed root `Cargo.lock`.

### Focused compile, test, and lint

Use these locked package checks while diagnosing the runtime harness:

```sh
cargo check --locked -p runtime-benchmarks --all-targets
cargo test --locked -p runtime-benchmarks
cargo clippy --locked -p runtime-benchmarks --all-targets -- -D warnings
```

The package tests cover CLI/network/cache policy, exact fixture hashing and parsing, metadata, CPU/RSS parsing, and download-free synthetic behavior; they do not execute real-product mode. Compile every workspace benchmark target without measuring it with:

```sh
cargo bench --workspace --no-run --locked
```

The canonical shared CI gate performs this compile-only benchmark step. Shared CI never executes the baseline runner or Criterion measurements, never downloads the product model, and never gates on elapsed time or statistical regression thresholds.

### Root-target hygiene and output

The documented benchmark runs use the shared repository-root `target`; do not create `benchmarks/runtime/target`, a nested lockfile, or a package-local result/cache tree. Before and after benchmark runs, use:

```sh
test ! -e benchmarks/runtime/Cargo.lock
test ! -d benchmarks/runtime/target
cargo xtask hygiene
```

The two `test` commands succeed silently when no nested lock/target exists. Raw JSON, Criterion data/HTML, profiles, and model caches remain under root `target` or outside the repository and are not committed. Curated reviewed conclusions, not generated result trees, belong in [performance evidence](performance.md).

A measurement run writes exactly one serde JSON document to stdout and progress plus the compact human summary to stderr. The runner writes no result file itself, so create the root-target destination before redirecting stdout. The baseline synthetic release command is:

```sh
mkdir -p target/runtime-benchmarks
cargo run --release --locked -p runtime-benchmarks --bin baseline -- \
  --mode synthetic --warmup 1 --cycles 3 \
  > target/runtime-benchmarks/synthetic.json
```

Temporary redb/cache state for download-free lifecycle checks is created beneath `target/runtime-benchmarks` and deleted by the runner. Criterion writes its generated output beneath root `target/criterion`.

### Criterion component measurements

Run both hosted-E0 Criterion targets with:

```sh
cargo bench --locked -p runtime-benchmarks --bench runtime
```

Run either exact selected target with:

```sh
cargo bench --locked -p runtime-benchmarks --bench runtime -- \
  e0_hosted_checked_prefill/4_tokens
cargo bench --locked -p runtime-benchmarks --bench runtime -- \
  e0_hosted_incremental_decode/1_token_after_2_token_prefill
```

These are statistical comparison tools with fixed bounded Criterion configuration, not pass/fail timing gates. Their exact measured and excluded boundaries are documented in [`benchmarks/runtime/README.md`](../../benchmarks/runtime/README.md).

### Exact real-product invocation contract

Real-product mode is opt-in and fixed to:

| Field | Required value |
|---|---|
| Repository | `neubla/tiny-random-LlamaForCausalLM` |
| Requested revision and required immutable commit | `1c81a3fba044af78df253edc66bdbab183184932` |
| Engine/source/device/format/scalar | Candle / Hugging Face Hub / CPU / Safetensors / F32 |

There are no repository, revision, or substitution flags. `--cache-dir PATH` is mandatory and must identify an existing directory whose canonical location is under shared root `target/` or outside the repository; source-tree cache locations are rejected. The public E1 resolver always performs immutable Hub metadata resolution, so `--allow-network` is mandatory and `HF_HUB_OFFLINE=1` is rejected as contradictory.

Ensure `HF_HUB_OFFLINE` is not set to `1`, then run:

```sh
mkdir -p target/runtime-benchmarks/hf-cache
cargo run --release --locked -p runtime-benchmarks --bin baseline -- \
  --mode real-product \
  --cache-dir target/runtime-benchmarks/hf-cache \
  --allow-network --warmup 1 --cycles 3 \
  > target/runtime-benchmarks/real-product-network.json
```

Real-product measurements are never ordinary CI. Model and tokenizer artifacts remain in the allowed explicit cache and are not redistributed through the repository. This command defines the validation procedure; it does not assert that the compiled mode was executed or that a product baseline or final validation exists on the current tree.

## Rust-native Candle Hub smoke

This existing external smoke is an explicit, opt-in correctness and lifecycle check, not a benchmark or timing baseline. It is network/cache dependent and implemented as an `application-runtime` Rust example. It reuses production Hub resolution, tokenizer loading, E1 lifecycle, E0 scheduling, and Candle execution; no separate downloader or model-conversion workflow is maintained. The `runtime-benchmarks` real-product runner is independent, uses public `ApplicationRuntime` methods directly, and never invokes, includes, or parses this example.

Run exactly:

```sh
LLM_APP_CANDLE_HUB_SMOKE=1 cargo run --locked -p application-runtime --example candle_hub_smoke
```

### Exact pinned model

| Field | Required value |
|---|---|
| Repository | `neubla/tiny-random-LlamaForCausalLM` |
| Requested revision and expected immutable commit | `1c81a3fba044af78df253edc66bdbab183184932` |
| Expected engine/source/device/format | Candle / Hugging Face Hub / CPU / Safetensors |
| Expected scalar | F32 |
| Required artifacts | `config.json`, `tokenizer.json`, and `model.safetensors` (or a supported Safetensors index layout) |

Use the full pinned revision. The model is intentionally tiny and random; successful output validates resolution, identity, execution, lifecycle, and cleanup rather than language quality.

### Network, cache, and authentication

- Resolution may perform HTTPS/DNS requests to `huggingface.co` and may populate the local Hugging Face cache. Transient network or service failures can make the smoke fail without invalidating download-free tests.
- `HF_HOME` selects the cache root used by the upstream Rust Hub client. When unset, the client uses its environment-derived default. The directory must be writable and have enough space. A fully cached exact revision may avoid artifact downloads, but repository resolution still follows the adapter's configured Hub behavior.
- `HF_TOKEN` supplies authentication when required; the public fixture normally permits anonymous access. Token values are redacted from diagnostics. Use an authorized token for access-controlled repositories without printing or committing it.
- If `HF_HUB_OFFLINE` is set, the exact revision and required artifacts must already be cached. Unset it when network resolution is required.

The example creates a temporary redb workspace, resolves the exact repository/revision on the production Hub worker, verifies the immutable commit and derived Candle/Hub/CPU/Safetensors/F32 facts, loads one model, runs an eight-token bounded direct completion, observes terminal and released state, unloads, explicitly shuts down both workers, and removes the temporary workspace.

Compilation of the example performs no network access. The opt-in environment value must equal `1`; otherwise the executable exits with configuration guidance.

Record the complete output, revision, and dirty state when this smoke is used as release evidence. A cached artifact, successful resolution, or successful load alone is not the complete smoke.

## Dependency, link, and graph audits

Run the locked policy and local Markdown checks with:

```sh
cargo deny --workspace --locked check advisories bans licenses sources
lychee --config lychee.toml --offline '**/*.md'
```

Useful graph/audit reports are:

```sh
cargo metadata --locked --format-version 1
cargo tree --workspace --locked
cargo tree -d --locked
cargo tree -e features --locked
```

Duplicate versions are an audit input, not an automatic failure. Interpret the selected graph against [dependency policy](dependency-policy.md).

Cross-target checks are documented in [portability](portability.md). Performance measurements have separate methodology in [performance evidence](performance.md). Current Ubuntu system prerequisites are documented in [dependency policy](dependency-policy.md#linux-ci-prerequisites).
