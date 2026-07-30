# Desktop runtime

## Scope

The desktop composition owns native process concerns while delegating model
lifecycle, generation, and compatible conversation orchestration to `application-runtime`:

1. map the product controls to E1's closed `ModelSelection`;
2. resolve immutable Hugging Face/Candle/Safetensors artifacts or a canonical, SHA-256-verified local llama.cpp/GGUF file;
3. use the product-matched tokenizer and streaming decoder selected inside E1;
4. load one CPU model on the selected E0 worker;
5. expose E1 submit, regenerate, clear, cancellation, bounded text-pull, and unload behavior;
6. use verified chat only for TinyLlama Chat v1 and direct completion for GGUF or any other unverified profile;
7. keep network, database, backend, tokenizer, and native inference types out of Slint.

Generation is composed through E1 rather than Slint callbacks. The window selects
one of the two E1 products and shows backend, source, device, format, scalar type,
quantization, and immutable identity from E1 state. Verified TinyLlama exposes Chat
mode and conversation regeneration. GGUF exposes Direct completion mode, calls
`start_generation`, and does not infer a chat template or history semantics.

See the [application runtime guide](application-runtime.md) for the complete E1 public boundary.

## Frontend-neutral orchestration

`application-runtime` is the E1 application engine. It owns:

- the bounded Hugging Face resolver worker and synchronous local GGUF inspection;
- immutable selection and identity validation before loading;
- persisted application preferences and Hugging Face model catalogue updates;
- two monomorphized, process-hosted E0 workers with one active backend;
- closed Hugging Face/GGUF tokenizer and streaming-decoder dispatch;
- loaded-generation and active-request state;
- shared direct-completion, lifecycle, cancellation, and unload semantics;
- verified TinyLlama chat, raw conversation state, regeneration/supersession, context planning, and diagnostics;
- normalized structured events;
- bounded shutdown commands and joins for both E0 workers and the Hub worker.

Its public state and events contain application and domain values rather than
Slint, Candle, llama.cpp/GGUF, Hugging Face, redb, or Flume types. Slint maps only
those E1 values and never constructs a backend source. A Tauri, CLI, or another
native frontend could drive the same in-process use cases without duplicating
backend orchestration.

`desktop-slint` owns only:

- per-user application-data path selection;
- Slint component construction;
- closed product-input mapping to `ModelSelection`;
- callback-to-E1-command mapping;
- E1 model-metadata label mapping;
- one 16 millisecond frame cadence for bounded event draining and one decoded-output pull;
- presentation-owned chat/direct-completion transcript formatting plus batched fragment and terminal-state mapping;
- control and usage synchronization from `ApplicationState`;
- process exit reporting.

The binary `src/main.rs` delegates directly to the Slint library entry point.

A standalone browser-only application cannot execute either native E0 worker
directly. It would require a future transport adapter to an E1 host; no
`application-api` or browser transport exists today. A Tauri application can invoke
the same native engine directly from its Rust backend.

## Artifact acquisition and local resolution

`hf-hub-adapter` accepts a validated repository and revision, inspects repository
metadata, and resolves:

- `config.json`;
- `tokenizer.json`;
- `model.safetensors`, standard numbered shards, or shards named by
  `model.safetensors.index.json`.

Repository-relative paths are rejected if they are absolute, empty, or contain
non-normal path components. After repository inspection, every download is
performed through a second repository handle pinned to the returned immutable
commit, so a moving branch cannot mix artifacts from different revisions. Cache
paths are never persisted as model identity; repository, requested revision, and
immutable commit remain the logical identity.

The adapter reads the cached configuration's `dtype` or legacy `torch_dtype`
field and recognizes F32, F16, and BF16. The application runtime rejects loading
when the declaration is absent or unsupported. It also rejects a load request if
the visible repository or revision changed after artifact resolution, preventing
a stale resolved model from being loaded under different UI text. The Candle
adapter still validates every tensor's actual scalar type during loading, so
configuration metadata is never trusted as the final authority.

The adapter is synchronous by design and runs only on a dedicated cold-path Hub
worker. Environment-derived Hugging Face cache and token configuration remains
active unless the application explicitly overrides it.

Local GGUF resolution does not use that worker. `resolve_model` synchronously
canonicalizes the selected path, computes SHA-256 before and after bounded metadata
inspection, rejects content changed during inspection, and builds the tokenizer from
the same verified source. E1 retains the canonical path and digest as the complete
selection and immutable identity. Loading reuses the verified digest and checks the
loaded descriptor, tokenizer vocabulary, backend, and metadata against resolution
evidence.

## Tokenizer boundary

The Hugging Face product uses `hf-tokenizer`; the GGUF product uses the tokenizer
and stateful decoder derived from the verified GGUF source. Both implement the
portable `tokenization` contracts. E1 keeps their closed dispatch private, writes
initial prompt encoding into a caller-owned token sink, and owns one request-local
streaming decoder because correct fragments can depend on surrounding token IDs,
whitespace state, and incomplete byte fallback sequences.

Neither adapter claims allocation-free execution. Upstream encoding and streaming
decode may allocate internally; those costs remain quarantined from the portable
feature contracts and must be measured before they enter a strict hot path.

## Persistence

`redb-storage` stores:

- application memory and drain-timeout settings;
- default repository and revision;
- logical model catalogue entries.

Records use explicit four-byte kind markers, a numeric schema version, fixed
little-endian numeric fields, and length-prefixed UTF-8 strings. Rust struct
layout and third-party serialization formats are not treated as the persistent
schema. Each write occurs in a redb transaction.

## Slint event cadence

The Slint thread owns the component and a repeated 16 millisecond timer. Each
tick drains at most 64 structured application events, pulls exactly one bounded E1
decoded-output batch, applies one presentation delta, then synchronizes controls and
usage from `ApplicationState`. Conversation ownership remains in E1; Slint formats a
snapshot when a turn starts, after a submission error that may have committed user
history, and when terminal/cleanup events change response provenance. It appends only
the new assistant frame fragment while streaming. Worker token or network frequency therefore cannot
directly enqueue Slint callbacks or trigger one layout update per generated token.

The presenter copies borrowed text fragments before E1 reuses its accumulator, filters
them by request identity, and sends only the new frame fragment through one Slint append
callback. The same read-only transcript `TextEdit` retains selection ownership; append and
turn-start replacement callbacks preserve its viewport. A turn-start snapshot is formatted
from E1 raw records so regeneration can show superseded provenance. Terminal snapshot
replacement is applied after that frame's output pull so final fragments are not duplicated.
Pulling remains unconditional after terminal state so final text and `Released` records
cannot be stranded. Clear conversation invokes E1, is disabled/rejected until the active
generation and cleanup lifecycle reaches release, and resets the transcript only after
semantic history clears.

## Generated Rust and unsafe linting

Slint-generated Rust applies a local `allow(unsafe_code)` around generated item
vtable code. A crate-level or workspace-level `forbid(unsafe_code)` cannot be
lowered and therefore rejects valid generated output with error E0453.

The workspace uses `unsafe_code = "deny"`, while project-authored pure crates
continue to declare `#![forbid(unsafe_code)]`. The Slint library and binary use
`#![deny(unsafe_code)]`. This keeps authored unsafe code a compilation error while
allowing the generated module to set the lint level required by Slint.

## Shutdown behavior

Local shutdown remains deterministic and bounded across both E0 endpoints:

1. stop application admission and request cooperative Hub shutdown;
2. submit distinct shutdown tickets to the Candle and GGUF workers, waiting only to each configured deadline;
3. attempt bounded completion and join for both workers, including the inactive endpoint, even if the first reports an error;
4. finish the Hub-worker join, or detach any worker that cannot finish because a synchronous vendor call has failed to return.

All shutdown and join deadlines use checked `Instant` arithmetic and each poll or
sleep is capped to the remaining budget. Timeout overflow is rejected as invalid
configuration rather than panicking. Explicit `ApplicationRuntime::shutdown` is
mandatory on normal frontend closure because `Drop` intentionally performs no
unbounded join; the Slint runner calls it immediately after the window loop exits.
If window construction fails after runtime startup, the runner also performs explicit
shutdown before returning. Combined Slint and shutdown failures are preserved in one
`DesktopError` rather than hiding the cleanup failure.

Hub resolution is different. The upstream synchronous `hf-hub` builder exposes
cache, authentication, retry, endpoint, and progress controls, but no global
request timeout or cancellation handle. The application runtime sends a
cooperative shutdown command and waits for a bounded interval. If an HTTP
operation is still in flight at the deadline, its thread handle is detached and
application exit continues; the operating system reclaims process resources.
Both E0 runtime joins use the same bounded-exit rule because safe Rust cannot
destroy model state while an uncooperative backend call still holds it.

A future cancellable Hub implementation should replace only `hf-hub-adapter` and
its worker composition. It must not alter feature, inference, storage, or
frontend contracts.

## State location

The Slint runner stores the database under the user's application-data root:

- `XDG_DATA_HOME/llm-app/state.redb` when configured;
- `%LOCALAPPDATA%\\llm-app\\state.redb` on Windows, with `%APPDATA%` fallback;
- `~/Library/Application Support/llm-app/state.redb` on macOS;
- `~/.local/share/llm-app/state.redb` on other Unix desktops.

Other frontends supply their own database path through
`ApplicationRuntimeConfiguration`.
