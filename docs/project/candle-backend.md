# Candle CPU Reference Backend

## Scope

`crates/adapters/candle-backend` is the Candle CPU reference implementation of the
`domain-contracts` backend boundary. It supports unquantized Hugging Face Llama
configuration files and one or more Safetensors weight shards on the CPU.

The adapter owns all Candle types. No Candle tensor, device, model, cache, or
error type crosses into a feature crate.

## Lifecycle

The loader performs three cold-path operations:

1. inspect configuration and weight-file metadata;
2. validate CPU device zero and the host-memory budget;
3. reserve the largest shard as transient loading headroom;
4. load Safetensors into Candle and construct the Llama model.

The loaded model exclusively owns weights. Each sequence owns an independent
Candle cache, position, fixed token capacity, and token staging allocation.
Sequences do not retain or clone the loaded model.

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
→ synchronize and prepare unload
```

## Phase 4 generation semantics

The adapter preserves the backend-independent F32 logits contract for every scalar
type it advertises. F32 and F16 sources execute in their native scalar type. Candle
0.11 CPU matmul does not support BF16 operands, so BF16 source weights are validated
as BF16 and upcast to F32 at load time. Admission accounts for the expanded resident
weights and F32 sequence cache. Returned vocabulary logits are normalized to F32
before copying into the caller-owned slice. Capacity and layout validation occur
before sampling.

The deterministic compatibility fixture assigns distinguishable token embeddings
and LM-head rows while keeping the transformer residual path stable. Tests therefore
verify all of the following as semantic facts rather than only successful calls:

- prefill consumes the complete prompt and reports the next absolute position;
- the returned prefill logits belong to the final prompt position;
- each decode call consumes exactly one selected token and increments position once;
- independent sequence caches do not alter each other's progression;
- cancelled decode leaves the sequence position unchanged;
- logits contain exactly one full vocabulary for F32, F16, and BF16 sources;
- explicit sequence destruction and model unload remain valid after generation.

`inference-runtime/tests/candle_generation.rs` additionally drives the actual
`CandleLlamaLoader` through the hosted E0 scheduler. It covers token-limit and EOS
completion, one-token output backpressure, cancellation between backend calls,
terminal/released publication, accounting release, unload, and worker shutdown.

The opt-in external-model procedure is documented in
[Phase 4 Candle Llama Smoke Procedure](../execution/phase4-candle-smoke.md).

## Allocation capability

The adapter intentionally does not advertise
`CapabilitySet::ALLOCATION_FREE_HOT_PATH`.

The upstream Candle 0.11 Llama implementation concatenates KV-cache tensors as
tokens are appended and constructs intermediate tensors during forward passes.
Claiming strict allocation-free execution would therefore be false even though
the adapter itself pre-reserves token staging and writes logits into
caller-owned slices.

A later engine may use this capability as an admission requirement. A future
strict backend must use pre-allocated KV-cache and execution arenas before it
sets the bit.

## Sequence reset capability

The adapter intentionally does not advertise `CapabilitySet::SEQUENCE_RESET`.
Candle's upstream Llama cache does not expose a way to clear its private KV and
mask state in place. Replacing the cache would allocate and would violate the
`LoadedModel::reset_sequence` contract. The adapter therefore returns
`SequenceError::Unsupported`; callers must destroy and recreate the sequence at
a cold lifecycle boundary.

## Failure containment

Candle failures are translated into allocation-free `BackendFailure` values
with stable categories and numeric codes. Candle's upstream Llama loader uses an
internal panic for a malformed layer, so model construction is isolated behind
`catch_unwind` at the cold adapter boundary and converted into an invalid-model
load failure.
