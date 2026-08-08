# hf-hub-adapter

Blocking Hugging Face Hub resolution is isolated behind a dedicated cold-path host.
The adapter accepts only `tokenizer.json`, `config.json`, and
unquantized Llama Safetensors layouts understood by the current Candle backend.
Repository inspection resolves mutable references to an immutable commit before
any required artifact is downloaded. Numbered shard layouts must be complete and
consistent. Recognized `dtype` or legacy `torch_dtype` values are retained as
optional configuration-declared scalar metadata; they are not evidence that the
selected Safetensors tensors are homogeneous. The public result is deliberately
named `ResolvedSafetensorsLlamaArtifacts`; it is not a generic model bundle. Future
model-format or artifact-source work requires its own reviewed contract rather than
overloading this current result.

`ApiBuilder::from_env` preserves environment-derived cache and authentication
unless explicit overrides are supplied. The upstream synchronous builder does
not expose a global request timeout, so callers must not run this adapter on an
event-loop or inference thread. Access tokens are redacted from adapter and
application configuration `Debug` output. E1 runs this synchronous adapter on one
bounded Hub worker, separate from its sole Candle inference worker, and applies a
bounded shutdown wait.
