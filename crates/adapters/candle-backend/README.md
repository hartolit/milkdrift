# candle-backend

CPU-default and feature-gated CUDA adapter for unquantized Hugging Face Llama models stored as Safetensors.

The crate owns all Candle/`cudarc`-specific types and implements only the portable contracts from `domain-contracts`. It supports device-independent inspection, device-aware admission planning, loading, independent sequence caches, prompt prefill, incremental decode, synchronization, and unload preparation.

## Scalar and accounting contract

`ModelDescriptor::metadata.scalar_type` reports the scalar stored in the source weights. `LoadPlan::execution_scalar_type` reports the scalar selected during device-aware planning, and `LoadedModel::execution_scalar_type()` reports the actual domain `ScalarType` retained by the loaded model. Candle's `DType` remains internal to this adapter.

The supported policy is:

- F32 source on CPU or CUDA executes as F32;
- F16 source on CPU or CUDA executes as F16;
- BF16 source on CPU executes as F32 because Candle 0.11 CPU matmul does not support BF16 operands;
- BF16 source on CUDA executes as BF16 only when the selected device reports support; otherwise planning fails before any model weights become resident.

Weight, cache, and rope accounting is computed from the selected execution scalar. Caller-visible vocabulary logits remain host F32 for every supported source and execution scalar.

`LoadPlan::expected_footprint` is the planned accounting quantity. After loading, `LoadedModel::accounted_footprint()` reports the accepted quantity for runtime verification. These values are not observations of physical memory use or availability. CUDA physical capacity and moment-in-time availability are reported separately by `CandleDeviceSummary::{total_memory_bytes, available_memory_bytes}`.

## CUDA initialization lifecycle

CPU device ID 0 maps directly to `Device::Cpu`. CUDA ordinals are available only behind the non-default `cuda` feature and never fall back to CPU.

Discovery, load planning, and loading are independent cold paths. Each CUDA invocation initializes its own Candle device and direct `cudarc` probe; no probe or context cache is retained between them. Planning drops its prepared device after returning the portable `LoadPlan`. Loading performs a fresh probe and retains only its loaded Candle device in the model.

Sequence creation, prefill, decode, synchronization, destruction, and unload preparation reuse the loaded model's device. They do not call `Device::new_cuda` or `CudaContext::new` per sequence or token. Unload preparation synchronizes the retained device; native resources are released when the owning model and sequence values are dropped.

## Allocation contract

This adapter does **not** advertise `CapabilitySet::ALLOCATION_FREE_HOT_PATH`. Candle 0.11's upstream Llama implementation concatenates KV-cache tensors as generation advances and creates tensors for forward operations. CUDA logits additionally use Candle's safe transfer into a temporary upstream CPU tensor before copying into reusable caller-owned output. A future strict path must provide measured pre-allocated cache, execution, and transfer storage before claiming the capability.

## Sequence reset

The upstream cache cannot be cleared in place without constructing a replacement. The adapter therefore does not advertise `CapabilitySet::SEQUENCE_RESET`; destroy and recreate the sequence at a cold lifecycle boundary.

## Tests

Default CPU tests deterministically cover F32, F16, and BF16 sources, including the BF16-source/F32-execution distinction, execution-scalar memory accounting, final-position prefill logits, decode progression, cancellation, independent caches, synchronized destruction, and unload preparation.

A CUDA-enabled build keeps explicit CPU execution non-ignored to prove that enabling CUDA does not change a CPU request. Actual CUDA execution remains both ignored and explicitly opted in with `MILKDRIFT_CUDA_TEST=1`. The target-specific tests validate CUDA ordinal 0, physical device observations, F32 CPU/CUDA compatibility, BF16-source/BF16-execution evidence, synchronization, and unload preparation.

## Supported scope

- CPU device ID 0, compiled by default and still usable in CUDA-enabled binaries
- Linux x86_64 CUDA ordinals behind the non-default `cuda` feature; explicit CUDA requests never fall back to CPU
- unquantized Llama-family models
- Hugging Face `config.json`
- one or more Safetensors shards
- F32, F16, and BF16 source weight types under the execution policy above
- CPU weights/cache/rope charged to host accounting; CUDA weights/cache/rope charged to device accounting with host load headroom represented separately

Model downloading and tokenizer integration remain separate adapters. Metal, cuDNN, flash attention, NCCL, multi-GPU execution, GPU-side sampling, GGUF, and other quantized formats are unsupported.
