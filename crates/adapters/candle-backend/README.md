# candle-backend

CPU-default and feature-gated CUDA adapter for unquantized Hugging Face Llama models stored as Safetensors.

The crate owns all Candle/`cudarc`-specific types and implements only the portable contracts from `domain-contracts`. It supports device-independent inspection plus device-aware admission planning, loading, independent sequence caches, prompt prefill, incremental decode, synchronization, and unload preparation. Loaded models report their actual `ExecutionDevice` and accepted footprint for E0 verification. Compatibility tests verify final-position prefill logits, decode position progression, cancellation boundaries, explicit synchronized destruction to `SequenceState::Finished`, post-unload cleanup, and host F32 output for F32, F16, and BF16 model sources.

## Allocation contract

This adapter does **not** advertise `CapabilitySet::ALLOCATION_FREE_HOT_PATH`. Candle 0.11's upstream Llama implementation concatenates KV-cache tensors as generation advances and creates tensors for forward operations. CUDA logits additionally use Candle's safe transfer into a temporary upstream CPU tensor before copying into reusable caller-owned output. The adapter remains useful as a correctness and compatibility backend, while a future strict path must provide measured pre-allocated cache, execution, and transfer storage before claiming the capability.

## Sequence reset

The upstream cache cannot be cleared in place without constructing a replacement. The adapter therefore does not advertise `CapabilitySet::SEQUENCE_RESET`; destroy and recreate the sequence at a cold lifecycle boundary.

## Supported scope

- CPU device ID 0, compiled by default and still usable in CUDA-enabled binaries
- Linux x86_64 CUDA ordinals behind the non-default `cuda` feature; explicit CUDA requests never fall back to CPU
- unquantized Llama-family models
- Hugging Face `config.json`
- one or more Safetensors shards
- F32, F16, and BF16 source weight types; BF16 CPU execution is upcast to F32, while supported CUDA devices retain BF16 execution
- CPU weights/cache/rope charged to host memory; CUDA weights/cache/rope charged to device memory with host load/transient storage represented separately

Model downloading and tokenizer integration remain separate adapters. Metal, cuDNN, flash attention, NCCL, multi-GPU execution, GPU-side sampling, GGUF, and other quantized formats are unsupported. Any future format work remains reviewed Candle-native work under this adapter rather than another local execution engine.
