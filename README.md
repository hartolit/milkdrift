# llm-app

A layered Rust workspace for local, CPU-only language-model inference, context planning, sampling, orchestration, persistence, and a replaceable native frontend.

## Current product state

The currently composed application path uses Candle on the CPU with Hugging Face artifacts and tokenization. It can resolve, validate, load, generate direct-completion text, cancel, drain, unload, and persist the selection for one resident model through the frontend-neutral `application-runtime` façade.

A GGUF/llama.cpp CPU adapter also implements the lower inference compatibility boundary. It is not selectable through `application-runtime` or the Slint UI yet.

The E0 inference runtime owns token-level scheduling, sampling, backend execution, cancellation boundaries, backpressure, and cleanup. Phase 5 exposes that loop through E1: `application-runtime` encodes UTF-8 prompts, translates stable generation settings, owns request-local streaming decode state, and returns bounded pulled text/state batches. In particular:

- frontends use E1 generation APIs rather than E0 commands, logits, or sequence state;
- tokenizer and decoded-text ownership live in E1 while sampling and token stepping remain in E0;
- the Slint frontend still exposes lifecycle controls only; prompt/output/generate/cancel presentation is Phase 6;
- general chat rendering and conversation history follow the direct-completion slice;
- GPU execution, remote transport, and multiple application-level resident models are not supported.

See the [current implementation status](docs/project/implementation-status.md) for the exact integration matrix and validation evidence. The [execution plan](docs/agent/execution/execution-plan.md) is the active roadmap.

## Workspace

```text
crates/features/     portable contracts and algorithms
crates/adapters/     model, tokenizer, storage, network, and host integrations
crates/engines/      inference ownership and application use cases
crates/apps/         presentation and process entry points
```

The current dependency policy and its enforcement scope are documented in [the architecture](docs/architecture.md). Documentation authority and all component guides are indexed in [the documentation map](docs/README.md).

## Validate

Run the current repository baseline gate with:

```text
cargo run --locked --bin llm-app -- verify
```

The root binary runs the Phase 1 architecture, formatting, workspace-check, ordinary-test, Clippy, API-documentation, and benchmark-compilation gates. Ordinary tests do not select benchmark targets. CI also enforces dependency policy, local Markdown links, and the named portable targets. This runner will be replaced by the planned `xtask` only after the earlier execution-plan gates are complete.

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

The frontend currently manages model resolution and lifecycle through the Candle CPU composition; it does not generate text yet. Application state is stored in the platform's per-user application-data directory.

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
