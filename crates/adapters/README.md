# Adapter crates

Model, tokenizer, storage, network, device, FFI, and third-party integrations are
quarantined here. Process-host execution primitives live under `crates/platform`.

Current adapters:

- `candle-backend`: CPU Llama reference backend using Candle and Safetensors;
- `hf-tokenizer`: Hugging Face tokenizer implementation of portable tokenizer
  contracts;
- `hf-hub-adapter`: synchronous cached model-artifact resolution;
- `redb-storage`: versioned desktop settings and model-catalogue persistence.

Adapters may depend downward on domain contracts. Domain and platform crates never
depend on adapters, and production adapters do not import one another. Runtime
composition selects multiple adapters when required.

## `gguf-backend`

Local GGUF CPU inference through quarantined llama.cpp bindings. It owns native
model/context/cache state and exposes only `domain-contracts` types.
