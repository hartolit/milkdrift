# Adapter crates

Model, tokenizer, storage, network, device, FFI, and third-party integrations are
quarantined here. Process-host execution primitives live under `crates/platform`.

Current adapters:

- `candle-backend`: the local Candle/Safetensors Llama execution adapter, with
  mandatory/default CPU and feature-gated explicit CUDA execution;
- `hf-tokenizer`: Hugging Face tokenizer implementation of portable tokenizer
  contracts;
- `hf-hub-adapter`: synchronous cached model-artifact resolution;
- `redb-storage`: versioned desktop settings and model-catalogue persistence.

Adapters may depend downward on domain contracts. Domain and platform crates never
depend on adapters, and production adapters do not import one another. Runtime
composition selects multiple adapters when required.

The current local model scope is Candle, immutable Hugging Face Safetensors, the
unquantized Llama path, mandatory/default CPU, and explicit CUDA ordinal 0 only on
the executed Linux x86_64 RTX 5070 Ti matrix. The feature does not establish
generic NVIDIA compatibility, and CUDA never falls back to CPU. GGUF and other
quantized model loading are not supported; any future format support must be
implemented and reviewed separately under the Candle execution path. The sole
product support matrix is in [`implementation-status.md`](../../docs/project/implementation-status.md).
