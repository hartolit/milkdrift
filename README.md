# llm-app

A layered Rust workspace for a local-first, composable language-model system with explicit inference ownership, context planning, workflows, persistence, and replaceable frontends.

## Current product state

The frontend-neutral `application-runtime` (E1) exposes two closed local CPU products through `ModelSelection`: Hugging Face Hub + Candle + Safetensors, and local file + llama.cpp + GGUF. Both use the same E1 model lifecycle, direct-completion generation, cancellation, bounded output, and unload semantics for one resident model.

E0 owns token-level scheduling, sampling, backend execution, cancellation boundaries, backpressure, and cleanup. E1 owns the shared lifecycle, generation, and conversation behavior, including closed Hugging Face/GGUF tokenizer and streaming-decoder dispatch. Its private local capability starts two monomorphized E0 workers and routes commands, events, and output through the one active backend.

- Hugging Face resolution runs on the bounded Hub worker and retains an immutable commit identity.
- Local GGUF resolution is synchronous, canonicalizes the selected path, and verifies exact bytes with SHA-256.
- `TinyLlama/TinyLlama-1.1B-Chat-v1.0` remains the only verified chat profile; GGUF and other unverified models use honest direct completion.
- Slint maps only E1 types and presents backend, source, device, format, scalar type, quantization, and immutable identity.
- There is no `application-api`, hosted-provider or peer execution, GPU path, remote frontend transport, or multiple application-level resident models.

See the [current implementation status](docs/project/implementation-status.md) for the exact integration matrix and validation evidence. The [execution plan](docs/agent/execution/execution-plan.md) is the active roadmap.
The [project vision](docs/vision.md) records the longer-term research direction; it is intentionally aspirational and does not override current architecture, ADRs, or support status.

## Workspace

```text
crates/domain/       portable contracts and algorithms
crates/platform/     process-host threading, timing, channels, and bounded output plumbing
crates/adapters/     model, tokenizer, storage, network, and vendor integrations
crates/runtime/      E0 inference, capability engines, and E1 application coordination
crates/apps/         presentation and process entry points
```

The applied system structure is documented in [project architecture](docs/project/architecture.md), with enforcement details in [dependency policy](docs/project/dependency-policy.md). Documentation authority and all component guides are indexed in [the documentation map](docs/README.md).

## Validate

Run the current repository baseline gate with:

```text
cargo run --locked --bin llm-app -- verify
```

The root binary runs the architecture, formatting, workspace-check, ordinary-test, Clippy, API-documentation, and benchmark-compilation gates. Ordinary tests do not select benchmark targets. CI also enforces dependency policy, local Markdown links, and the named portable targets.

Plain Cargo commands also work normally:

```text
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
```

## Slint frontend

Run the native frontend with:

```text
cargo run -p desktop-slint
```

The frontend selects either closed CPU product, resolves and loads it through E1, streams bounded decoded output, cancels active work, and unloads deterministically. Verified TinyLlama uses Chat mode; GGUF uses Direct completion mode. Normal window closure invokes E1's bounded shutdown protocol for the Hub worker and both E0 workers. Application state is stored in the platform's per-user application-data directory.

Relevant guides:

- [Application runtime](docs/project/application-runtime.md)
- [Desktop runtime](docs/project/desktop-runtime.md)
- [Candle backend](docs/project/candle-backend.md)
- [GGUF backend](docs/project/gguf-backend.md)

## License

Project-authored source is available under either of:

- [Apache License 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

Third-party dependencies retain their own terms; the reviewed policy and Slint licensing note are documented in the [dependency policy](docs/project/dependency-policy.md).
