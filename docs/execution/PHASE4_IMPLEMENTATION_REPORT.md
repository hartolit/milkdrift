# Phase 4 Completion Report

**Prepared:** 2026-07-24
**Scope:** Phase 4 Candle CPU vertical slice against the final applied working tree
**Baseline:** Phase 3 completion tree plus the Phase 4 implementation and validation corrections

## Result

Phase 4 is complete on the validated source tree.

The repository's canonical locked verification completed successfully under the
toolchain pinned by `rust-toolchain.toml`. The pinned external-model smoke also
completed successfully using the exact Safetensors fixture and revision documented
in `docs/execution/phase4-candle-smoke.md`.

The evidence proves the real Candle Llama path through E0: model inspection and
load, prompt prefill, sampling, incremental decode, bounded token output,
cancellation between backend calls, terminal and released publication, empty
request/workspace/cleanup accounting, model unload, worker shutdown, and join.

Phase 5 may begin after the validated tree is committed without further source
changes. If source changes are made before or after the commit, rerun both gates.

## Phase 4 closure matrix

| Phase 4 requirement | Validated closure |
|---|---|
| Prompt positions | Deterministic Candle fixtures verify prefill consumes the complete prompt and decode advances from the preserved position. |
| Final prefill logits | Token-identity fixture weights prove that the final prompt token controls the full-vocabulary logits used for sampling. |
| Decode progression | Interleaved sequences verify independent token and position progression. |
| Vocabulary logits | Adapter and E0 integration tests require exact vocabulary-sized caller-owned F32 output. |
| EOS handling | Candle E0 integration publishes the EOS token followed by terminal and released EOS outcomes. |
| Scalar compatibility | F32 and F16 execute using their supported CPU dtypes. BF16 source tensors are validated as BF16 and deliberately upcast to F32 because Candle 0.11 CPU matmul does not execute BF16 operands. Resident-weight and KV-cache admission use the execution dtype so memory is not undercounted. |
| Sequence destruction | Adapter tests destroy native sequences explicitly; E0 tests require terminal `Released` state and zero retained request, workspace, and cleanup accounting. |
| Model unload | Completion, EOS, and cancellation paths unload with `RejectIfBusy`, then shut down and join the worker. |
| Cancellation boundary | One-token output capacity creates deterministic backpressure; cancellation is observed before another backend call and ownership is released. |
| Real-model execution | The pinned `neubla/tiny-random-LlamaForCausalLM` revision loaded and generated eight tokens through the hosted E0 worker. |
| Failure classification | The smoke distinguishes fixture/configuration failures from runtime/lifecycle failures. |
| Measurements | The successful smoke recorded load time, time to first token, decode throughput, cancellation latency, unload time, and RSS checkpoints. |
| Ordinary CI | Deterministic integration tests use tiny committed/local Safetensors fixtures and require no network download. |

## Final corrective changes included

- Corrected the Candle backend test identifier construction from `u32` to the
  required `u64` backend identifier type.
- Preserved BF16 as source metadata while upcasting BF16 CPU execution to F32,
  matching Candle 0.11's supported CPU matmul dtypes.
- Updated BF16 resident-memory and KV-cache admission estimates to use the F32
  execution footprint.
- Merged identical scalar-type match arms to satisfy strict Clippy.
- Replaced an explicit test `panic!`, inlined format arguments, and removed an
  obsolete lint expectation so `cargo clippy -- -D warnings` passes.

## Canonical validation evidence

The following command completed successfully against the final source tree:

```text
cargo run --locked --bin llm-app -- verify
```

That command passed:

- workspace architecture and dependency-policy validation;
- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets --locked`;
- `cargo test --workspace --locked`, including doctests and the real Candle fixture integration tests;
- `cargo clippy --workspace --all-targets --locked -- -D warnings`;
- `cargo doc --workspace --no-deps --locked`; and
- `cargo bench --workspace --no-run --locked`.

## Pinned real-model smoke evidence

The downloaded model file passed the required SHA-256 check:

```text
49c20f32c6c597480fcaec5df2f86c645eabea765cbea1e67886dbae45e5c992
```

Smoke fixture:

| Field | Validated value |
|---|---|
| Repository | `neubla/tiny-random-LlamaForCausalLM` |
| Revision | `39ca1f8a1fc940377c5cb49a21aff73bb99b52f5` |
| Expected architecture | `LlamaForCausalLM` / runtime `Llama` |
| Prompt token IDs | `1,2,3` |
| Generated token IDs | `18568, 1727, 8705, 3598, 27426, 4496, 998, 16911` |

Recorded diagnostics:

| Measurement | Result |
|---|---:|
| Model load duration | 0.005661 s |
| Time to first generated token | 0.060969 s |
| Decode throughput | 21.954 tokens/s |
| Cancellation latency | 0.045297 s |
| Model unload duration | 0.000380 s |
| RSS before load | 4,636 KiB |
| RSS after load | 11,116 KiB |
| RSS during generation | 14,088 KiB |
| RSS after unload | 10,412 KiB |

The elevated post-unload RSS is not treated as evidence of retained model
ownership: allocators may retain freed pages for reuse. The runtime's explicit
ownership evidence is the successful released records, empty accounting, model
unload, worker shutdown, and clean process exit.

## Evidence provenance

The validation was recorded against the final local working tree before its Phase
4 commit. Add the resulting commit SHA here after committing the tree without
source changes:

```text
Phase 4 source commit: <record after commit>
```

## Remaining product limitations

- This is a CPU-only Candle Llama vertical slice.
- The E0 request accepts token IDs and emits token IDs; tokenizer ownership,
  incremental decoded-text streaming, E1 generation commands, and frontend
  generation remain Phase 5 work.
- The tiny random smoke model proves execution and lifecycle integration, not
  language quality.
- Strict allocation-free Candle execution is not claimed because upstream Candle
  allocates intermediate and KV-cache tensors.
- GPU execution, general chat rendering, and GGUF UI generation remain unsupported.

Phase 4 is complete. The next execution step is
**Phase 5 — Expose generation through `application-runtime`**.
