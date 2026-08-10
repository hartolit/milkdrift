# desktop-slint

Thin Slint presentation adapter over `application-runtime` (E1).

## Supported local model flow

The frontend accepts a repository and revision as E1's device-independent `ModelSelection`. It projects only application-level resolution facts: whether resolution succeeded, a recognized configuration scalar declaration when present, and chat compatibility. Backend engine, source, artifact format, immutable commit, tokenizer vocabulary size, and model-handle generation remain implementation details.

CPU is the fresh-install default. A compact device `ComboBox` presents E1's explicit CPU/CUDA catalogue. Rust owns stable `ApplicationDevice` identity/index mapping, so labels are never parsed for semantics. Unavailable devices have a distinct label; persisted unavailable CUDA remains selected and visible, and neither Slint nor E1 falls back to CPU.

Selection enabled state comes from E1's `can_select_device` lifecycle policy, and load enabled state comes from E1's selected-device availability/lifecycle state. The UI presents the selected device independently, a recognized configuration declaration when one exists, and the actual execution scalar and execution device from the loaded-model receipt. A configuration declaration is producer-intent metadata, not an execution fact. When model resources remain retained, the same summary explicitly projects exact, unverified, or unknown ownership certainty plus E1's cleanup disposition; it does not present the model as unloaded. Repository and revision inputs remain locked while lifecycle ownership prevents editing.

## Generation presentation

The composer maps E1's `ApplicationState::generation_mode()` directly. In **Chat** mode it delegates message submission, regeneration, conversation ownership, prompt rendering, and conversation snapshots to E1.

Otherwise the composer is labeled **Direct completion** and delegates the prompt to E1's ordinary `start_generation` lifecycle. The frontend displays exactly one prompt/completion transcript, does not infer chat roles, history, or template semantics, and does not offer regeneration. Clearing is available only after active generation has ended and resets the presentation.

## Frontend boundary

This crate owns only the Slint event loop, widget callbacks, stable device identity/index presentation mapping, and per-user database path. It preserves one 16 millisecond frame cadence, processes at most 64 E1 events per frame, performs one bounded output pull, and appends decoded text once per frame batch while preserving transcript selection and viewport state.

The crate depends on E1's application types and APIs. It does **not** import adapter crates, backend/source/format types, redb, Flume, tokenizer internals, or inference commands. Model resolution and loading, compatibility verification, immutable identity, generation lifecycle, cancellation, conversation state, and terminal ownership remain in E1.

The binary entry point delegates to the library so process startup remains lean.

Run the mandatory/default CPU graph with:

```text
cargo run --locked -p desktop-slint
```

On the exact supported Linux x86_64 RTX 5070 Ti matrix, opt into the `desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda` chain with:

```text
CUDA_COMPUTE_CAP=120 \
cargo run --release --locked \
    -p desktop-slint \
    --features cuda
```

A CUDA-enabled application can still explicitly select CPU. There is no generic `gpu` alias, CUDA is never enabled by default, and feature compilation alone is not hardware-execution evidence. See the sole [product support matrix](../../../docs/project/implementation-status.md).
