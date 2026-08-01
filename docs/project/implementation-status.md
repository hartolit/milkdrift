# Current implementation status

**Status date:** 2026-08-01
**Prior committed checkpoint:** `3942a19b97d347fd238c451d2b0a2fcbea287873` (tree `be069879fea9531799038c5189c9edb3007ebf72`)
**Current validation scope:** pre-Phase 10 lifecycle, benchmark-policy, hygiene, and synthetic-fixture closure based on that clean checkpoint
**Toolchain observed:** Rust/Cargo 1.96.1; host `x86_64-unknown-linux-gnu`
**Execution position:** pre-Phase 10 closure is complete; Phase 10 implementation is next and has not started
**Validation state:** the starting tree passed the canonical gate; focused E0/E1 and `xtask` policy suites pass on the closure working tree; final command evidence is reported against the exact resulting diff rather than predicted here
**Canonical plan:** [LLM App Execution Plan](../agent/execution/execution-plan.md)
**Current working context:** [post-Phase 9 execution context](../agent/execution/current.md)

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

## Pre-Phase 10 architecture closure

[ADR-0018](../agent/decisions/0018-benchmark-and-model-fixture-policy.md) establishes measurement ownership without adding a Phase 10 package or suite:

- one crate-owned operation is measured only by a real benchmark in that crate's conventional `benches/` directory;
- future cross-crate E0/E1 and product measurements live only in `benchmarks/runtime` (`runtime-benchmarks`) as a non-production outer consumer of exact reviewed public APIs;
- future benchmark packages use the root workspace, lockfile, and target directory, declare `publish = false`, have no build script, and cannot receive production dependencies;
- tracked nested targets, benchmark lockfiles/build scripts/generated result trees, and model caches fail hygiene checks;
- raw Criterion/profiler data remains under root `target`, while curated summaries belong in canonical performance documentation.

The prior committed Candle fixture had technically synthetic structure but insufficient recorded authorship and redistribution provenance. It was replaced by a newly generated deterministic F32 Llama fixture produced by project-owned Cargo/Rust tooling without external model or tokenizer assets. Exact old/new sizes, SHA-256 hashes, licensing, tensor construction, and scope are recorded in its [provenance file](../../crates/runtime/inference-runtime/tests/fixtures/candle-llama/PROVENANCE.md).

No Phase 10 performance result has been recorded.

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

The clean starting checkpoint `3942a19…` matched freshly fetched `origin/main`, had tree `be06987…`, and passed `cargo xtask verify` before editing. The following focused closure checks then passed on the working tree:

| Command/evidence | Observed result |
|---|---|
| `cargo test --locked -p inference-runtime` | Passed 52 tests/doctests, including deterministic ordinary-unload zero accounting |
| `cargo test --locked -p application-runtime` | Passed 42 tests/doctests, including retryable joins, sticky terminal cleanup failure, model-drop suppression, and endpoint abandonment/disconnection |
| `cargo test --locked -p xtask` | Passed 32 unit/integration/doc tests covering benchmark role, reverse edges, package properties, recursive target ignore, root membership, and generated/cache artifact rejection |
| Explicit Cargo-native fixture generation, repeated twice | Both runs succeeded and produced identical 360-byte config and 4,800-byte Safetensors hashes recorded in the fixture provenance document |
| `cargo test --locked -p inference-runtime --test native_backend_generation` | Passed both download-free real-adapter lifecycle scenarios against the replacement synthetic fixture |

The final canonical, policy, link, formatting, and focused Clippy commands remain acceptance requirements in [validation](validation.md); their exact outcomes belong in the execution report or CI log for the resulting diff. This document does not predict a commit or Git tree containing itself.

The opt-in Rust-native external Hub smoke was not rerun. It is a network/cache-dependent correctness check, not Phase 10 performance evidence. No manual graphical desktop acceptance session was performed.

## Known limitations and next work

- CPU is the only supported execution device.
- E1 supports one selected/resident model.
- Chat compatibility is limited to the exact reviewed TinyLlama repository, immutable commit, and tokenizer/EOS evidence.
- Conversation history is in memory only; persistence and arbitrary branch trees are not implemented.
- Slint uses E1's default generation settings; no settings panel is exposed.
- The project-generated tiny Candle fixture proves integration/lifecycle behavior rather than language quality or performance.
- Strict allocation-free Candle or Hugging Face tokenization/decoding is not claimed because upstream libraries allocate internally.
- GGUF/quantized loading is unsupported. Possible future work must be Candle-native and separately reviewed.
- GPU, hosted-provider, peer, remote/browser transport, multiple-model residency, and `application-api` are not implemented.
- The prior external Hub smoke proves one pinned tiny random model path; it does not establish broad model compatibility or current-tree network behavior.

Phase 10 is the next operation but has not started. Its mandatory scope is the existing sampling-benchmark expansion, one `benchmarks/runtime` system harness, reproducible environment metadata, and controlled lifecycle/memory measurements. Tokenizer, context-planner, output-accumulator, and isolated Candle microbenchmarks are conditional on a named question and a documented reason the system harness is insufficient.

## Historical context

The [recovered implementation plan](implementation-plan.md) is historical source material and is not authoritative. Completed Phase 8 plan text and [Phase 8 history](../agent/execution/history.md#phase-8--gguf-parity-and-native-composition-evidence) describe the former dual-product tree; they are not current support claims. The Candle-only correction checkpoint is recorded by prior commit `f0fe9c6…` and tree `db8a9ae…`; the subsequent Phase 9 structural closure is recorded separately in [execution history](../agent/execution/history.md).
