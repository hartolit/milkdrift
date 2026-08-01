# Current execution context

**Status date:** 2026-08-02
**Clean implementation start:** `HEAD` `f61a0fadd2311a53e1bce55094f886e3465b0c95`, tree `1bee6fa25f8b4819ac68d02cc10324f0f1848e9e`
**Start-state evidence:** source/index clean; only root `./target/` present; `cargo xtask verify` passed before edits
**Current target:** Phase 10 repository acceptance is complete; Phase 11 is not active
**Implementation state:** Phase 10 work packages 10.1–10.6 are implemented and accepted
**Evidence state:** focused checks, selected measurements, full benchmark compilation, canonical verification, dependency policy, offline links, whitespace, and target hygiene passed
**External-product state:** pinned opt-in E1 real-product mode compiled but was not executed; no external product baseline is claimed
**Canonical plan:** [execution-plan.md](execution-plan.md)
**Current product truth:** [project implementation status](../../project/implementation-status.md)
**Historical evidence:** [execution history](history.md)

This is the dense handoff for the accepted Phase 10 benchmark program. It distinguishes the clean starting-tree gate, final validation on the implementation diff, controlled synthetic/component evidence, and still-outstanding external product evidence. The pre-edit canonical pass is not reused as proof for the Phase 10 diff.

## Product boundary remains unchanged

The supported product is still one CPU-only local composition:

```text
Slint or another native frontend
        ↓
application-runtime (E1)
        ├── one bounded Hugging Face Hub worker
        ├── one Hugging Face tokenizer/decoder path
        ├── redb application persistence
        └── one Candle E0 worker/thread
                    ↓
             inference-runtime (E0)
                    ↓
     Candle + Safetensors + CPU
```

`ModelSelection` remains a normalized Hugging Face repository/revision pinned to an immutable Hub commit. Direct completion remains available to loaded models; built-in chat remains limited to `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6` with `</s>` token ID 2.

Phase 10 changed no production public API or production implementation. It added no optimization, unsafe code, GPU, GGUF/quantized path, second engine, hosted-provider/peer/browser path, multi-model residency, or other product axis.

## Implemented Phase 10 surfaces

### 10.1 — Sampling component benchmark

`crates/domain/sampling/benches/sampling_pipeline.rs` now runs the public sampler over an explicit matrix:

- boundaries: `sample_only` and `restore_and_sample`;
- cases: greedy, default top-k/top-p, min-p, and varied repetition histories;
- vocabulary sizes: 8,192, 32,768, and 131,072;
- stop matching: token hit, last-pattern hit, and miss.

`sample_only` restores mutable logits before timing. `restore_and_sample` includes restoration. Target documentation states the question, included/excluded work, and evidence limits; setup and capacity allocation stay outside the measured loop.

### 10.2–10.5 — Sole system package and separated harness roles

The only root benchmark package is `benchmarks/runtime` (`runtime-benchmarks`). It is an outer non-production consumer, uses the root workspace/lock/target, is non-publishable, has no build script, and has no incoming production dependency. Its external normal dependencies are exactly `serde`, `serde_json`, and `sha2` 0.11; `sha2` recomputes the committed fixture hashes before normal-runner or Criterion setup.

| Surface | Role and current evidence | Explicit limitation |
|---|---|---|
| Normal `baseline` binary, synthetic mode | Bounded download-free public-E0 start/load/prefill/generation/backpressure/cancellation/unload/shutdown cycles, runtime/model accounting, process RSS, and matching fresh E1 start/shutdown cycles | Synthetic integration/lifecycle evidence, not product-model performance or quality |
| Normal `baseline` binary, real-product mode | Pinned public-E1 path for `neubla/tiny-random-LlamaForCausalLM` commit `1c81a3fba044af78df253edc66bdbab183184932`; requires `--allow-network` plus an existing cache under shared root `target/` or outside the repository and rejects `HF_HUB_OFFLINE=1` | Compiled only; not executed; no current product baseline |
| Criterion `runtime` target | Hosted public-E0 checked-prefill and incremental-decode command/event boundaries against the deterministic fixture | Component-regression evidence, not E1 latency, full generation throughput, RSS, or an allocation count |
| Stable report/metadata/RSS code | Versioned serde JSON to stdout, progress/summary to stderr, typed lifecycle/accounting records, Git/toolchain/target/host/workload identity, exact SHA-256 fixture verification, and tested CLI/cache/fixture/toolchain/CPU/`/proc` paths | No environment-wide dump, token IDs/text, secret capture, native-resource attribution, or result-file writer |

Tokenizer encode/streaming-decode, context-planner, output-accumulator, and isolated Candle microbenchmarks were reviewed and deferred. None had both a named unresolved question and evidence that the implemented public E0/system surfaces were insufficient. No placeholder benchmark directory or copied production logic was added.

### 10.3 — Controlled lifecycle and memory evidence

The release synthetic run used one warmup and three recorded samples:

```text
cargo run --release --locked -p runtime-benchmarks --bin baseline -- \
  --mode synthetic --warmup 1 --cycles 3
```

The final corrected run completed successfully. All nine generation operations across the three recorded cycles reached matching `Terminal` and `Released` states with no pending or exhausted cleanup; every cancellation emitted two tokens. Every release returned accounting to model-only state, model unload ended with exact zero model/request/workspace/cleanup/maintenance accounting, E0 shutdown/join was clean, and the paired download-free E1 start/shutdown cycles were clean.

RSS is a sampled process-wide Linux `/proc` observation when available. It includes the executable, workers, mappings, allocator caches, and unrelated process-resident state; it can miss transient peaks and cannot attribute Candle/native/device allocations. Public E0 accounting is deterministic ownership/admission evidence and is intentionally reported separately from RSS.

### 10.6 — No speculative optimization

No measured result was used to justify a production change. There is no new inline mandate, custom allocator, SIMD, unsafe block, lock-free structure, collection/data-layout rewrite, or timing threshold. Shared CI is intended to compile the targets and catch API drift, not enforce wall-clock performance.

## Focused validation already recorded

The following evidence passed after implementation:

- `cargo metadata --locked --format-version 1 --no-deps`;
- the required package-level `cargo check`, `cargo test`, and strict `cargo clippy` command set for `sampling`;
- the required package-level `cargo check`, `cargo test`, and strict `cargo clippy` command set for `runtime-benchmarks`—15 tests passed;
- `cargo test --locked -p xtask`—32 total tests passed (14 unit and 18 integration tests; doc tests contained no cases).

Selected Criterion estimate intervals were:

| Target | Selected interval |
|---|---:|
| `e0_hosted_checked_prefill/4_tokens` | `[5.0955, 5.1104] ms` |
| `e0_hosted_incremental_decode/1_token_after_2_token_prefill` | `[5.0474, 5.0890] ms` |
| `sample_only/default_top_k_top_p/32768` | `[77.211, 77.228] µs` |
| `restore_and_sample/default_top_k_top_p/32768` | `[78.288, 78.302] µs` |

Exactly these four Criterion targets were statistically executed; every other sampling matrix target and all stop-matching targets are compile-only evidence. The intervals are controlled synthetic/component evidence only. They are not a production model baseline, language-quality result, broad compatibility claim, representative steady-state serving result, or shared-CI threshold.

## External evidence not taken

The immutable real-product mode was compiled but not run. The current public E1 resolver requires Hub metadata resolution, so any future run must use `--allow-network`, reject `HF_HUB_OFFLINE=1`, and supply an existing cache whose canonical path is under shared root `target/` or outside the repository. No output, latency, throughput, lifecycle result, or product comparison has been inferred; no current product baseline exists.

## Final acceptance evidence

The exact Phase 10 diff passed:

- `cargo bench --workspace --no-run --locked`;
- `cargo xtask verify`, including architecture, hygiene, workspace tests, strict Clippy, documentation, and benchmark compilation;
- `cargo deny --workspace --locked check advisories bans licenses sources` (configured duplicate-version warnings only; all four checks passed);
- `lychee --config lychee.toml --offline '**/*.md'` with zero errors;
- `git diff --check`;
- repeated target-directory checks showing only root `./target`.

The first final canonical attempt exposed an oversized `xtask` benchmark-policy test helper. The helper was split by responsibility without weakening policy; focused strict `xtask` Clippy and the complete canonical rerun then passed.

No portability command was required because no portable production library source or dependency changed: `sampling` changes are its benchmark, component guide, and package readme metadata only. Phase 10 repository acceptance is **complete**. The external real-product measurement remains an explicit evidence gap rather than a hidden acceptance claim; Phase 11 is not activated by this closure.
