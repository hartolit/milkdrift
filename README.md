# Milkdrift

A layered Rust workspace for a local-first, composable language-model system with explicit inference ownership, context planning, workflows, persistence, and replaceable frontends.

## Current product state

The current local product uses one coherent composition:

- **execution engine:** Candle;
- **artifact source:** immutable Hugging Face Hub revisions;
- **model format:** Safetensors;
- **device:** CPU.

`application-runtime` (E1) is the frontend-neutral façade. `ModelSelection` contains a Hugging Face repository and requested revision. Resolution pins that selection to an immutable Hub commit, loads the matching Hugging Face tokenizer, and reports application-owned engine, source, device, format, scalar, vocabulary, and commit facts. Callers cannot assemble unsupported combinations.

E1 owns one concrete Candle E0 worker/thread, one bounded Hub resolver worker, one resident-model lifecycle, and the concrete Hugging Face tokenizer/streaming-decoder path. E0 owns loaded model resources, sequences, request admission, token scheduling, sampling, cancellation boundaries, output backpressure, cleanup quarantine, accounting, unload, and shutdown.

- Direct completion is available for every successfully loaded model.
- Chat is enabled only for `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6` when `</s>` resolves to token ID 2.
- Conversation/context planning, regeneration and supersession, bounded decoded output, cancellation, cleanup retry/exhaustion, unload policy, redb-backed preferences/catalogue state, and explicit bounded shutdown remain implemented.
- Startup rolls back an already-started inference worker if Hub-worker startup fails. An incompatible load receipt is never published as the resident model: its handle remains privately accounted through unload retry or exhaustion. Shutdown is retryable, and a timeout retains worker handles rather than detaching them.
- Slint maps only E1-owned types and displays the derived engine, source, CPU device, Safetensors format, scalar type, and immutable Hub identity.

GGUF is not supported by the current product. Possible Candle-native GGUF or other quantized-format work requires a separate reviewed implementation. GPU execution is also deferred. There is no current llama.cpp product, hosted-provider or peer execution path, browser transport, `application-api`, or multiple application-level resident models.

See the [current implementation status](docs/project/implementation-status.md) for the exact support and validation state. The [execution plan](docs/agent/execution/execution-plan.md) is the active roadmap, and the [project vision](docs/vision.md) is intentionally aspirational rather than a support claim.

## Workspace

The root `Cargo.toml` is a virtual workspace manifest; there is no root Rust package. Repository-defined maintenance tooling is the `xtask` member under `tools/xtask`, exposed through the alias in `.cargo/config.toml`.

```text
.cargo/              workspace-local Cargo aliases
tools/xtask/         architecture, hygiene, and composite verification tooling
benchmarks/runtime/  non-production E0/E1 measurement observer
crates/domain/       portable contracts and algorithms
crates/platform/     process-host threading, timing, channels, and bounded output plumbing
crates/adapters/     Candle, tokenizer, Hub, storage, and vendor integrations
crates/runtime/      E0 inference, capability engines, and E1 application coordination
crates/apps/         presentation and process entry points
```

The applied structure is documented in [project architecture](docs/project/architecture.md), with exact members and crate edges in [workspace boundaries](docs/project/workspace.md) and enforcement in [dependency policy](docs/project/dependency-policy.md). Documentation authority and component guides are indexed in the [documentation map](docs/README.md).

## Validate

Run the canonical locked repository gate with:

```text
cargo xtask verify
```

Run the two custom policy checks independently with:

```text
cargo xtask architecture
cargo xtask hygiene
```

`xtask` owns only repository-specific policy and the composite gate. Ordinary Cargo operations are direct rather than forwarded through custom subcommands:

```text
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo bench --locked -p sampling --bench sampling_pipeline
```

Ordinary workspace tests are download-free. The opt-in, network-dependent E1 Candle/Hub smoke and its exact immutable revision are documented in [project validation](docs/project/validation.md#rust-native-candle-hub-smoke).

## Slint frontend

Run the native frontend with:

```text
cargo run --locked -p desktop-slint
```

The frontend accepts a Hugging Face repository and revision, resolves and loads the immutable Candle/Safetensors CPU model through E1, streams bounded decoded output, supports cancellation and deterministic unload, and calls E1's bounded shutdown protocol for the inference and Hub workers on normal closure. Verified TinyLlama uses Chat mode; every other loaded model uses honest Direct completion mode. Application state is stored in the platform's per-user application-data directory.

Relevant guides:

- [Application runtime](docs/project/application-runtime.md)
- [Desktop runtime](docs/project/desktop-runtime.md)
- [Candle backend](docs/project/candle-backend.md)
- [Validation](docs/project/validation.md)

## License

Milkdrift project-authored source code and documentation are licensed under the [Apache License 2.0](LICENSE); see [NOTICE](NOTICE) for attribution. The license permits commercial use, including paid inference, modification, redistribution, proprietary integrations, and products or services built using Milkdrift, subject to its terms.

The Milkdrift name, logo, and related brand assets are governed separately by the [trademark policy](TRADEMARKS.md) and are not licensed under Apache-2.0.

Third-party dependencies retain their own license terms. Slint licensing, attribution, and distribution obligations remain documented in the [dependency policy](docs/project/dependency-policy.md).
