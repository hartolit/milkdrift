# desktop-slint

Thin Slint presentation adapter over `application-runtime` (E1).

## Supported local model flow

The frontend accepts a Hugging Face repository and revision as E1's device-independent `ModelSelection`. E1 resolves immutable Safetensors artifacts for Candle execution and reports artifact/source/format/source scalar/tokenizer/identity/compatibility facts without attaching a device.

CPU is the fresh-install default. A compact device `ComboBox` presents E1's explicit CPU/CUDA catalogue. Rust owns stable `ApplicationDevice` identity/index mapping, so labels are never parsed for semantics. Unavailable devices have a distinct label; persisted unavailable CUDA remains selected and visible, and neither Slint nor E1 falls back to CPU.

Selection enabled state comes from E1's `can_select_device` lifecycle policy, and load enabled state comes from E1's selected-device availability/lifecycle state. The UI presents the selected device independently, a resolved-source summary containing only the source scalar, and a loaded-execution summary containing explicit Source scalar, Execution scalar, and Actual device facts verified through E0's receipt. Source and execution scalar are never inferred from the device: a BF16 source may execute as F32 on CPU or remain BF16 on supported CUDA. Unload clears the loaded execution facts while the selected device remains unchanged. Repository and revision inputs remain locked while a lifecycle operation is busy or a model is loaded.

## Generation presentation

When E1 reports verified chat compatibility for the loaded model, the composer is labeled **Chat** and delegates message submission, regeneration, conversation ownership, prompt rendering, and conversation snapshots to E1.

Otherwise the composer is labeled **Direct completion** and delegates the prompt to E1's ordinary `start_generation` lifecycle. The frontend displays exactly one prompt/completion transcript, does not infer chat roles, history, or template semantics, and does not offer regeneration. Clearing is available only after active generation has ended and resets the presentation.

## Frontend boundary

This crate owns only the Slint event loop, widget callbacks, stable device identity/index presentation mapping, and per-user database path. It preserves one 16 millisecond frame cadence, processes at most 64 E1 events per frame, performs one bounded output pull, and appends decoded text once per frame batch while preserving transcript selection and viewport state.

The crate depends on E1's application types and APIs. It does **not** import adapter crates, concrete Candle or Hugging Face source types, redb, Flume, tokenizer internals, or inference commands. Model resolution and loading, compatibility verification, immutable identity, generation lifecycle, cancellation, conversation state, and terminal ownership remain in E1.

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
