# hf-hub-adapter

Blocking Hugging Face Hub resolution isolated behind a dedicated cold-path host
worker. The adapter accepts only `tokenizer.json`, `config.json`, and
unquantized Llama Safetensors layouts understood by the current Candle backend.
Repository inspection resolves mutable references to an immutable commit before
any required artifact is downloaded. Numbered shard layouts must be complete and
consistent, and configuration scalar declarations are exposed for admission. The
public result is deliberately named `ResolvedSafetensorsLlamaArtifacts`; it is not
a generic model bundle and should not be overloaded with a GGUF acquisition path.

`ApiBuilder::from_env` preserves environment-derived cache and authentication
unless explicit overrides are supplied. The upstream synchronous builder does
not expose a global request timeout, so callers must not run this adapter on an
event-loop or inference thread. Access tokens are redacted from adapter and
application configuration `Debug` output. The desktop runner uses bounded
command/event queues and a bounded shutdown wait.
