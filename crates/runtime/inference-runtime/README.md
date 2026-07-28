# inference-runtime

Single-owner model registry, cleanup quarantine, and backend-independent generation engine.

The crate is generic over one concrete `ModelLoader`. It owns loaded model weights, request sequences, generation-safe handles, aggregate memory admission, cancellation, bounded drain escalation, synchronization, and unload. Model and request admission use prepare/validate/commit transactions. If explicit rollback fails, the runtime quarantines the only model or sequence cleanup handle, retains its memory and capacity accounting, and reports both the primary and cleanup failure classifications.

`RuntimeCommand::Generate` admits an already-tokenized direct-completion request. Before native sequence publication it validates prompt and sequence bounds, exact vocabulary-sized logits, output policy, backend memory, and all bounded host workspace payloads. Workspace accounting remains reserved through terminal output publication, even if backend cleanup completes while output is backpressured.

The hosted worker alternates bounded command handling, one fair generation opportunity, one cleanup-maintenance opportunity, unload/deadline maintenance, and nonblocking output publication. Sampling executes inside this crate through the portable `sampling` feature; a frontend never drives individual token steps.

Generated token IDs and ordered terminal state use `host-runtime`'s preallocated pull accumulator. Full output capacity yields without another backend step. Cleanup failure publishes pending and, when applicable, exhausted state while preserving the original terminal classification. Cleanup uses a configurable total-attempt limit and never retries a successfully released or exhausted resource automatically.

Phase 4 adds an ordinary download-free integration test that drives the real `CandleLlamaLoader` through this same E0 scheduler for token-limit and EOS completion, backpressure, cancellation, release, unload, a final empty runtime/model snapshot, and shutdown. The opt-in `candle_llama_smoke` example exercises a pinned local model and emits diagnostic lifecycle timings and RSS checkpoints.

See the [inference runtime guide](../../../docs/project/inference-runtime.md) for lifecycle, accounting, cancellation, output, and cleanup semantics.
