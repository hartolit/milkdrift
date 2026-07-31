# Agent Brief: Remove llama.cpp, Python Tooling, and the Accidental Dual-Backend Architecture

## Repository

- Repository: `https://github.com/hartolit/llm-app`
- Inspected baseline: `d7d03e46c0239d4be8c34e8a5e16959fb5bd46c3` (`main`, commit message: `Big phase 8 vibes`)
- The repository may have moved since this brief was prepared. Start by recording `git rev-parse HEAD`, `git status --short`, and the current dependency graph. Adapt paths and line numbers to the actual tree; do not assume the inspected commit is still HEAD.

## Mission

Execute a complete architectural correction. Do not merely write a plan.

The project is returning to a **Candle-first, Rust-native local inference architecture**. The llama.cpp/GGUF product path was introduced because of an incorrect assumption that llama.cpp was required for CPU and GPU support. It is not a foundational project requirement, and the implementation has spread native/runtime duplication, C/C++ build tooling, GGUF-specific product concepts, and Python-based fixture generation through the repository.

Remove that detour cleanly while preserving the working Candle application path and the backend-neutral inference contracts.

The resulting repository must:

1. use Candle as the sole local execution engine;
2. contain no project-owned Python source, generator, build, test, validation, packaging, release, or runtime requirement;
3. require no Python CLI such as `python`, `python3`, `pip`, or the Python Hugging Face `hf` CLI for documented project workflows;
4. remove llama.cpp, `llama-cpp-2`, the current `gguf-backend`, and native tooling that exists only for them;
5. retain a clean architecture in which execution engine, model format, artifact source, and device are separate concepts;
6. preserve all useful Candle lifecycle, generation, chat, context, cancellation, backpressure, cleanup, unload, persistence, and shutdown behavior;
7. leave the repository fully validated, documented, and free of dead compatibility scaffolding.

Do not preserve accidental complexity merely because Phase 8 validated it.

## Authority and documentation conflict

Read the project documentation in the repository-prescribed order:

1. `docs/agent/persona.md`
2. `docs/README.md`
3. `docs/architecture.md`
4. `docs/rules.md`
5. `docs/project/architecture.md`
6. `docs/project/implementation-status.md`
7. `docs/agent/execution/current.md`
8. relevant ADRs and component guides

However, this user decision supersedes current documents that require the dual Candle/llama.cpp composition to remain intact.

In particular, the current tree contains:

- accepted ADR-0012 requiring two monomorphized local E0 workers and a closed Candle/llama.cpp product set;
- Phase 9 handoff invariants requiring the GGUF tokenizer, SHA-256 identity chain, two-worker shutdown, and two-product UI to be preserved;
- architecture and status documents that describe llama.cpp/GGUF as a supported product.

Those are now stale architectural instructions. Do not obey them as current constraints. Preserve them only as historical evidence where appropriate.

Create new ADRs using the next available numbers:

1. **Candle is the sole local execution engine.** Model formats and devices are orthogonal to the engine. This ADR must explicitly supersede ADR-0012 and affirm or update the intent of ADR-0002.
2. **Project-owned operational tooling is Rust/Cargo-native and must not require Python.** Define the precise boundary and enforcement mechanism.

Mark ADR-0012 as superseded and link it to the replacement. Do not delete accepted historical ADRs or rewrite Phase 8 history as though it never occurred.

## Architectural decision

### Current accidental shape

```text
application-runtime
├── HostedRuntime<CandleLlamaSource>
├── HostedRuntime<GgufSource>
├── active-backend routing
├── Candle/HF tokenizer dispatch
├── llama.cpp/GGUF tokenizer dispatch
├── dual worker lifecycle and shutdown
└── product cross-coupling

Safetensors -> Candle
GGUF        -> llama.cpp
```

This incorrectly ties a serialization format to an execution engine and forces E1 to maintain two local runtimes even though it supports one resident model.

### Required target shape

```text
apps/desktop-slint
        |
        v
application-runtime                 E1 application semantics
        |
        v
inference-runtime                   E0 lifecycle, scheduling, sampling,
        |                            cancellation, backpressure, cleanup
        v
candle-backend                      sole local execution adapter
        |
        +-- current: Safetensors + Hugging Face artifacts
        +-- future, separate work: Candle-native GGUF/quantized loading
        |
        +-- current device: CPU
        +-- future, separate work: Candle device/feature selection
```

The boundaries mean:

- **Candle** is the local execution engine.
- **Safetensors and GGUF** are model formats, not backends.
- **Hugging Face Hub or local file** are artifact sources, not execution engines.
- **CPU/CUDA/Metal/other devices** are execution-device choices, not reasons to introduce a second runtime architecture.

Do not implement Candle-native GGUF in this cleanup. Candle exposes GGUF and quantized loading primitives, but model-family compatibility, tokenizer provenance, quantization behavior, and device support require a separate reviewed implementation. First restore a small, coherent Candle/Safetensors baseline. Record Candle-native GGUF as future work under the Candle adapter rather than preserving `GGUF == llama.cpp`.

## Non-negotiable invariants to preserve

Preserve the behavior, not the accidental dual-backend implementation:

- E0 exclusively owns loaded model resources, sequences, request admission, generation workspaces, token scheduling, sampling, cancellation boundaries, cleanup quarantine, accounting, unload, and shutdown.
- Model ownership remains exclusive; do not introduce public `Arc<Model>` ownership.
- Token-sensitive execution remains statically dispatched. Do not replace the current architecture with trait objects or a plugin registry in the decode loop.
- `application-runtime` remains a frontend-neutral, non-generic E1 façade.
- Frontends do not construct Candle loaders, model sources, tokenizer implementations, native devices, or inference commands.
- Hugging Face artifact resolution remains immutable-revision aware and Rust-native.
- Direct completion remains functional.
- The verified TinyLlama chat profile, prompt rendering, EOS policy, conversation/context planning, regeneration/supersession, and bounded transcript behavior remain functional.
- Bounded command/event/output channels, cancellation, output backpressure, cleanup retry/exhaustion, unload policy, and explicit bounded shutdown remain functional.
- redb-backed application preferences/catalogue behavior remains functional unless a concrete bug requires a narrowly justified change.
- `corrective-workflow` remains an independent capability runtime and is not absorbed into E1.
- Portable domain crates and their target claims remain intact.
- Existing strict lint and unsafe-code policies remain intact.

## Required source and dependency cleanup

### Remove the llama.cpp implementation completely

Delete the entire current adapter:

```text
crates/adapters/gguf-backend/
```

Remove all production and development references to it, including:

- root workspace members and workspace dependencies;
- `application-runtime` production dependency;
- `inference-runtime` development dependency;
- architecture-validator reviewed edges and validator tests;
- imports, source enums, tokenizer/decoder enums, backend routing, configuration, error variants, state, summaries, tests, examples, and UI branches;
- documentation and current execution instructions.

Remove from the selected Cargo graph:

- `llama-cpp-2`;
- `llama-cpp-sys-2` and other llama.cpp transitive packages;
- `self_cell` if no remaining independently justified use exists;
- native build dependencies introduced only for llama.cpp.

Regenerate `Cargo.lock` through Cargo. Do not hand-edit it.

### Remove the Python fixture path

Delete:

```text
crates/runtime/inference-runtime/tests/fixtures/gguf-llama/
```

This includes the committed GGUF binary, its README, and `generate_gguf.py`.

Do not port the 400-line GGUF converter to Rust merely to preserve a product path that is being removed. Preserve the useful runtime behavior tests using the Candle fixture and deterministic test backends.

### Simplify E1 concrete composition

`crates/runtime/application-runtime/src/local.rs` currently starts two hosted E0 workers and routes through an active-backend switch.

Replace that with one Candle endpoint and one inference worker. Then reassess whether `local.rs` still owns a coherent boundary:

- keep a small private module if it meaningfully isolates Candle source/tokenizer/worker composition from application semantics;
- otherwise inline the remaining trivial wrapper into the appropriate E1 module;
- do not retain closed dispatch enums, `active` state, endpoint-availability maps, duplicate shutdown state, or placeholder backend variants for hypothetical future implementations.

The result should have one source type, one runtime handle, one runtime thread, one tokenizer implementation, and one streaming decoder implementation for local execution.

### Simplify the public application vocabulary

Remove concepts that exist only because of the accidental second product, including the equivalent current forms of:

- `LocalModelProduct::LocalLlamaCppGguf`;
- `ApplicationBackend::LlamaCpp`;
- `ModelSelection::LocalGguf`;
- `ApplicationGgufConfiguration` and its validation fields;
- GGUF-only scalar/quantization compatibility variants;
- GGUF SHA-256 identity variants;
- local GGUF path handling;
- source/backend/format cross-product routing introduced for the second backend.

Reassess all single-variant enums and wrappers after removal. Keep a type only when it expresses a current invariant, stabilizes a real public boundary, or has multiple real consumers. Do not keep speculative variants solely to make a future backend look easy to add.

A clean current selection may be a simple application-owned structure containing Hugging Face repository and revision. Resolved/loaded state may still report engine, source, format, scalar type, device, and immutable identity when those values provide real frontend or validation evidence; they must derive from the actual resolved product rather than expose arbitrary unsupported combinations.

Do not expose Candle types in the public E1 API.

### Simplify the desktop UI

Remove the two-product selector and every GGUF-specific control from Slint and the presenter:

- product index;
- llama.cpp label;
- GGUF path property and input;
- selection branches;
- GGUF summaries and formatting;
- related generated-binding tests.

Keep the Hugging Face repository/revision flow and current chat/direct-completion presentation behavior.

### Preserve backend contract coverage

Do not respond to the removal by deleting valuable tests.

The current shared native-backend suite proves load, generation, deterministic sampling, EOS, token limits, output backpressure, cancellation, cleanup, unload, shutdown, and worker join. Preserve those behaviors through:

- a Candle real-fixture integration test; and
- deterministic E0 test loaders/backends where backend-independent behavior is better tested without a native implementation.

Remove a generic test trait that only has one implementation if it no longer improves clarity. Keep generic helpers when they genuinely express the E0 substitution contract rather than serving as a monument to the deleted backend.

Preserve E1 integration tests for resolution, load, direct completion, verified chat, cancellation, backpressure, unload, worker disconnection, persistence, and shutdown. Delete or rewrite only assertions whose subject was specifically llama.cpp/GGUF.

## Python and external-tooling removal

### Exact policy boundary

The repository itself must not contain or require Python for any maintained workflow.

Remove project-owned:

- `.py` files;
- notebooks;
- `pyproject.toml`, `requirements*.txt`, `Pipfile*`, Poetry files, or equivalent Python environment definitions;
- Python shebangs and Python subprocess invocation;
- `python`, `python3`, `pip`, `pipx`, `uv`, `conda`, `poetry`, `pytest`, or `maturin` commands in source, build scripts, workflows, maintained validation docs, examples, and release instructions;
- PyO3 or other embedded-Python/runtime bindings from the selected Cargo graph;
- Python-based Hugging Face download/conversion procedures.

Do **not** reject a Rust crate merely because its upstream repository contains optional Python examples or bindings that Cargo neither builds nor invokes. The enforceable boundary is the selected project build/runtime/tooling path, not the language composition of every upstream monorepo.

### Replace the documented Hugging Face CLI smoke flow

`docs/project/validation.md` currently tells the developer to install the `hf` CLI, run `hf download`, and use external SHA tooling before the Candle smoke.

Replace this with a Rust-native validation path using the existing `hf-hub-adapter` and the exact pinned immutable revision.

Choose the layer according to ownership:

- E0 must not acquire Hugging Face/network responsibilities.
- A Rust example or focused integration executable in `application-runtime`, or an appropriately layered Rust maintenance command, may resolve the model through `hf-hub-adapter` and then exercise E1/Candle.
- Reuse production resolution and lifecycle code rather than implementing a second downloader.
- Do not add a shell or Python wrapper around the Rust path.

The documented smoke should be invokable with Cargo and should require no manual model conversion. Authentication/cache overrides may remain environment-based through the existing Rust adapter.

Use an established pure-Rust digest crate only if exact byte-digest verification remains necessary. Do not retain or relocate a large handwritten SHA-256 implementation without a demonstrated production requirement.

### Add regression prevention

Add a Rust-owned repository hygiene check, preferably integrated into the architecture validator or the planned `xtask`, that fails on newly tracked operational Python artifacts or maintained Python invocations.

The check should cover at least:

- project-owned Python source/config/notebook extensions and conventional filenames;
- workflows and maintained operational docs invoking Python package/runtime commands;
- Cargo manifests selecting Python bindings;
- reintroduction of the removed llama.cpp packages.

Allow narrow, explicit exclusions for historical text in superseded ADRs or execution history when that text is necessary to explain an old decision. Historical mention is not an operational dependency.

Where supported cleanly by the current `cargo-deny` version, add explicit bans for the removed llama.cpp crates and direct Python-binding crates such as PyO3. Do not add broad bans that reject unrelated valid Rust dependencies or break the selected graph without evidence.

## Native CI toolchain cleanup

The current Linux CI installs `cmake`, `clang`, and `libclang-dev` alongside UI/system packages. After removing llama.cpp:

1. determine which packages are still required by the selected workspace build;
2. remove `cmake`, `clang`, `libclang-dev`, and `build-essential` individually when a clean CI-equivalent build proves they are unnecessary;
3. keep only system libraries genuinely required by Slint/windowing/font integration or another selected dependency;
4. document any remaining non-Rust prerequisite and its owner.

Do not claim a Rust-native repository by hiding required native tools. Prove the minimized list through a clean build.

## Documentation work

Update every current source of truth affected by the decision. At minimum inspect and correct:

- root `README.md`;
- `docs/project/architecture.md`;
- `docs/project/implementation-status.md`;
- `docs/project/workspace.md`;
- `docs/project/dependency-policy.md`;
- `docs/project/application-runtime.md`;
- `docs/project/inference-runtime.md`;
- `docs/project/desktop-runtime.md`;
- `docs/project/validation.md`;
- `docs/project/README.md`;
- `crates/adapters/README.md`;
- affected crate READMEs;
- `docs/agent/execution/current.md`;
- active execution-plan work packages and acceptance criteria;
- ADR index and the new superseding ADRs;
- execution history/status evidence after validation.

Rules:

- Current architecture/status/validation documents must not describe llama.cpp or the deleted GGUF product as supported.
- Mark ADR-0012 superseded rather than deleting it.
- Keep Phase 8 history factual and clearly historical.
- Do not mechanically erase every occurrence of `GGUF`; future architecture may state that Candle-native GGUF is deferred. It must never imply current support.
- Do not duplicate the same rationale across many documents. Put decision rationale in ADRs, current structure in architecture/workspace docs, current support in implementation status, commands in validation, and completed evidence in history.

## Explicit non-goals

- no Candle-native GGUF implementation in this cleanup;
- no new local inference engine;
- no GPU implementation or feature matrix yet;
- no hosted-provider, peer, browser, or network transport implementation;
- no public plugin registry;
- no dynamic dispatch in token-sensitive execution;
- no generic `ApplicationRuntime<A, B, C, ...>` façade;
- no speculative `application-api` crate;
- no broad rewrite of E0 lifecycle semantics;
- no removal of useful tests merely to make the diff smaller;
- no weakening Clippy, rustdoc, architecture, unsafe-code, capacity, cleanup, or dependency-policy gates;
- no `allow(dead_code)`, placeholder variants, empty compatibility modules, or TODO-only scaffolding to make compilation pass;
- no blind revert of the whole Phase 8 commit. Preserve unrelated valid work and remove the detour surgically.

## Execution sequence

1. Record exact baseline and dirty state.
2. Read the repository-prescribed docs and inspect all current llama.cpp/GGUF/Python references.
3. Write the superseding ADRs before allowing stale Phase 9 invariants to steer implementation.
4. Remove the GGUF/llama.cpp crate, fixture, manifests, lockfile graph, and validator edges.
5. Simplify E1 composition, public types, configuration, state, and shutdown to one Candle path.
6. Simplify Slint and presenter code.
7. Preserve/restructure Candle and backend-independent tests.
8. Replace Python/HF CLI validation with a Rust-native Cargo path.
9. Add regression enforcement for Python and removed native dependencies.
10. Minimize CI native packages based on clean-build evidence.
11. Update current documentation and historical/superseded records correctly.
12. Run focused checks, then the complete canonical gate on the exact resulting tree.
13. Review the final diff for orphaned concepts, accidental API scars, stale docs, and unjustified dependencies.

Do not stop after compilation. The task is complete only when architecture, code, tests, tooling, CI, lockfile, and documentation agree.

## Required audits

Run repository-wide searches after implementation. Use exact/current equivalents of:

```sh
git ls-files | rg '(\.py$|\.pyi$|\.ipynb$|(^|/)(pyproject\.toml|requirements[^/]*\.txt|Pipfile(\.lock)?|poetry\.lock)$)'
rg -n --hidden --glob '!target/**' --glob '!.git/**' \
  '(python3?|pipx?|pytest|maturin|pyo3|llama-cpp|llama\.cpp|gguf-backend|LocalGguf|LlamaCpp|hf download)'

cargo tree --workspace --locked | rg \
  '(llama-cpp|llama_cpp|pyo3|pythonize|rustpython)'
```

Interpret results rather than blindly requiring zero textual matches:

- active source, manifests, workflows, current docs, and commands must have none;
- superseded ADR/history references may remain when clearly historical;
- a future note may mention Candle-native GGUF, but current support must remain explicit;
- the selected Cargo graph must contain none of the removed llama.cpp or Python-runtime packages.

Also inspect:

```sh
cargo metadata --locked --format-version 1
cargo tree -d --locked
cargo tree -e features --locked
```

Confirm there are no orphan workspace members, stale feature flags, unused dependencies, or duplicate packages retained solely by the deleted path.

## Validation gates

Run focused checks while editing, then run all applicable repository gates on the exact final tree.

The current baseline canonical command is:

```sh
cargo run --locked --bin llm-app -- verify
```

If the same change set completes the planned xtask migration and updates the canonical command consistently, use the repository's resulting documented xtask command instead. Do not maintain two competing canonical gates.

Also run and report:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked
cargo bench --workspace --no-run --locked

git diff --check
cargo tree -d --locked
```

Run the repository's named portability checks for `domain-contracts`, `tokenization`, `context-planner`, `sampling`, and `task-graph` on both `wasm32-unknown-unknown` and `thumbv7em-none-eabihf`.

Run dependency and documentation policy gates:

```sh
cargo deny --workspace --locked check advisories bans licenses sources
lychee --config lychee.toml --offline '**/*.md'
```

Run the Rust-native external Candle smoke when network/model access is available. If unavailable, report that limitation precisely and do not claim external-model validation. Fixture-based and ordinary tests must still be download-free.

Where practical, run a clean build with Python command shims that fail immediately, proving no selected build/test path attempts to invoke Python. Do not confuse GitHub Actions' own implementation details with project requirements; the project command itself must succeed without Python.

## Acceptance criteria

The cleanup is accepted only when all of the following are true:

### Architecture

- Candle is the sole local execution engine.
- E0 remains backend-neutral in its contracts and owns local inference lifecycle semantics.
- E1 owns one local Candle composition and no longer starts or shuts down an unused second worker.
- Engine, format, source, and device are no longer bundled into the false `Candle/Safetensors` versus `llama.cpp/GGUF` axis.
- The public API contains no dead second-backend scaffolding.

### Dependency and tooling graph

- `crates/adapters/gguf-backend` is gone.
- `llama-cpp-2`, its sys crate, and llama.cpp are absent from `Cargo.lock` and `cargo tree`.
- `self_cell` is absent unless a remaining independent use is documented and tested.
- no project-owned Python files or environment definitions remain;
- no maintained command requires Python, pip, the Python HF CLI, or a model-conversion script;
- CI no longer installs native compiler/build packages that were required only by llama.cpp;
- the lockfile is regenerated and clean.

### Behavior

- Candle model resolution, load, generation, direct completion, verified chat, streaming decode, cancellation, backpressure, cleanup, unload, persistence, and shutdown remain covered and passing.
- backend-independent E0 behavior remains covered after the shared suite is simplified.
- the desktop frontend builds and exposes the single coherent Candle/Hugging Face flow.

### Documentation and evidence

- new ADRs record and enforce the decisions;
- ADR-0012 is marked superseded and retained historically;
- current architecture/status/validation documents describe the resulting reality;
- Phase 8 history remains factual rather than being erased;
- exact validation commands and results are recorded against the final revision/working tree.

## Final response required from the implementing agent

Return a concise but complete implementation report containing:

1. baseline commit and whether the starting/final trees were dirty;
2. architectural before/after summary;
3. deleted crates, fixtures, files, public concepts, and dependencies;
4. key simplifications in E0, E1, Slint, tests, validation, CI, and docs;
5. proof that Python and llama.cpp are absent from maintained workflows and the selected Cargo graph;
6. exact validation commands and their results;
7. any command not run and the precise reason;
8. remaining risks or deliberately deferred work, especially Candle-native GGUF and GPU support;
9. `git diff --stat` and a list of materially changed files.

Do not claim success based only on static inspection. Do not call the result pristine unless every reported gate actually passed on the exact final tree.

If repository write access is unavailable, produce one binary-safe patch against the recorded baseline and include these exact user-facing application commands, adjusted to the patch filename:

```sh
git apply --check --binary ~/Downloads/<patch-name>.patch
git apply --binary ~/Downloads/<patch-name>.patch
```
