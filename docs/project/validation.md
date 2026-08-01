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

Ordinary tests and the canonical gate do not resolve or download external models. The repository commits a project-generated deterministic Candle/Safetensors fixture for real-adapter E0 coverage; deterministic loaders cover backend-independent failure paths. The fixture's generator, tensor construction, licensing, exact sizes/hashes, replacement audit, and integration-only scope are recorded in its [provenance document](../../crates/runtime/inference-runtime/tests/fixtures/candle-llama/PROVENANCE.md).

Useful focused commands are:

```sh
cargo test --locked -p candle-backend
cargo test --locked -p inference-runtime --test native_backend_generation
cargo test --locked -p application-runtime
cargo test --locked -p desktop-slint
```

The Candle E0 target covers load, descriptor validation, prompt prefill, incremental decode, greedy and seeded sampling, EOS and token limits, output backpressure, cancellation, explicit sequence cleanup/release, unload, empty post-unload accounting, shutdown, and worker join. E1 tests additionally cover immutable resolution evidence, direct completion, exact TinyLlama chat compatibility, context/regeneration behavior, decoded output, unload policies, private incompatible-model cleanup retention through retry/exhaustion, successful transactional rollback after Hub startup failure, retained/reaped inference ownership after rollback timeout, persistence, worker disconnection, and retryable bounded shutdown with worker-handle retention. Slint tests cover E1-only selection/state mapping and presentation behavior.

These fixtures prove integration and lifecycle behavior, not model language quality, real-model performance, GPU execution, or strict allocation-free behavior inside upstream libraries.

Fixture regeneration is an explicit source-maintenance operation, not an ordinary test:

```text
cargo test --locked -p candle-backend --test generate_synthetic_fixture -- --ignored --exact regenerate_committed_candle_fixture
```

Run it only when intentionally replacing the committed fixture, then review both generated files and update the provenance hashes in the same change. The generator performs no network access and uses no external model or tokenizer assets.

## Benchmark authoring and validation

A crate-local `benches/` directory is added only with a real component benchmark that states its question and runs production code. Cross-crate/system measurement is reserved for `benchmarks/runtime`, which does not yet exist.

When Phase 10 creates that package, use this order:

1. add the exact `benchmarks/runtime` member to root `Cargo.toml`;
2. create `benchmarks/runtime/Cargo.toml` with package name `runtime-benchmarks`, `publish = false`, no nested workspace, and no build target;
3. add only exact reviewed dependencies required by the implemented harness;
4. only then run Cargo from the repository root so the root lockfile and shared target remain authoritative.

Do not invoke Cargo directly against an unregistered benchmark manifest. Shared CI compiles benchmark targets through the canonical workspace gate but does not execute statistical measurements or enforce wall-clock thresholds. Raw Criterion/report/profile/cache output remains under the root `target`; curated summaries belong in [performance evidence](performance.md).

Real-model performance runs are opt-in and require an explicit external identifier, immutable revision, existing local cache or explicit artifact path, and no repository redistribution. No Phase 10 performance result exists yet.

## Rust-native Candle Hub smoke

The external smoke is explicit, opt-in, network/cache dependent, and implemented as an `application-runtime` Rust example. It reuses production Hub resolution, tokenizer loading, E1 lifecycle, E0 scheduling, and Candle execution; no separate downloader or model-conversion workflow is maintained.

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
