# Desktop runtime

## Scope

`desktop-slint` is the thin native reference host for the optional
`application-runtime` services kit. It demonstrates one current desktop
composition; it is not Milkdrift's universal API or workflow plane.

The desktop host owns native process and presentation concerns:

- per-user database-path selection;
- Slint component construction and event-loop integration;
- repository/revision input mapping to `ModelSelection`;
- Rust-owned device identity/index mapping;
- callback mapping to coarse E1 operations;
- frame-batched text and transcript presentation; and
- normal-close shutdown and process-exit reporting.

E1 owns the model/device lifecycle, immutable resolution, generation and compatible
conversation semantics, persistence, retained cleanup, and worker coordination.
Slint receives no Candle, Safetensors, Hub-client, redb, Flume, tokenizer, or E0
command types.

A browser-only application cannot execute native E0 directly. It would need an
explicit transport to a native or remote host. No browser transport or generic
`application-api` is implied by this reference application.

## Model and device projection

The reference UI deliberately projects a small application-owned fact set:

- selected repository and revision;
- recognized-or-absent configuration declaration, without treating absence as
  mixed-layout conversion authority;
- selected device identity and availability;
- receipt-reported actual execution scalar and device; and
- retained resource, ownership certainty, cleanup disposition, and failures.

The unit `ChatCompatibility::{Supported, Unsupported}` fact controls whether the
chat composer is available; no private prompt profile crosses the boundary.

`ApplicationDeviceSummary` is structured data, not a label DTO. It provides
`ApplicationDevice`, availability and discovery facts, memory/capability
observations, and an optional backend-reported `display_name`. Slint constructs its
own CPU/CUDA/ordinal/availability labels and keeps the exact Rust identity in an
index model. Labels are never parsed back into semantics.

The UI does not project or select an engine, artifact source, format helper,
per-tensor layout, required scalar, conversion policy, fallback, or local model
path. Selected device and resolved declaration remain independent from loaded
execution. Unload clears actual execution facts but preserves selection and
resolution.

Retained state and normal loaded state are mutually exclusive. While
`ApplicationState::retained_model()` is present, editing selection, selecting a
device, resolving, loading, and generation remain locked. Worker disconnect is
shown as `WorkerDisconnected`; it is not presented as release.

The presenter reads `ApplicationState` capability projections for busy state,
selection editing, conversation clearing, resolution, load, generation,
cancellation, cleanup retry, and unload. Slint does not reconstruct phase legality
from activity plus optional model fields.

## Resolution and loading

The desktop maps repository/revision input to E1. The Hub worker pins that selection
to an immutable commit and resolves `config.json`, `tokenizer.json`, and the
supported unquantized Llama Safetensors layout.

Declaration handling is strict:

- absent/null `dtype` and `torch_dtype` continue with no declaration;
- recognized `F32`, `F16`, or `BF16` continues as producer-intent metadata;
- malformed, unsupported, or conflicting present declarations fail during
  resolution with stable application-owned failure categories and no raw vendor
  value.

Candle determines required tensors, conversion, materialization, and execution.
E1 requires only nonempty complete observed scalar evidence and does not reject
truthful unused `F16`, `BF16`, `I8`, `U8`, or `Other` categories. E1's final
footprint check uses checked host/device totals and the fixed budget; it does not
apply a CPU/CUDA placement rule. The desktop shows only the actual execution scalar
and device verified by the E0 receipt.

CPU is the fresh-install default. CUDA remains an explicit opt-in through
`desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`. An
unavailable persisted CUDA selection remains selected and visible; neither E1 nor
Slint silently substitutes CPU.

## Retained cleanup presentation

`ModelCleanupPending { resource, disposition }` is a compact transition event.
Slint rereads the complete `ApplicationRetainedModel` from `ApplicationState`
and formats:

- the retained resource;
- `Exact`, `Unverified`, or `Unknown` ownership;
- `Pending`, `LowerRetryable`, `LowerExhausted`,
  `CoordinationRetryAvailable`, `WorkerDisconnected`, or
  `RetainedUntilProcessExit`; and
- independent primary and cleanup/coordination failures.

`ModelCleanupReleased { resource }` is the explicit retained model-owner release
event outside an in-flight unload. Cleanup released during a correlated unload
remains pending until `ModelUnloaded`, including any successor cleanup owner for the
same model. Disconnect, worker-handle absence, a missing snapshot owner, or zero
exact aggregate bytes is never rendered as release proof.

`ApplicationRuntime::retry_model_cleanup` is available to hosts when
`ApplicationState::can_retry_model_cleanup()` reports
`CoordinationRetryAvailable`. It retries E1 coordination only; it does not reset
lower exhaustion or process-lifetime retention.

## Persistence

`redb-storage` stores settings and logical model catalogue records. It does not
persist runtime execution or ownership facts.

Application settings write `LAS1` version 2 and read exact version 1. Model records
write latest `LAM1` version 3:

```text
presence 0             -> no configuration declaration
presence 1 + code 0    -> F32
presence 1 + code 1    -> F16
presence 1 + code 2    -> BF16
```

Exact `LAM1` versions 1 and 2 remain readable and are not automatically rewritten;
only an explicit write emits version 3. The timestamp field is
`last_resolved_unix_milliseconds`. Wrong magic, unknown version/tag/code,
truncation, invalid UTF-8, trailing bytes, invalid fields, and table-key versus
embedded-name mismatch are explicit errors.

Observed tensor sets, required scalar policy, execution scalar/device, footprints,
shard/cache identity, loaded state, and retained cleanup state are not persisted.

## Slint event cadence

The Slint thread owns one repeated 16 millisecond timer. Each tick:

1. drains at most 64 structured E1 events;
2. performs exactly one bounded decoded-output pull;
3. applies one presentation delta; and
4. synchronizes controls and usage from `ApplicationState`.

Output pulling remains unconditional through terminal presentation so final text
and release records are not stranded. The presenter copies borrowed fragments
before E1 reuses its accumulator and filters them by request identity. Worker token
frequency therefore does not create one Slint callback or layout update per token.

## Generated Rust and unsafe linting

Slint-generated Rust receives a narrow local `allow(unsafe_code)` for generated
vtable code. Project-authored Slint code remains under `#![deny(unsafe_code)]`, and
pure authored crates retain their stronger lint policy.

## Shutdown

The runner calls `ApplicationRuntime::shutdown` after the window loop and after a
post-startup window-construction failure. Shutdown attempts Hub stop, E0 shutdown,
and both joins even if an earlier step fails. Retryable timeouts retain unfinished
handles for a later call.

Cleanup and joining are independent facts. A clean correlated E0 shutdown result
can prove release; disconnect or join-handle absence cannot. Terminal cleanup
retention remains visible as `RetainedUntilProcessExit` even after the worker has
stopped. Combined Slint and shutdown failures remain visible in `DesktopError`.

## State location

The runner stores `state.redb` under the platform application-data root:

- `XDG_DATA_HOME/milkdrift/state.redb` when configured;
- `%LOCALAPPDATA%\\milkdrift\\state.redb` on Windows, with `%APPDATA%` fallback;
- `~/Library/Application Support/milkdrift/state.redb` on macOS; and
- `~/.local/share/milkdrift/state.redb` on other Unix desktops.

When the Milkdrift path does not yet contain a database but the former `llm-app/state.redb` path does, startup atomically moves that file into the Milkdrift directory. An existing Milkdrift database always wins and the legacy file is left untouched; migration failure is explicit rather than silently starting with empty state.

Other native hosts choose their own path and pass it to
`ApplicationRuntimeConfiguration::new`.

This guide makes no current-tree validation or hardware-support claim. See
[implementation status](implementation-status.md) for the canonical evidence and
support matrix.
