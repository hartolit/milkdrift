# inference-runtime

Single-owner model registry, cleanup quarantine, and backend-independent generation engine.

The crate is generic over one concrete `ModelLoader`. It owns loaded model weights, request sequences, generation-safe handles, aggregate memory admission, cancellation, bounded drain escalation, synchronization, and unload. Model and request admission use prepare/validate/commit transactions. If explicit rollback fails, the runtime quarantines the only model or sequence cleanup handle, retains its memory and capacity accounting, and reports both the primary and cleanup failure classifications.

`RuntimeCommand::Generate` admits an already-tokenized direct-completion request. Before native sequence publication it validates prompt and sequence bounds, advertised capabilities, exact vocabulary-sized logits, output policy, backend memory, and all bounded host workspace payloads. One composed admission transaction owns the preallocated task and sole uncommitted backend sequence until runtime accounting/indexes and then scheduler visibility commit. Its single rollback path either destroys that sequence or quarantines the same owner for cleanup retry. Loaded models must retain the complete admitted descriptor and report the exact requested execution device, actual execution scalar accepted by the load plan, and complete accounted footprint. The inspected source scalar remains distinct from the execution scalar selected by the load plan. Every ready prefill/decode result is checked for stable sequence identity/capacity/state, exact position advancement, and complete host F32 logits before sampling. Workspace accounting remains reserved through terminal output publication, even if backend cleanup completes while output is backpressured.

The hosted worker owns scheduler, event backlog, unload correlation, maintenance,
terminal-stop, clock, and polling state in one loop object. Both immediate and
timeout command receipt use the same application path. Each turn alternates
bounded command handling, one fair generation opportunity, one cleanup-maintenance
opportunity, unload/deadline maintenance, and nonblocking output publication;
full queues wait for the configured poll interval instead of spinning. Generation
advancement is split into allocation-free prefill, pending-token, decode, and
staged terminal-publication transitions. Sampling executes inside this crate
through the portable `sampling` feature; a frontend never drives individual token
steps.

Generated token IDs and ordered terminal state use `host-runtime`'s typed token
wrapper over its private preallocated bounded-output core. Full output capacity
yields without another backend step. Cleanup failure publishes pending and, when
applicable, exhausted state while preserving the original terminal classification.
Consumer-busy and capacity failures retry without losing pending output; mutex
poisoning is terminal and initiates bounded worker shutdown. Cleanup uses a
configurable total-attempt limit and never retries a successfully released or
exhausted resource automatically.

## Test coverage

`tests/native_backend_generation.rs` contains ordinary, download-free Candle tests over the project-generated synthetic fixture documented in `tests/fixtures/candle-llama/PROVENANCE.md`. They drive `CandleLlamaLoader` through the hosted E0 scheduler and cover model load, deterministic greedy and seeded sampling, EOS and token-limit completion, output backpressure, cancellation, sequence cleanup and release, unload, empty post-unload state, shutdown, and worker join. The separate harness-free `cuda_hardware` target requires the package-local `cuda-hardware-tests` feature and `MILKDRIFT_CUDA_TEST=1`; it proves CUDA receipt/snapshot identity, execution scalar, device accounting, prefill/decode, host-side sampling, synchronization, unload, and zero post-unload accounting. Deterministic test loaders in `tests/generation.rs`, `tests/runtime.rs`, and `tests/fault_injection.rs` retain backend-independent E0 contract and failure-path coverage, including wrong device ID/kind, execution scalar, and accounted-footprint rollback, without requiring a native model implementation.

## Opt-in local diagnostic

The `candle_llama_smoke` example is an E0 lifecycle and performance diagnostic for already-resolved local Candle Llama artifacts. It performs no network or Hugging Face resolution. The opt-in `runtime-benchmarks` external runner owns the controlled CPU/CUDA product procedure and drives E1’s production artifact resolver before exercising Candle through E0; keeping network resolution above this crate preserves E0’s network-free boundary. See [validation](../../../docs/project/validation.md#controlled-cpu-and-cuda-external-product-evidence).

See the [inference runtime guide](../../../docs/project/inference-runtime.md) for lifecycle, accounting, cancellation, output, and cleanup semantics.
