# Adapter crates

Model, tokenizer, storage, network, device, FFI, and third-party integrations are
quarantined here. Process-host execution primitives live under `crates/platform`.

Current adapters:

- `candle-backend`: the local execution adapter, currently supporting CPU Llama
  inference with Candle and Safetensors;
- `hf-tokenizer`: Hugging Face tokenizer implementation of portable tokenizer
  contracts;
- `hf-hub-adapter`: synchronous cached model-artifact resolution;
- `redb-storage`: versioned desktop settings and model-catalogue persistence.

Adapters may depend downward on domain contracts. Domain and platform crates never
depend on adapters, and production adapters do not import one another. Runtime
composition selects multiple adapters when required.

The current local model scope is Candle, Safetensors, and CPU execution. GGUF and
quantized model loading are not currently supported; any future format support
must be implemented and reviewed separately under the Candle execution path.
