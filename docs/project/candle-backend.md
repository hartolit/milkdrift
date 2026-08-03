# Candle CPU reference backend

## Scope

`crates/adapters/candle-backend` is the sole current local execution adapter. It implements the `domain-contracts` backend boundary for unquantized Hugging Face Llama configuration files and one or more Safetensors weight shards on CPU.

The adapter owns every Candle type. No Candle tensor, device, model, cache, or error crosses into a domain crate or E1's public application API. Artifact download and tokenizer integration remain separate adapters composed by E1.

## Lifecycle

The loader performs four cold-path operations:

1. inspect configuration and weight-file metadata;
2. validate CPU device zero and the host-memory budget;
3. reserve the largest shard as transient loading headroom;
4. load Safetensors into Candle and construct the Llama model.

The loaded model exclusively owns weights. Each sequence owns an independent Candle cache, position, fixed token capacity, and token staging allocation. Sequences do not retain or clone the loaded model. Successful explicit destruction marks the sequence `SequenceState::Finished` before drop, preventing post-destruction reuse.

The compatibility test executes:

```text
inspect
→ plan and load
→ create two independent sequences
→ prefill both
→ interleave decode
→ cancel one at a checked boundary
→ reject unsupported in-place sequence reset
→ expire a bounded drain window
→ explicitly destroy both sequences and assert Finished
→ synchronize and prepare unload
```

## Generation semantics

The adapter preserves the backend-independent F32 logits contract for every scalar type it advertises. F32 and F16 sources execute in their native scalar type. Candle 0.11 CPU matmul does not support BF16 operands, so BF16 source weights are validated as BF16 and upcast to F32 at load time. Admission accounts for expanded resident weights and F32 sequence cache. Vocabulary logits are normalized to F32 before copying into the caller-owned slice.

The deterministic fixture assigns distinguishable token embeddings and LM-head rows while keeping the transformer residual path stable. Tests verify that:

- prefill consumes the complete prompt and returns final-position logits;
- each decode consumes one selected token and increments position exactly once;
- independent sequence caches do not alter each other;
- cancelled decode leaves position unchanged;
- logits contain exactly one full vocabulary for F32, F16, and BF16 sources;
- successful destruction transitions each sequence to `Finished`;
- explicit destruction and model unload remain valid after generation.

`inference-runtime/tests/native_backend_generation.rs` drives `CandleLlamaLoader` through the hosted E0 scheduler. It covers token-limit and EOS completion, seeded repeatability, one-token output backpressure, cancellation between backend calls, terminal/released publication, accounting release, unload, an empty post-unload snapshot, shutdown, and worker join.

The opt-in external-model procedure is the authoritative E1 [external CPU product baseline](validation.md#external-cpu-product-baseline). It resolves the exact immutable artifact through `hf-hub-adapter`; ordinary adapter/runtime tests remain download-free.

## Allocation capability

The adapter intentionally does not advertise `CapabilitySet::ALLOCATION_FREE_HOT_PATH`.

The upstream Candle 0.11 Llama implementation concatenates KV-cache tensors as tokens are appended and constructs intermediate tensors during forward passes. Claiming strict allocation-free execution would be false even though the adapter pre-reserves token staging and writes logits into caller-owned slices.

A future strict implementation must use preallocated KV-cache and execution arenas before setting that capability.

## Sequence reset capability

The adapter intentionally does not advertise `CapabilitySet::SEQUENCE_RESET`. Candle's upstream Llama cache does not expose a way to clear private KV and mask state in place. Replacing the cache would allocate and violate the reset contract. Callers destroy and recreate the sequence at a cold lifecycle boundary.

## Failure containment

Candle failures are translated into allocation-free `BackendFailure` values with stable categories and numeric codes. Candle's upstream Llama loader uses an internal panic for a malformed layer, so model construction is isolated behind `catch_unwind` at the cold adapter boundary and converted into an invalid-model load failure.

## Deferred format and device work

The current adapter supports CPU, Hugging Face Llama configuration, unquantized Safetensors shards, and F32/F16/BF16 source weights under the behavior above.

GGUF and other quantized formats are not currently supported. If added, they remain Candle-native format work under this execution adapter and require reviewed model-family compatibility, tokenizer provenance, artifact identity, quantization, lifecycle, and test evidence. CUDA and Metal are likewise deferred device work; neither format nor device support is a reason by itself to introduce another local execution engine.
