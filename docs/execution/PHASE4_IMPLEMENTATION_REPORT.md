# Phase 4 Closure Report

**Prepared:** 2026-07-25
**Implementation baseline:** `8de2ebf2811d5158e3439efe2114379de59322d0`
**Scope:** Candle CPU vertical slice plus lifecycle, validation-provenance, and external-fixture closure corrections

## Result

The Phase 4 implementation is complete. Formal closure is pending one final locked verification and one pinned external-model smoke on the commit produced after applying the closure patch.

The recorded baseline run proved the real Candle Llama path through E0: model inspection and load, prompt prefill, sampling, incremental decode, bounded token output, cancellation between backend calls, terminal and released publication, empty request/workspace/cleanup accounting, model unload, worker shutdown, and join.

The closure patch strengthens that evidence by:

- transitioning a successfully destroyed Candle sequence to `SequenceState::Finished`;
- explicitly destroying both adapter-test sequences before unload;
- requiring an empty runtime and model registry snapshot after unload in deterministic integration tests and the real-model smoke;
- removing the downloaded external model and machine-specific transcript from the repository;
- ignoring `.phase4/`, which is the documented local download location; and
- synchronizing the canonical status, root README, component guide, smoke procedure, and this report.

Because those changes touch source and tests, the recorded baseline output is not represented as proof for the new commit. Phase 5 remains gated on the final commands in [Final closure rule](#final-closure-rule).

## Phase 4 closure matrix

| Phase 4 requirement | Implemented closure |
|---|---|
| Prompt positions | Deterministic Candle fixtures verify prefill consumes the complete prompt and decode advances from the preserved position. |
| Final prefill logits | Token-identity fixture weights prove that the final prompt token controls the full-vocabulary logits used for sampling. |
| Decode progression | Interleaved sequences verify independent token and position progression. |
| Vocabulary logits | Adapter and E0 integration tests require exact vocabulary-sized caller-owned F32 output. |
| EOS handling | Candle E0 integration publishes the EOS token followed by terminal and released EOS outcomes. |
| Scalar compatibility | F32 and F16 execute using their supported CPU dtypes. BF16 source tensors are validated as BF16 and deliberately upcast to F32 because Candle 0.11 CPU matmul does not execute BF16 operands. Resident-weight and KV-cache admission use the execution dtype so memory is not undercounted. |
| Sequence destruction | Successful Candle destruction marks the sequence `Finished`; adapter tests assert the state and E0 tests require terminal `Released` publication. |
| Model unload | Completion, EOS, and cancellation paths unload with `RejectIfBusy`, then assert zero loaded models, zero retained accounting, and an empty model snapshot before shutdown. |
| Cancellation boundary | One-token output capacity creates deterministic backpressure; cancellation is observed before another backend call and ownership is released. |
| Real-model execution | The pinned `neubla/tiny-random-LlamaForCausalLM` revision generated eight tokens through the hosted E0 worker in the recorded baseline run. |
| Failure classification | The smoke distinguishes fixture/configuration failures from runtime/lifecycle failures. |
| Measurements | The recorded smoke captured load time, time to first token, decode throughput, cancellation latency, unload time, and RSS checkpoints. |
| Ordinary CI | Deterministic integration tests use a tiny project-authored committed fixture and require no network download. |
| External fixture hygiene | The pinned external model is downloaded into ignored `.phase4/` storage and is not redistributed by this repository. |

## Recorded baseline validation evidence

The following command completed successfully for the implementation assembled into the Phase 4 baseline:

```text
cargo run --locked --bin llm-app -- verify
```

That run passed:

- workspace architecture and dependency-policy validation;
- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets --locked`;
- `cargo test --workspace --locked`, including doctests and Candle fixture integration tests;
- `cargo clippy --workspace --all-targets --locked -- -D warnings`;
- `cargo doc --workspace --no-deps --locked`; and
- `cargo bench --workspace --no-run --locked`.

No GitHub Actions run is attached to the baseline commit. This section records the supplied local output and does not claim independent CI attestation.

## Recorded pinned smoke evidence

The downloaded model file passed the required SHA-256 check:

```text
49c20f32c6c597480fcaec5df2f86c645eabea765cbea1e67886dbae45e5c992
```

| Field | Recorded value |
|---|---|
| Repository | `neubla/tiny-random-LlamaForCausalLM` |
| Revision | `39ca1f8a1fc940377c5cb49a21aff73bb99b52f5` |
| Expected architecture | `LlamaForCausalLM` / runtime `Llama` |
| Prompt token IDs | `1,2,3` |
| Generated token IDs | `18568, 1727, 8705, 3598, 27426, 4496, 998, 16911` |

| Measurement | Recorded result |
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

Elevated post-unload RSS is not treated as evidence of retained model ownership: allocators may retain freed pages for reuse. Explicit ownership evidence consists of released records, empty accounting, the post-unload empty snapshot, successful worker shutdown, and clean process exit.

## Final closure rule

After applying the closure patch, create or identify the final commit and run:

```text
cargo run --locked --bin llm-app -- verify
```

Then run the exact procedure in [Phase 4 Candle Llama Smoke Procedure](phase4-candle-smoke.md). Record:

```text
git rev-parse HEAD
```

with the complete output from both commands. Phase 4 is formally closed, and Phase 5 may begin, only when both commands pass on that exact commit without further source changes.

## Remaining product limitations

- This is a CPU-only Candle Llama vertical slice.
- The E0 request accepts token IDs and emits token IDs; tokenizer ownership, incremental decoded-text streaming, E1 generation commands, and frontend generation remain Phase 5 work.
- The tiny random smoke model proves execution and lifecycle integration, not language quality.
- Strict allocation-free Candle execution is not claimed because upstream Candle allocates intermediate and KV-cache tensors.
- GPU execution, general chat rendering, and GGUF UI generation remain unsupported.
