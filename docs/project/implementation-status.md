# Current implementation status

**Status date:** 2026-08-02
**Clean implementation start:** `HEAD` `f61a0fadd2311a53e1bce55094f886e3465b0c95` (tree `1bee6fa25f8b4819ac68d02cc10324f0f1848e9e`)
**Start state:** source/index clean; only root `./target/` present; baseline `cargo xtask verify` passed before edits
**Current validation scope:** Phase 10 sampling and `runtime-benchmarks` implementation, selected controlled measurements, and full repository acceptance
**Execution position:** Phase 10 work packages 10.1–10.6 and repository acceptance are complete; Phase 11 is not active
**Validation state:** focused checks, full benchmark compilation, canonical verification, dependency policy, offline links, whitespace, status, and root-target hygiene passed
**Product-evidence state:** the pinned opt-in E1 real-product mode compiled but was not executed; no current-tree product baseline is claimed
**Canonical plan:** [LLM App Execution Plan](../agent/execution/execution-plan.md)
**Current working context:** [Phase 10 execution context](../agent/execution/current.md)

This is the canonical product-level status page. Component behavior belongs in the corresponding project guide, accepted rationale belongs in [architecture decisions](../agent/decisions/README.md), repeatable commands belong in [validation](validation.md), and baseline-specific evidence belongs in [execution history](../agent/execution/history.md). This document deliberately does not claim the SHA or Git tree that contains itself; required CI logs those identities immediately before validation.

## Supported devices and products

| Product/capability | Device | E0/local path | `application-runtime` (E1) | Slint UI |
|---|---|---|---|---|
| Immutable Hugging Face Hub Llama artifacts + Candle + Safetensors | CPU | Supported through one statically dispatched Candle worker | Repository/revision selection, immutable commit resolution, load, direct completion, exact TinyLlama chat profile, cancellation, cleanup, unload, persistence, shutdown | Supported as the sole model flow |
| GGUF or another quantized format | Any | Unsupported | No selection/load path | No |
| Candle CUDA/Metal or another GPU device | GPU | Deferred | No | No |
| Hosted provider or peer | Remote | Not an E0 backend | Not implemented | No |

The product is CPU-only and deliberately single-model at E1. Candle is the sole local execution engine. The current artifact source is Hugging Face Hub, the current model format is Safetensors, and the execution device is CPU. These remain distinct facts rather than one backend/product enum.

`ModelSelection` contains a normalized Hugging Face repository and requested revision. Resolution pins it to an immutable Hub commit. `ResolvedModel` and `LoadedModel` derive application-owned engine, source, device, format, scalar, tokenizer vocabulary, and repository/commit evidence from the supported artifacts. Callers cannot assemble unsupported engine/source/format/device cross-products.

Direct completion is supported for every successfully loaded model. Chat support is intentionally exact: only `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`, with `</s>` resolving to token ID 2, receives the built-in role renderer and EOS policy. Every other loaded model remains direct-completion-only.

## Runtime and ownership state

`application-runtime` remains the public frontend-neutral, non-generic E1 façade. Private composition owns:

- one `HostedRuntime<CandleLlamaSource>` and inference worker thread;
- one bounded Hugging Face Hub worker thread;
- one resolved `HfTokenizer` and request-local `HfOwnedStreamingDecoder` values;
- one resident-model/application state machine;
- redb-backed preferences and model-catalogue state.

E0 exclusively owns loaded model resources, sequence state, request admission, generation workspaces, token scheduling and sampling, cancellation boundaries, output backpressure, cleanup quarantine, accounting, unload, and terminal shutdown. Production token-sensitive execution is statically dispatched. Deterministic loaders retain backend-independent fault coverage without becoming another production engine.

The lifecycle corrections now provide these guarantees:

- startup owns partially created inference state in a rollback guard, attempts bounded shutdown/join if later Hub startup fails, preserves the primary Hub error, and quarantines any unresolved rollback owner for a later bounded reap;
- a rejected incompatible model remains privately accounted with its handle, compatibility failure, and unload state through successful cleanup, proven runtime disconnection, or observable bounded retry exhaustion;
- shutdown distinguishes running, stopping, cleanly stopped, retryable failure, and terminal failure;
- inference and Hub join handles remain in their owners after a wait timeout, so a later `shutdown()` retries unresolved joins and may complete cleanly;
- E0 cleanup exhaustion is terminal: the worker reports the structured failure, retains the runtime allocation until process exit instead of invoking unverified implicit backend destruction, and can still be joined;
- E1 retains that terminal failure after the inference handle is joined, so later shutdown calls return the same failure rather than inferring success from absent handles;
- an endpoint disconnection without already observed clean completion does not independently establish clean shutdown.

## Phase 10 benchmark implementation

[ADR-0018](../agent/decisions/0018-benchmark-and-model-fixture-policy.md) now has one concrete crate-local sampling suite and exactly one system benchmark package. Production support and dependency direction remain unchanged.

| Package/surface | Role | Current evidence | Limitations |
|---|---|---|---|
| `sampling` / `benches/sampling_pipeline.rs` | Criterion component matrix over the public sampler: explicit `sample_only` versus `restore_and_sample`, greedy/default/min-p/repetition cases at 8K/32K/128K vocabularies, plus public stop matching | Focused package check/test/strict-Clippy passed; selected default-policy 32K intervals are recorded below | Component timing only; not E0/E1 throughput, allocation counting, native-resource evidence, or a CI threshold |
| `runtime-benchmarks` / normal `baseline` binary, synthetic mode | Bounded download-free E0 lifecycle/accounting/RSS runner plus matching fresh E1 start/shutdown cycles | Final corrected release run with one warmup and three recorded samples passed; all nine generation operations reached matching `Terminal` + `Released`, each cancellation emitted two tokens, unload had exact zero accounting, and shutdown/join was clean | Deterministic synthetic integration evidence; not product-model performance, language quality, representative steady state, or broad compatibility |
| `runtime-benchmarks` / Criterion `runtime` target | Hosted public-E0 checked-prefill and incremental-decode command/event boundaries | Both targets executed; selected intervals are recorded below | Not E1/product latency, full generation throughput, RSS, allocation counts, or native/device attribution |
| `runtime-benchmarks` / real-product mode | Pinned opt-in public-E1 path for `neubla/tiny-random-LlamaForCausalLM` commit `1c81a3fba044af78df253edc66bdbab183184932`; requires `--allow-network`, rejects `HF_HUB_OFFLINE=1`, and permits an existing cache only under shared root `target/` or outside the repository | Compiled | Not executed; no product baseline exists |
| `runtime-benchmarks` / report, metadata, and RSS support | Versioned stable serde JSON, Git/toolchain/target/host/workload metadata, typed lifecycle/accounting records, exact pre-setup fixture SHA-256 verification, and safe CLI/cache/fixture/toolchain/CPU/`/proc` paths | Focused package tests passed; 15 tests | Process-wide sampled RSS is not per-model ownership, allocation counting, native attribution, or device memory; output intentionally excludes tokens/text, broad environment dumps, and secrets |

`benchmarks/runtime` is the sole root benchmark package. It is a non-production outer consumer using the root workspace, lockfile, and target directory; it declares `publish = false`, has no build script, and has no incoming production dependency. Its external normal dependencies are exactly `serde`, `serde_json`, and `sha2` 0.11. The normal system runner and Criterion component target are intentionally separate and neither imposes timing thresholds.

Tokenizer encode/streaming-decode, context-planner, output-accumulator, and isolated Candle microbenchmarks were reviewed and deferred because no candidate had both a named unresolved question and a demonstrated insufficiency in the implemented public E0/system surfaces. No placeholder suite or benchmark-only copy of production logic was added.

The deterministic F32 Llama fixture remains project-authored Cargo/Rust-generated data with no external model or tokenizer assets. Exact sizes, SHA-256 hashes, licensing, tensor construction, and scope are recorded in its [provenance file](../../crates/runtime/inference-runtime/tests/fixtures/candle-llama/PROVENANCE.md). The E0 integration test remains its primary owner, existing `application-runtime` tests and the benchmark are additional in-place consumers, and the benchmark recomputes both hashes before runner or Criterion setup.

Phase 10 changed no production public API or production implementation and added no optimization, unsafe code, GPU, GGUF/quantized path, second engine, hosted-provider/peer/browser path, or multi-model product variant.

## Phase 9 structural closure (historical baseline)

The root `Cargo.toml` is now a virtual workspace. `tools/xtask` owns custom architecture/hygiene policy and the canonical composite `cargo xtask verify` command. One-step checking, testing, formatting, linting, documentation, and benchmarking use Cargo directly; forwarding subcommands were removed.

The domain policy is an exact reviewed DAG rather than an absolute F1-peer ban. Its current registered production edges are:

```text
tokenization    -> domain-contracts
context-planner -> domain-contracts
sampling        -> domain-contracts
task-graph      -> domain-contracts
```

Every domain edge requires an exact source/target/kind rationale, and the complete registry must remain acyclic. `TaskId` moved to its natural owner, `task-graph`. A type belongs in `domain-contracts` only when it crosses backend/runtime boundaries or has at least two stable, genuinely distinct domain consumers.

Responsibility-based internal splits now separate:

- E0 admission, execution, cleanup, accounting, inspection, unload, and shutdown;
- E1 generation settings, output, admission, and bridge/session behavior;
- task graph, artifact flow, task state, and graph errors;
- desktop callback, control, model, and output presentation;
- architecture policy/traversal/reporting and hygiene orchestration/manifest/invocation parsing.

The mandatory lint gate retains stable `clippy::all`, `clippy::pedantic`, and explicitly selected lints under `-D warnings`. The blanket nursery group is no longer inherited or mandatory; scheduled CI reports it separately without making nursery findings merge-blocking.

## Operational-tooling closure

Ubuntu CI retains `build-essential` for native compilation and the Slint font/XCB/XKB development packages. It no longer installs system CMake. The selected non-FIPS AWS-LC path is forced through its CC builder; the Rust `cmake` package may remain in the upstream locked dependency set without invoking a system CMake executable.

Required CI runs the canonical gate from a fresh target directory with fail-fast shims for CMake, Clang, Python runtimes/package tools, and Python-distributed Hugging Face commands. The temporary Candle cleanup execution brief was removed. Hygiene scans every tracked operational surface without filename-, directory-, history-, or ADR-status whole-file exemptions.

## Current validation evidence

Implementation began from clean `HEAD` `f61a0fadd2311a53e1bce55094f886e3465b0c95`, tree `1bee6fa25f8b4819ac68d02cc10324f0f1848e9e`. Only root `./target/` was present, and `cargo xtask verify` passed before edits. That is starting-tree evidence and does not validate the Phase 10 diff.

The following focused evidence passed after implementation:

| Command/evidence | Observed result |
|---|---|
| `cargo metadata --locked --format-version 1 --no-deps` | Passed with the new sole `runtime-benchmarks` package in locked workspace metadata |
| Required package-level `cargo check`, `cargo test`, and strict `cargo clippy` commands for `sampling` | Passed |
| Required package-level `cargo check`, `cargo test`, and strict `cargo clippy` commands for `runtime-benchmarks` | Passed; 15 tests passed |
| `cargo test --locked -p xtask` | Passed 32 total tests—14 unit and 18 integration tests; doc tests contained no cases—including the exact benchmark-package role/dependency policy |
| Final corrected release synthetic baseline, one warmup plus three recorded samples | Passed; all nine generation operations reached matching `Terminal` and `Released` with no pending/exhausted cleanup, each cancellation emitted two tokens, unload accounting was exactly zero, and shutdown/join was clean |

Selected Criterion estimate intervals from the controlled synthetic/component runs are:

| Target | Selected interval |
|---|---:|
| `e0_hosted_checked_prefill/4_tokens` | `[5.0955, 5.1104] ms` |
| `e0_hosted_incremental_decode/1_token_after_2_token_prefill` | `[5.0474, 5.0890] ms` |
| `sample_only/default_top_k_top_p/32768` | `[77.211, 77.228] µs` |
| `restore_and_sample/default_top_k_top_p/32768` | `[78.288, 78.302] µs` |

Exactly these four Criterion targets were statistically executed; all remaining sampling-matrix and stop-matching targets are compile-only evidence. These results are synthetic fixture/component evidence, not current product-model latency or throughput. The immutable real-product mode was compiled but not executed; future execution requires authorized network access and an allowed existing cache. External product evidence remains outstanding and no result is inferred.

Final repository acceptance passed `cargo bench --workspace --no-run --locked`, canonical `cargo xtask verify`, locked cargo-deny policy, offline link checking, `git diff --check`, and root-target/generated-artifact hygiene. The first canonical attempt exposed only an oversized `xtask` benchmark-policy test helper; it was split without weakening policy, focused strict Clippy passed, and the complete canonical rerun passed.

## Known limitations and next work

- CPU is the only supported execution device.
- E1 supports one selected/resident model.
- Chat compatibility is limited to the exact reviewed TinyLlama repository, immutable commit, and tokenizer/EOS evidence.
- Conversation history is in memory only; persistence and arbitrary branch trees are not implemented.
- Slint uses E1's default generation settings; no settings panel is exposed.
- The project-generated tiny Candle fixture proves integration/lifecycle behavior rather than language quality or product performance.
- The synthetic short post-first-token window is an integration proxy, not representative production steady-state throughput.
- Process RSS is sampled, process-wide host memory; it is not allocation counting, ownership proof, native-resource attribution, or device-memory accounting.
- Strict allocation-free Candle or Hugging Face tokenization/decoding is not claimed because upstream libraries allocate internally.
- GGUF/quantized loading is unsupported. Possible future work must be Candle-native and separately reviewed.
- GPU, hosted-provider, peer, remote/browser transport, multiple-model residency, and `application-api` are not implemented.
- The compiled Phase 10 real-product mode has not run, so there is no current-tree external product baseline.
- The prior external Hub smoke proves one historical pinned tiny-random-model correctness path; it does not establish broad model compatibility, current-tree network behavior, or Phase 10 product performance.
- Phase 10 repository acceptance is complete, but the opt-in external product baseline remains unmeasured.

Phase 10 is complete with focused and canonical validation plus controlled synthetic/component evidence. A future evidence run may exercise the pinned product mode only with explicit network authorization and an allowed existing cache. Phase 11 is not active.

## Historical context

The [recovered implementation plan](implementation-plan.md) is historical source material and is not authoritative. Completed Phase 8 plan text and [Phase 8 history](../agent/execution/history.md#phase-8--gguf-parity-and-native-composition-evidence) describe the former dual-product tree; they are not current support claims. The Candle-only correction checkpoint is recorded by prior commit `f0fe9c6…` and tree `db8a9ae…`; the subsequent Phase 9 structural closure is recorded separately in [execution history](../agent/execution/history.md).
