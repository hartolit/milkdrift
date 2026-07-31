# Current implementation status

**Status date:** 2026-08-01
**Prior committed checkpoint:** `f0fe9c6623f1e2afd569767d903f3978e00560da` (tree `db8a9ae77f41e0e769c7434ce21a940ae33784ae`)
**Current validation scope:** Phase 9 closure working tree based on that checkpoint; CI records the resulting commit and tree externally at runtime
**Toolchain observed:** Rust/Cargo 1.96.1; host `x86_64-unknown-linux-gnu`
**Execution position:** Phase 9 is closed; Phase 10 measurement work is next
**Validation state:** canonical, focused lifecycle, clean forbidden-tool, portability, dependency-policy, and local-link checks pass; the network-dependent external Hub smoke was not rerun for this structural closure
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
- shutdown distinguishes running, stopping, stopped, and failed/retryable outcomes;
- inference and Hub join handles remain in their owners after a wait timeout, so a later `shutdown()` retries unresolved joins;
- idempotent shutdown success means both workers were confirmed stopped, not merely that a prior attempt began.

## Phase 9 structural closure

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

The following completed successfully on the Phase 9 closure working tree based on the prior `f0fe9c6…` checkpoint:

| Command/evidence | Observed result |
|---|---|
| `cargo xtask verify` | Passed architecture, hygiene, formatting, all-target workspace checking, ordinary tests/doctests, mandatory Clippy, strict rustdoc, and benchmark compilation |
| Fresh-target `cargo xtask verify` with failing CMake/Clang/Python/package-manager/Hugging Face CLI shims and non-FIPS AWS-LC CC-builder selection | Passed; no shim was invoked |
| Focused `application-runtime` tests | Passed 37 unit tests, 3 integration tests, and doctests, including successful startup rollback, retained/reaped ownership after rollback timeout, pre-worker deadline-bound validation, incompatible-cleanup submission failure/exhaustion, and retryable shutdown joins |
| Focused `inference-runtime`, `task-graph`, `corrective-workflow`, `desktop-slint`, and `xtask` suites | Passed after responsibility-based source splits and `TaskId` ownership migration |
| Named portability checks for the five portable crates on `wasm32-unknown-unknown` and `thumbv7em-none-eabihf` | Passed |
| `cargo deny --workspace --locked check advisories bans licenses sources` | Passed; configured duplicate-version findings remain audit warnings |
| `lychee --config lychee.toml --offline '**/*.md'` | Passed with no local-link errors |
| Locked metadata, dependency-tree, lockfile, architecture, and hygiene audits | Passed; no removed engine or prohibited Python runtime/binding package is selected |
| `git diff --check` | Passed |

The opt-in Rust-native external Hub smoke last passed at the prior Candle cleanup checkpoint against `neubla/tiny-random-LlamaForCausalLM` commit `1c81a3fba044af78df253edc66bdbab183184932`. It was not rerun for this structural/lifecycle closure, so that older network result is not presented as exact-tree evidence. Ordinary validation remains download-free.

No manual graphical desktop acceptance session was performed.

## Known limitations and next work

- CPU is the only supported execution device.
- E1 supports one selected/resident model.
- Chat compatibility is limited to the exact reviewed TinyLlama repository, immutable commit, and tokenizer/EOS evidence.
- Conversation history is in memory only; persistence and arbitrary branch trees are not implemented.
- Slint uses E1's default generation settings; no settings panel is exposed.
- The tiny Candle fixture proves integration rather than language quality.
- Strict allocation-free Candle or Hugging Face tokenization/decoding is not claimed because upstream libraries allocate internally.
- GGUF/quantized loading is unsupported. Possible future work must be Candle-native and separately reviewed.
- GPU, hosted-provider, peer, remote/browser transport, multiple-model residency, and `application-api` are not implemented.
- The prior external Hub smoke proves one pinned tiny random model path; it does not establish broad model compatibility or current-tree network behavior.

Phase 10 may now begin with measured sampling, tokenizer, context, output, backend, and end-to-end Candle baselines. It must not treat the correctness smoke as a benchmark or optimize from unmeasured assumptions.

## Historical context

The [recovered implementation plan](implementation-plan.md) is historical source material and is not authoritative. Completed Phase 8 plan text and [Phase 8 history](../agent/execution/history.md#phase-8--gguf-parity-and-native-composition-evidence) describe the former dual-product tree; they are not current support claims. The Candle-only correction checkpoint is recorded by prior commit `f0fe9c6…` and tree `db8a9ae…`; the subsequent Phase 9 structural closure is recorded separately in [execution history](../agent/execution/history.md).
