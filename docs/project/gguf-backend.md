# GGUF backend

## Scope

`gguf-backend` is the CPU adapter for local GGUF files and their native
tokenizers. It quarantines `llama-cpp-2`, llama.cpp native resources, GGUF
metadata, tokenization, and context-cache management behind portable project
contracts.

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

A `GgufTokenizer` separately owns a llama.cpp model loaded in vocabulary-only
mode plus a clone of the same explicit backend-initialization token. Tokenizer
clones share that immutable native vocabulary model and its precomputed token
evidence through `Arc`; they do not create inference contexts or own
`GgufModel`.

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

## Content identity

`Sha256Digest` is the canonical 32-byte GGUF content identity, with strict
64-character hexadecimal parsing and bounded streaming file hashing.
`GgufSource::new_verified` and `with_expected_digest` bind a local path to an
expected digest.

When a `GgufSource` carries an expected digest, `GgufLoader` hashes the file
before and after every metadata inspection and before and after native model
loading. A read error, initial mismatch, or change during either operation is a
load failure. The descriptor, admission plan, and loaded model therefore cannot
silently refer to different bytes at the same path. `GgufSource::new` remains an
explicit unverified form for callers that do not supply an identity.

`GgufTokenizer` always hashes before and after vocabulary loading. Its verified
constructors also compare both observations with the required digest, and every
successful tokenizer retains the digest of the exact bytes used for its native
vocabulary.

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

## Native tokenization

`GgufTokenizer` loads the GGUF through llama.cpp with
`LlamaModelParams::with_vocab_only(true)` and implements the portable
`tokenization::Tokenizer` and `StreamingTokenizer` contracts. Construction
validates the vocabulary size and optional BOS/EOS identifiers, classifies
control, user-defined, unknown, and end-of-generation tokens, and records safe
spellings for recognized special tokens.

Prompt encoding uses the lock-pinned safe `llama-cpp-2` API. That API exposes
`str_to_token(text, AddBos)` but not llama.cpp's `parse_special` switch and
always enables native special-token parsing. The adapter therefore checks the
input against its vocabulary evidence before calling it:

- `SpecialTokenPolicy::Allow` permits recognized special spellings;
- `OrdinaryText` fails closed when ordinary input contains one, rather than
  silently interpreting it as a special token;
- `Reject` rejects the same input with its distinct portable error code.

The native call always uses `AddBos::Never`. The portable beginning- and
end-of-sequence options are then applied independently by explicitly prepending
or appending the validated BOS or EOS token. Requesting a boundary token that
the vocabulary does not define is an error.

Decode first obtains each token's native byte piece. The stateful borrowed and
owned streaming decoders pass those pieces through `IncrementalUtf8Decoder`,
retain incomplete UTF-8 bytes across token boundaries, and reject an incomplete
final stream from `finish`. With `skip_special_tokens`, control/special and
end-of-generation tokens produce no bytes and report that they were skipped.
All output still uses the portable bounded sink contracts.

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

## Real GGUF execution evidence

The repository commits a deterministic, real GGUF v3 model at
`crates/runtime/inference-runtime/tests/fixtures/gguf-llama/tiny-llama-f32.gguf`.
It is 6,144 bytes, uses F32 tensors, and contains one tiny Llama block plus a
complete 16-token test vocabulary. The adjacent project-owned generator uses
only the Python standard library, verifies its committed Candle source fixture
and tensor schema, and reproduces the GGUF byte for byte.

`inference-runtime` defines one generic native-backend E0 suite and instantiates
it for both Candle and GGUF/llama.cpp. The GGUF instance performs a real native
load and drives `RuntimeCommand::Generate` through prompt prefill and
incremental decode. Across both instances the suite covers greedy generation,
seeded repeatability, EOS and token-limit completion, output backpressure,
user cancellation, sequence/workspace cleanup, model unload, empty post-unload
accounting, runtime shutdown, and worker join.

GGUF support is therefore no longer compile-only or real-load-unproven. The
target machine still needs the native llama.cpp C/C++ build toolchain. The
committed fixture and focused suite require no model download, but they prove
CPU execution and lifecycle behavior only: they are not language-quality
evidence, exercise no GPU path, and do not establish allocation-free execution.
See the [canonical fixture and native E0 procedure](validation.md#gguf-fixture-and-shared-native-e0-suite).
