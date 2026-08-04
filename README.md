# Milkdrift

A layered Rust workspace for a local-first, composable language-model system with explicit inference ownership, context planning, workflows, persistence, and replaceable frontends.

## Current product state

Milkdrift’s current local product uses Candle with immutable Hugging Face Hub revisions, Safetensors, and the unquantized Llama path. CPU is mandatory, compiled by default, and selected on a fresh installation. Explicit non-default CUDA ordinal 0 is supported only on the executed Linux x86_64 NVIDIA GeForce RTX 5070 Ti matrix recorded in [implementation status](docs/project/implementation-status.md); this is not a generic NVIDIA compatibility claim, and an unavailable CUDA selection never falls back to CPU.

`application-runtime` (E1) is the frontend-neutral façade. `ModelSelection` contains a Hugging Face repository and requested revision. Resolution pins that selection to an immutable Hub commit and reports source evidence without choosing a device. After load, E1 reports the verified source scalar, execution scalar, and actual execution device separately. In particular:

```text
BF16 source on CPU
    -> F32 execution

BF16 source on supported CUDA
    -> BF16 execution
```

E1 owns one concrete Candle E0 worker/thread, one bounded Hub resolver worker, one resident-model lifecycle, and the Hugging Face tokenizer/streaming-decoder path. E0 owns loaded resources, request admission, scheduling, host-side sampling over F32 logits, cancellation, bounded output, cleanup accounting, synchronization, unload, and shutdown.

- Direct completion is available for every successfully loaded compatible model.
- Built-in chat is limited to `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6` when `</s>` resolves to token ID 2.
- Slint uses only E1-owned types and keeps selected device, source scalar, execution scalar, and actual loaded device distinct.
- CUDA-enabled builds can still explicitly select CPU.

GGUF and other quantized formats, Metal, GPU-side sampling, generic GPU selection, automatic CPU fallback, another local engine, hosted or peer execution, browser transport, `application-api`, and multiple application-level resident models are not supported.

See [implementation status](docs/project/implementation-status.md) for the sole product support matrix and accepted validation state. The [execution plan](docs/agent/execution/execution-plan.md) records the completed program and inactive future tracks; no product phase is active. The [project vision](docs/vision.md) remains aspirational rather than a support claim.

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

Ordinary workspace tests and the normal quality workflow are download-free and use the mandatory default CPU feature graph. The opt-in controlled CPU/CUDA external runner and its exact immutable model revision are documented in [project validation](docs/project/validation.md#controlled-cpu-and-cuda-external-product-evidence).

## Slint frontend

Run the default CPU build with:

```text
cargo run --locked -p desktop-slint
```

On the exact supported CUDA matrix, build and run the opt-in CUDA graph with:

```text
CUDA_COMPUTE_CAP=120 \
cargo run --release --locked \
    -p desktop-slint \
    --features cuda
```

The CUDA-enabled application still requires explicit device selection and can explicitly run on CPU. Feature compilation alone is not hardware-execution evidence. The frontend resolves immutable Candle/Safetensors artifacts through E1, streams bounded decoded output, supports cancellation and deterministic unload, and calls E1’s bounded shutdown protocol on normal closure. Verified TinyLlama uses Chat mode; every other compatible loaded model uses Direct completion mode. Application state is stored in the platform’s per-user application-data directory.

Relevant guides:

- [Application runtime](docs/project/application-runtime.md)
- [Desktop runtime](docs/project/desktop-runtime.md)
- [Candle backend](docs/project/candle-backend.md)
- [Validation](docs/project/validation.md)

## License

Milkdrift project-authored source code and documentation are licensed under the [Apache License 2.0](LICENSE); see [NOTICE](NOTICE) for attribution. The license permits commercial use, including paid inference, modification, redistribution, proprietary integrations, and products or services built using Milkdrift, subject to its terms.

The Milkdrift name, logo, and related brand assets are governed separately by the [trademark policy](TRADEMARKS.md) and are not licensed under Apache-2.0.

Third-party dependencies retain their own license terms. Slint licensing, attribution, and distribution obligations remain documented in the [dependency policy](docs/project/dependency-policy.md).
