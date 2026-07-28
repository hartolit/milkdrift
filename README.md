# llm-app

A layered Rust workspace for a local-first, composable language-model system with explicit inference ownership, context planning, workflows, persistence, and replaceable frontends.

## Current product state

The currently composed application path uses Candle on the CPU with Hugging Face artifacts and tokenization. It can resolve, validate, load, generate direct-completion text, cancel, drain, unload, and persist the selection for one resident model through the frontend-neutral `application-runtime` façade.

A GGUF/llama.cpp CPU adapter also implements the lower inference compatibility boundary. It is not selectable through `application-runtime` or the Slint UI yet.

The E0 inference runtime owns local token-level scheduling, sampling, backend execution, cancellation boundaries, backpressure, and cleanup. E1 `application-runtime` exposes that loop as frontend-neutral application behavior: it encodes UTF-8 prompts, translates stable generation settings, owns request-local streaming decode state, and returns bounded pulled text/state batches. The independently stateful corrective workflow is a separate capability engine. In particular:

- frontends use E1 generation APIs rather than E0 commands, logits, or sequence state;
- tokenizer and decoded-text ownership live in E1 while sampling and token stepping remain in E0;
- the Slint frontend exposes the complete direct-completion Phase 6 path with prompt/output/generate/cancel controls;
- general chat rendering and conversation history follow the direct-completion slice;
- GPU execution, hosted-provider/peer execution, remote frontend transport, and multiple application-level resident models are not supported yet.

See the [current implementation status](docs/project/implementation-status.md) for the exact integration matrix and validation evidence. The [execution plan](docs/agent/execution/execution-plan.md) is the active roadmap.

## Workspace

```text
crates/features/     portable contracts and algorithms
crates/adapters/     model, tokenizer, storage, network, and host integrations
crates/engines/      E0 inference, capability engines, and E1 application coordination
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

The frontend resolves and loads the Candle CPU model, submits direct completions through E1, streams bounded decoded output, cancels active work, and unloads deterministically. Application state is stored in the platform's per-user application-data directory.

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
