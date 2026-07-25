# Desktop Runtime

## Scope

The desktop composition owns native process concerns while delegating model
lifecycle and direct-completion orchestration to `application-runtime`:

1. resolve one immutable Hugging Face model revision;
2. cache and validate its required artifacts;
3. load its serialized tokenizer;
4. persist the logical repository/revision selection;
5. load one Candle CPU model;
6. expose E1 direct-completion start, cancel, text-pull, and unload behavior;
7. keep all network, database, vendor, and UI types outside portable features.

Phase 5 wires generation through E1, not through Slint callbacks. The current Slint
window remains lifecycle-only until Phase 6 adds prompt input, generated-output
presentation, generate/cancel controls, and frame-aligned text pulls. General chat
templates and conversation history remain later work.

See the [application runtime guide](application-runtime.md) for the complete E1 public boundary.

## Frontend-neutral orchestration

`application-runtime` is the E1 application engine. It owns:

- the bounded Hugging Face resolver worker;
- tokenizer validation;
- persisted application preferences and model catalogue updates;
- exact-selection validation before loading;
- the hosted inference-runtime endpoint;
- loaded-generation and active-request state;
- direct-completion prompt/tokenizer/text orchestration;
- explicit reject/cancel/drain unload behavior;
- normalized structured events;
- bounded worker shutdown and joins.

Its public state and events contain application and domain values rather than
Slint, Candle, Hugging Face, redb, or Flume types. A Slint, Tauri, CLI, or another
native frontend can therefore drive the same use cases without duplicating
backend orchestration.

`desktop-slint` owns only:

- per-user application-data path selection;
- Slint component construction;
- callback-to-command mapping;
- 16 millisecond event polling;
- structured-event presentation;
- process exit reporting.

The binary `src/main.rs` delegates directly to the Slint library entry point.

A standalone browser-only Leptos application cannot execute the native Candle
runtime directly. It would use a transport adapter to a native or remote
`application-runtime` host. A Tauri application can invoke the same native
engine directly from its Rust backend.

## Artifact acquisition

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

The adapter is synchronous by design and runs only on a dedicated cold-path host
worker. Environment-derived Hugging Face cache and token configuration remains
active unless the application explicitly overrides it.

## Tokenizer boundary

`hf-tokenizer` adapts the upstream tokenizer to the portable `tokenization`
contracts. Initial prompt encoding writes into a generic caller-owned token sink.
Model output uses a request-local stateful decoder because correct text fragments
can depend on surrounding token IDs, whitespace state, and incomplete byte
fallback sequences.

The adapter does not claim allocation-free execution. Upstream encoding and
streaming decode may allocate internally; those costs remain quarantined from the
portable feature contracts and must be measured before they enter a strict hot
path.

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
tick pulls at most a fixed number of structured application events. Worker token
or network frequency therefore cannot directly enqueue Slint callbacks or
trigger unbounded layout work.

E1 generation output already uses `host-runtime`'s preallocated pull-oriented
accumulators. Phase 6 will make the Slint frame timer pull one bounded decoded-text
batch per frame, append it to presentation state, and release the borrowed storage
promptly.

## Generated Rust and unsafe linting

Slint-generated Rust applies a local `allow(unsafe_code)` around generated item
vtable code. A crate-level or workspace-level `forbid(unsafe_code)` cannot be
lowered and therefore rejects valid generated output with error E0453.

The workspace uses `unsafe_code = "deny"`, while project-authored pure crates
continue to declare `#![forbid(unsafe_code)]`. The Slint library and binary use
`#![deny(unsafe_code)]`. This keeps authored unsafe code a compilation error while
allowing the generated module to set the lint level required by Slint.

## Shutdown behavior

Model shutdown remains deterministic at the inference boundary:

1. submit a runtime shutdown command;
2. wait for the matching ticketed shutdown event;
3. wait a bounded interval for the exclusively owning runtime worker to finish;
4. join the completed worker, or detach it at process shutdown if a backend call
   has failed to return.

All shutdown and join deadlines use checked `Instant` arithmetic and each poll or
sleep is capped to the remaining budget. Timeout overflow is rejected as invalid
configuration rather than panicking. Explicit `ApplicationRuntime::shutdown` is
mandatory on normal frontend closure because `Drop` intentionally performs no
unbounded join; the Slint runner calls it immediately after the window loop exits.

Hub resolution is different. The upstream synchronous `hf-hub` builder exposes
cache, authentication, retry, endpoint, and progress controls, but no global
request timeout or cancellation handle. The application runtime sends a
cooperative shutdown command and waits for a bounded interval. If an HTTP
operation is still in flight at the deadline, its thread handle is detached and
application exit continues; the operating system reclaims process resources.
The runtime join uses the same bounded-exit rule because safe Rust cannot destroy
model state while an uncooperative backend call still holds it.

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
