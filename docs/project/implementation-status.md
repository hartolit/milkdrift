# Current implementation status

**Status date:** 2026-07-31
**Reviewed source baseline:** `15d9e87cdaee77fd0d49247712d3c12dfb3adea2` plus the current uncommitted Candle-only cleanup working tree
**Toolchain observed:** Rust/Cargo 1.96.1; host `x86_64-unknown-linux-gnu`
**Execution position:** Phase 9 Candle-only architectural correction is complete; evidence-driven structural review is next
**Validation state:** the canonical full locked gate, supplemental policy/portability/audit gates, a clean shimmed build, and the Rust-native external Hub smoke passed on the current working tree
**Canonical plan:** [LLM App Execution Plan](../agent/execution/execution-plan.md)
**Current working context:** [Phase 9 execution context](../agent/execution/current.md)

This is the canonical product-level status page. Component behavior belongs in the corresponding project guide, accepted rationale belongs in [architecture decisions](../agent/decisions/README.md), repeatable commands belong in [validation](validation.md), and historical phase evidence belongs in [execution history](../agent/execution/history.md).

## Supported devices and products

| Product/capability | Device | E0/local path | `application-runtime` (E1) | Slint UI |
|---|---|---|---|---|
| Immutable Hugging Face Hub Llama artifacts + Candle + Safetensors | CPU | Supported through one statically dispatched Candle worker | Repository/revision selection, immutable commit resolution, load, direct completion, exact TinyLlama chat profile, cancellation, cleanup, unload, persistence, shutdown | Supported as the sole model flow |
| GGUF or another quantized format | Any | Unsupported | No selection/load path | No |
| Candle CUDA/Metal or another GPU device | GPU | Deferred | No | No |
| Hosted provider or peer | Remote | Not an E0 backend | Not implemented | No |

The product is CPU-only and deliberately single-model at E1: one selected model may be resident. Candle is the sole local execution engine. The current artifact source is Hugging Face Hub, the current model format is Safetensors, and the execution device is CPU. These are distinct facts rather than one backend/product enum.

`ModelSelection` contains a normalized Hugging Face repository and requested revision. Resolution pins the selection to an immutable Hub commit. `ResolvedModel` and `LoadedModel` derive and expose application-owned engine, source, device, format, scalar, tokenizer vocabulary, and repository/commit evidence from the actual supported artifacts. Callers cannot assemble unsupported engine/source/format/device cross-products.

Direct completion is supported for every successfully loaded model. Chat support is intentionally exact: only `TinyLlama/TinyLlama-1.1B-Chat-v1.0` at immutable commit `fe8a4ea1ffedaf415f4da2f062534de366a451e6`, with `</s>` resolving to token ID 2, receives the built-in role renderer and EOS policy. Every other loaded model remains direct-completion-only; E1 does not infer a chat template.

## Current runtime and composition boundaries

`application-runtime` remains the public frontend-neutral, non-generic E1 façade. It owns application selection, immutable resolution, redb-backed preferences/catalogue state, one model lifecycle, direct completion, compatible conversation semantics, bounded decoded output, cancellation, unload policy, and explicit shutdown. No `application-api` crate exists.

Concrete local composition is private:

- one `HostedRuntime<CandleLlamaSource>`;
- one inference worker thread;
- one bounded Hugging Face Hub worker thread;
- one resolved `HfTokenizer` and request-local `HfOwnedStreamingDecoder` values;
- one resident-model/application state machine;
- no active-backend routing, dormant worker, dispatch enum, local-file selection, or placeholder product variant.

E0 exclusively owns loaded model resources, sequence state, request admission, generation workspaces, token scheduling and sampling, cancellation boundaries, output backpressure, cleanup quarantine, accounting, unload, and terminal shutdown. Production token-sensitive execution is statically dispatched. Deterministic test loaders retain backend-independent fault and lifecycle coverage without becoming another production engine.

`corrective-workflow` remains an independent capability runtime rather than an E1 subsystem. `desktop-slint` depends on E1 only and does not construct adapter sources, Hub clients, tokenizers, devices, or inference commands.

[ADR-0013](../agent/decisions/0013-candle-only-local-execution.md) supersedes the former Phase 8 two-worker/two-product composition. [ADR-0014](../agent/decisions/0014-rust-cargo-native-operational-tooling.md) governs maintained operational tooling.

## Preserved application behavior

The current source retains:

- immutable Hub revision resolution and complete repository/revision matching before load;
- F32, F16, and BF16 source scalar reporting/validation for the supported Candle path;
- direct completion with bounded prompt/output storage;
- exact TinyLlama role rendering and EOS compatibility;
- raw conversation provenance, regeneration/supersession, turn-atomic context planning, bounded exact-token correction, and in-memory history;
- generation cancellation at safe E0 boundaries;
- output backpressure without token loss or duplicate publication;
- distinct terminal and resource-release states;
- cleanup retry, quarantine, exhaustion, and retained accounting;
- reject/cancel/drain unload behavior;
- redb preferences and model-catalogue persistence;
- explicit bounded shutdown and joins for the sole inference worker and Hub worker.

The committed Candle fixture and deterministic loaders keep ordinary validation download-free. The Rust-native external Hub smoke is separate and opt-in.

## Validation evidence reported for this cleanup

The following commands completed successfully on 2026-07-31 against the current Candle-only cleanup worktree based on `15d9e87cdaee77fd0d49247712d3c12dfb3adea2`:

| Command | Observed result |
|---|---|
| `cargo run --locked --bin llm-app -- verify` | Passed; architecture, hygiene, formatting, all-target check, complete ordinary tests/doctests, strict Clippy, strict rustdoc, and benchmark compilation |
| `cargo test --locked -p inference-runtime --test native_backend_generation` | Passed; 2 Candle real-fixture E0 tests |
| `cargo test --locked -p application-runtime` | Passed; 31 unit tests, 3 state integration tests, and doctests |
| `cargo test --locked -p desktop-slint` | Passed; 19 presenter tests, binary target, and doctests |
| Named portability checks for the five portable crates on `wasm32-unknown-unknown` and `thumbv7em-none-eabihf` | Passed |
| `cargo deny --workspace --locked check advisories bans licenses sources` | Passed; configured duplicate-version findings remained warnings |
| `lychee --config lychee.toml --offline '**/*.md'` | Passed; 200 links valid, 0 errors, 10 configured exclusions |
| Locked metadata, duplicate-tree, feature-tree, lockfile, and forbidden-package audits | Passed; no removed engine, Python runtime/binding, or `self_cell` package in the selected graph |
| Fresh-target all-target check and test compilation with failing Python/Hugging Face CLI and `clang`/`clang++` shims | Passed; no prohibited command was invoked |
| `LLM_APP_CANDLE_HUB_SMOKE=1 cargo run --locked -p application-runtime --example candle_hub_smoke` | Passed against `neubla/tiny-random-LlamaForCausalLM` commit `1c81a3fba044af78df253edc66bdbab183184932`; resolved immutable artifacts, loaded Candle/F32 on CPU, generated 8 tokens, observed terminal/released state, unloaded, shut down, and removed temporary state |
| `git diff --check` | Passed |

The first external-smoke attempt used the older historical Phase 4 revision and failed correctly because that commit lacks the `tokenizer.json` required by the production E1 path. The maintained smoke was repinned to the repository's immutable `1c81a3f...` commit, whose complete artifact set then passed. This is integration and lifecycle evidence, not a language-quality claim.

No manual graphical desktop acceptance session was performed. CI prerequisite minimization was not reproduced inside a fresh Ubuntu 24.04 package image; local selected-graph inspection plus a fresh target build with failing Clang shims proved that Clang is not invoked, while the build did exercise retained CMake/native owners.

## Known limitations and deferred work

- CPU is the only supported execution device.
- E1 supports one selected/resident model.
- Chat compatibility is limited to the exact reviewed TinyLlama repository, immutable commit, and tokenizer/EOS evidence.
- Conversation history is in memory only; persistence and arbitrary branch trees are not implemented.
- Slint uses E1's default generation settings; no settings panel is exposed.
- The tiny Candle fixture proves integration rather than language quality.
- Strict allocation-free Candle or Hugging Face tokenization/decoding is not claimed because upstream libraries allocate internally.
- GGUF/quantized loading is unsupported. Possible future work must be Candle-native and separately reviewed for model compatibility, tokenizer provenance, immutable identity, quantization, lifecycle, and devices.
- GPU, hosted-provider, peer, remote/browser transport, multiple-model residency, and `application-api` are not implemented.
- The external Hub smoke proves one pinned tiny random model path; it does not establish broad Hub availability, model-family compatibility, language quality, or graphical desktop behavior.

## Historical context

The [recovered implementation plan](implementation-plan.md) is retained as historical source material and is not authoritative. Completed Phase 8 plan text and [Phase 8 history](../agent/execution/history.md#phase-8--gguf-parity-and-native-composition-evidence) accurately describe the former dual-product tree; they are not current support claims. The active roadmap is the [execution plan](../agent/execution/execution-plan.md), and the current working set is [current execution context](../agent/execution/current.md).
