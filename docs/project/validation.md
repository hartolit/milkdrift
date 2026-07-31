# Validation

This document owns repeatable current validation procedures. [Implementation status](implementation-status.md) records which commands have evidence on the current working tree; [execution history](../agent/execution/history.md) preserves older, baseline-specific results.

## Canonical repository gate

Run from the repository root on the exact tree being evaluated:

```sh
cargo run --locked --bin llm-app -- verify
```

The runner validates architecture and repository hygiene, then checks formatting, every workspace target, ordinary tests/doctests, strict Clippy, API documentation with warnings denied, and benchmark compilation. It does not run the network-dependent external smoke.

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

Record the exact source revision and whether the tree is dirty with the result. A command that passed on another commit or earlier working tree is not evidence for the current tree.

## Repository hygiene

Run the Rust-owned tooling and selected-graph hygiene policy independently with:

```sh
cargo run --locked --bin llm-app -- hygiene
```

The architecture command also includes hygiene:

```sh
cargo run --locked --bin llm-app -- architecture
```

The policy examines tracked project artifacts, maintained operational surfaces, direct manifests, and locked Cargo metadata. [Dependency policy](dependency-policy.md#rust-owned-operational-hygiene) defines the boundary and historical-text exclusions.

## Download-free focused validation

Ordinary tests and the canonical gate do not resolve or download external models. The repository commits a tiny deterministic Candle/Safetensors fixture for real-adapter E0 coverage; deterministic loaders cover backend-independent failure paths.

Useful focused commands are:

```sh
cargo test --locked -p candle-backend
cargo test --locked -p inference-runtime --test native_backend_generation
cargo test --locked -p application-runtime
cargo test --locked -p desktop-slint
```

The Candle E0 target covers load, descriptor validation, prompt prefill, incremental decode, greedy and seeded sampling, EOS and token limits, output backpressure, cancellation, explicit sequence cleanup/release, unload, empty post-unload accounting, shutdown, and worker join. E1 tests additionally cover immutable resolution evidence, direct completion, exact TinyLlama chat compatibility, context/regeneration behavior, decoded output, unload policies, persistence, worker disconnection, and bounded shutdown. Slint tests cover E1-only selection/state mapping and presentation behavior.

These fixtures prove integration and lifecycle behavior, not model language quality, GPU execution, or strict allocation-free behavior inside upstream libraries.

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
