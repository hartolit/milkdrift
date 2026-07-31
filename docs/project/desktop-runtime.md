# Desktop runtime

## Scope

The desktop composition owns native process and presentation concerns while delegating model lifecycle, generation, and compatible conversation orchestration to `application-runtime`:

1. map Hugging Face repository/revision inputs to E1's `ModelSelection`;
2. ask E1 to resolve immutable Hub artifacts and the concrete Hugging Face tokenizer;
3. load one Candle/Safetensors model on the CPU E0 worker;
4. expose E1 submit, regenerate, clear, cancellation, bounded text-pull, and unload behavior;
5. use verified chat only for the exact TinyLlama Chat v1 profile and direct completion otherwise;
6. keep network, database, backend, tokenizer, and inference-command types out of Slint.

The UI presents engine, artifact source, device, format, scalar type, and immutable Hub identity from E1 state. It has no backend/product selector and no local-file model path. See the [application runtime guide](application-runtime.md) for the complete E1 boundary.

## Frontend-neutral orchestration

`application-runtime` owns:

- the bounded Hugging Face resolver worker;
- immutable repository/revision and commit validation before loading;
- persisted application preferences and model catalogue updates;
- one monomorphized, process-hosted Candle E0 worker;
- the concrete Hugging Face tokenizer and request-local streaming decoder;
- loaded-generation and active-request state;
- direct completion, cancellation, backpressure, cleanup, and unload semantics;
- verified TinyLlama chat, raw conversation state, regeneration/supersession, context planning, and diagnostics;
- normalized events and bounded text output;
- bounded shutdown commands and joins for the E0 and Hub workers.

Its public state/events contain application and domain values rather than Slint, Candle, Hugging Face adapter internals, redb, or Flume types. A Tauri, CLI, or another native frontend could drive the same in-process use cases without duplicating orchestration.

`desktop-slint` owns only:

- per-user application-data path selection;
- Slint component construction;
- repository/revision input mapping to `ModelSelection`;
- callback-to-E1-command mapping;
- E1 model-metadata label mapping;
- one 16 millisecond frame cadence for bounded event draining and one decoded-output pull;
- presentation-owned chat/direct-completion transcript formatting plus frame-batched fragments and terminal state;
- control and usage synchronization from `ApplicationState`;
- process exit reporting.

The binary `src/main.rs` delegates directly to the Slint library entry point.

A browser-only application cannot execute native E0 directly. It would require a future transport adapter to an E1 host; no `application-api` or browser transport exists. A Tauri application can invoke E1 from its Rust backend.

## Artifact acquisition

`hf-hub-adapter` accepts a validated repository and revision, inspects repository metadata, and resolves:

- `config.json`;
- `tokenizer.json`;
- `model.safetensors`, standard numbered shards, or shards named by `model.safetensors.index.json`.

Repository-relative paths are rejected if absolute, empty, or non-normal. After inspection, every required artifact is resolved through a repository handle pinned to the returned immutable commit, so a moving branch cannot mix revisions. Cache paths are not model identity; repository, requested revision, and immutable commit are the logical facts.

The adapter reads `dtype` or legacy `torch_dtype` and recognizes F32, F16, and BF16. E1 rejects loading when the declaration is absent or unsupported. It also rejects a load if the visible repository or revision changed after resolution. Candle validates actual tensor types during load, so configuration metadata is not the final authority.

The adapter is synchronous by design and runs only on the dedicated cold-path Hub worker. Environment-derived Hugging Face cache and token configuration remain active unless E1 explicitly overrides them.

## Tokenizer boundary

The current product uses `hf-tokenizer` for prompt encoding and `HfOwnedStreamingDecoder` for request-local decode. Both satisfy portable `tokenization` contracts. E1 writes prompt encoding into caller-owned token storage and owns the decoder because correct fragments can depend on surrounding tokens, whitespace state, and incomplete byte fallback sequences.

The tokenizer adapter does not claim allocation-free execution. Upstream encoding/decode may allocate internally; those costs remain outside portable allocation claims and require measurement before any stricter capability is advertised.

## Persistence

`redb-storage` stores:

- application memory and drain-timeout settings;
- default repository and revision;
- logical model catalogue entries.

Records use explicit four-byte kind markers, a numeric schema version, fixed little-endian numeric fields, and length-prefixed UTF-8 strings. Rust struct layout and third-party serialization formats are not treated as the persistent schema. Each write occurs in a redb transaction.

## Slint event cadence

The Slint thread owns the component and a repeated 16 millisecond timer. Each tick drains at most 64 structured E1 events, pulls exactly one bounded decoded-output batch, applies one presentation delta, then synchronizes controls and usage from `ApplicationState`.

Conversation ownership remains in E1. Slint formats a snapshot when a turn starts, after a submission error that may have committed user history, and when terminal/cleanup events change response provenance. It appends only the new assistant fragment while streaming. Worker token or network frequency therefore cannot enqueue one Slint callback or layout update per generated token.

The presenter copies borrowed fragments before E1 reuses its accumulator, filters by request identity, and sends one frame-batched append. Pulling remains unconditional after terminal state so final text and `Released` records cannot be stranded. Clearing invokes E1 and remains disabled/rejected until generation and cleanup reach release.

## Generated Rust and unsafe linting

Slint-generated Rust applies a local `allow(unsafe_code)` around generated vtable code. A crate/workspace `forbid(unsafe_code)` cannot be lowered and would reject valid generated output.

The workspace uses `unsafe_code = "deny"`; project-authored pure crates continue to use `#![forbid(unsafe_code)]`, and the Slint library/binary use `#![deny(unsafe_code)]`. Authored unsafe code remains a compilation error while the private generated module can use the lint level required by Slint.

## Shutdown behavior

Local shutdown is deterministic and bounded across the current workers:

1. stop application admission and request cooperative Hub shutdown;
2. submit one ticketed shutdown command to the Candle E0 worker and wait only to the configured deadline;
3. attempt bounded completion and join for the inference worker even if a prior step failed;
4. finish the Hub-worker join, or detach a worker that cannot finish because a synchronous vendor call has not returned.

All shutdown/join deadlines use checked `Instant` arithmetic and cap each poll or sleep to the remaining budget. Timeout overflow is invalid configuration rather than a panic. Explicit `ApplicationRuntime::shutdown` is mandatory on normal closure because `Drop` performs no unbounded join; the Slint runner calls it after the window loop and also after post-startup window-construction failure. Combined Slint and shutdown failures remain visible in one `DesktopError`.

The synchronous Hub client exposes cache, authentication, retry, endpoint, and progress controls but no global request timeout/cancellation handle. If an HTTP operation is still in flight at the deadline, the thread handle is detached and process exit continues. Safe Rust likewise cannot destroy model state while an uncooperative backend call holds it.

## State location

The Slint runner stores the database under the user's application-data root:

- `XDG_DATA_HOME/llm-app/state.redb` when configured;
- `%LOCALAPPDATA%\\llm-app\\state.redb` on Windows, with `%APPDATA%` fallback;
- `~/Library/Application Support/llm-app/state.redb` on macOS;
- `~/.local/share/llm-app/state.redb` on other Unix desktops.

Other frontends supply their own database path through `ApplicationRuntimeConfiguration`.
