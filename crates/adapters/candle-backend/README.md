# candle-backend

`candle-backend` is Milkdrift's CPU-default, feature-gated CUDA adapter for the
reviewed unquantized Hugging Face Llama/Safetensors path. It implements portable
`domain-contracts` model-loader/model/sequence contracts while keeping Candle,
Safetensors, filesystem, device, and `cudarc` types private.

## Source and identity API

`CandleLlamaSource` pairs bounded `config.json` with one or more
`CandleWeightShard` values. A shard is constructed with either:

- `with_expected_content(path, CandleExpectedContentIdentity)` for exact expected
  byte length and SHA-256, without claiming provider provenance or path
  immutability; or
- `unverified_local(path)`, which makes Candle establish a whole-file baseline
  from the retained open file before admission.

Materialization always verifies the complete retained bytes. Candle does not trust
filenames, cache conventions, symlinks, inodes, mtimes, ETags, or provider labels.

## Loader contract

Inspection reads bounded configuration plus complete Safetensors headers before
device initialization. It validates structure and the required Llama schema while
keeping four facts separate: optional producer declaration, complete observed
scalar categories, adapter-private required scalar policy, and selected execution
scalar.

`prepare_load` returns one source/configuration/device/budget-bound
`CandleLlamaPreparedLoad` and immutable `LoadPlan`. `load_prepared` consumes it,
re-verifies shards sequentially, streams ignored ranges through a fixed buffer,
materializes only required tensors, and uses bounded shard-aware transfer batches
for accelerator loading.

The final footprint counts required execution ownership. The separate loading peak
models the actual staging/verifier/transfer-plan lifetimes. `SequencePlan` likewise
separates persistent all-layer state from additional transient headroom. Full
formulas, limits, conversion policy, and source-derived Candle phase evidence live
in the [Candle project guide](../../../docs/project/candle-backend.md).

## Failure ownership

The unmaterialized preparation is ordinary-drop-safe. Once native acquisition
begins, every failure returns the distinct `CandleLlamaFailedPreparation` sole
owner. It retains open shards, completed tensors, the complete current transfer
batch, selected device, and any constructed model until explicit retryable cleanup
succeeds. E0 owns bounded retry, exact/unverified accounting, exhaustion, and
terminal process-retention policy.

## Scope

The adapter supports independent sequences, prefill, incremental decode,
synchronization, and explicit unload. It does not claim an allocation-free Candle
hot path or in-place sequence reset. Model download/tokenization, quantized/GGUF
loading, AMD/ROCm, Metal, cuDNN, flash attention, NCCL, multi-GPU, and GPU-side
sampling are outside this crate.

See the sole [support/evidence matrix](../../../docs/project/implementation-status.md)
for current product availability and the [operation guide](../../../docs/project/operation.md)
for its place in the end-to-end transaction.
