# GGUF backend

## Scope

`gguf-backend` is the CPU adapter for local GGUF files. It quarantines
`llama-cpp-2`, llama.cpp native resources, GGUF metadata, and context-cache
management behind the portable `domain-contracts` API.

The crate does not depend on another adapter or on an engine. Compile-time
compatibility checks live in `inference-runtime`, whose downward development
edge to the adapter is permitted by the layered architecture.

## Initialization and ownership

llama.cpp permits one initialized backend token per process. The application
must initialize `GgufBackendRuntime` explicitly and inject clones into each
`GgufLoader`. No hidden global is created by project code.

A loaded model owns, in drop order:

1. one reusable native batch;
2. one self-referential model/context cell;
3. the native model;
4. the final shared backend-initialization token.

The self-reference is implemented with `self_cell`. Its macro-generated
implementation is quarantined in one private module with a local
`allow(unsafe_code)`. The surrounding adapter crate uses `deny(unsafe_code)`,
project-authored code contains no unsafe block, and no native pointer crosses
the adapter API.

The inference registry remains the only owner of `GgufModel`. Logical
`GgufSequence` values contain identifiers, positions, capacity, and lifecycle
state only. They do not retain model weights.

## Multiple sequences

One llama.cpp context is allocated with a fixed total context capacity:

```text
context tokens per sequence × maximum resident sequences
```

Each Rust sequence receives one bounded native sequence identifier. Prefill and
decode submit tokens tagged with that identifier, so their KV-cache contents
remain distinct inside the shared context.

Sequence destruction and reset remove the complete native sequence cache. A failed
native release preserves the runtime-owned sequence and is retried at later safe
lifecycle polls. `prepare_unload` is rejected while any sequence slot remains
occupied.

## Metadata inspection

Admission planning happens before native model loading. A streaming Rust GGUF
reader accepts versions 2 and 3 and extracts the transformer metadata required
by the portable model descriptor:

- architecture;
- file type;
- context length;
- block and embedding dimensions;
- attention and KV-head counts;
- tokenizer vocabulary count.

Inspection is bounded by independent limits for total header bytes, entry count,
string length, and array length. Unknown values are skipped without loading
tensor data. Architecture fields are matched exactly; similarly suffixed keys
such as rope-scaling metadata cannot overwrite the primary context length.

The current memory estimator targets attention-based decoder models whose GGUF
metadata supplies these dimensions. Unsupported metadata is rejected rather
than guessed.

## Memory admission

The loader configures F16 key and value caches explicitly. Its admission plan is
a deterministic lower-bound reservation containing:

```text
blocks × KV width × key/value × F16 bytes × total context tokens
```

It also reserves one vocabulary-sized F32 logits row. The complete model file size
is counted as host weight memory even when memory mapping is enabled. llama.cpp may
allocate additional architecture- and platform-specific compute graphs or native
scratch storage that cannot be predicted from portable GGUF metadata alone. The
reported footprint must therefore be treated as an admission lower bound, not an
exact RSS guarantee.

Sequence plans report zero additional cache allocation because the complete
native cache arena is created with the model context. They still report the
required caller-owned logits capacity.

## Execution guarantees

The adapter supports:

- CPU loading;
- prompt prefill;
- incremental decode;
- multiple logical sequences;
- sequence reset;
- bounded caller-owned logits output;
- deterministic sequence-slot return;
- synchronous CPU completion.

It deliberately does not advertise `ALLOCATION_FREE_HOT_PATH`. The upstream
safe wrapper updates internal vectors during decode, and llama.cpp retains its
own native execution behavior. Project-owned output slices do not resize, but
that is not sufficient to claim a backend-wide allocation guarantee.

The first model load or context execution against a real GGUF file remains an
integration concern for the target machine because `llama-cpp-2` builds native
C/C++ code and requires its documented build toolchain.
