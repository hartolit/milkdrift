# LLM App Execution Plan

**Intended repository path:** `docs/agent/execution/execution-plan.md`
**Companion analysis:** `docs/agent/execution/analyzer.md`
**Plan status:** Active execution baseline; pre-Phase 10 closure complete and Phase 10 next
**Prepared:** 2026-07-22
**Current amendment:** 2026-08-01

> **Current-use notice.** Phases 0–8 below are preserved as the specifications that produced their historical results. In particular, completed Phase 8 accurately records the former dual-product experiment; it does not define current support. [ADR-0013](../decisions/0013-candle-only-local-execution.md) through [ADR-0017](../decisions/0017-stable-clippy-gate-exploratory-nursery.md) govern the completed Candle-only Phase 9 architecture. The amended [ADR-0006](../decisions/0006-explicit-bounded-shutdown.md) and [ADR-0018](../decisions/0018-benchmark-and-model-fixture-policy.md) govern the completed pre-Phase 10 lifecycle, benchmark-layout, artifact, and fixture closure. Phase 10 has not started; it is the next implementation phase.

## 1. Purpose

This plan turns the findings in `analyzer.md` into an ordered implementation program. Its primary goal is to get a real product loop running without discarding the strong ownership, capacity, lifecycle, and portability work already present.

The plan deliberately starts with documentation. The repository currently uses its documentation as architectural instruction for both humans and agents, but several documents contain stale status, broken links, accidental content, and performance claims stated as universal rules. Any agent that consumes those documents before they are corrected can faithfully implement the wrong thing. Documentation repair is therefore a dependency of the engineering work, not a cleanup task left for the end.

The execution sequence is:

```text
truthful documentation
    -> reproducible quality gates
    -> transactional runtime safety
    -> backend-independent generation kernel
    -> Candle CPU vertical slice
    -> application-runtime generation façade
    -> usable Slint interface
    -> chat/context integration
    -> historical Phase 8 GGUF parity (later superseded)
    -> architectural simplification
    -> system benchmarks
    -> GPU support
```

The central milestone is not another isolated subsystem. It is this complete path:

```text
user input
  -> prompt preparation
  -> tokenization
  -> context admission
  -> sequence creation
  -> prefill
  -> sampling
  -> incremental decode
  -> bounded streaming output
  -> cancellation or completion
  -> deterministic sequence cleanup
  -> deterministic model unload
```

## 2. Current architectural position

The current repository has a strong low-level foundation:

- loaded models are exclusively owned rather than shared through `Arc<Model>`;
- backend operations use static dispatch in token-sensitive paths;
- model, sequence, request, drain, cancellation, and unload state are explicit;
- F0/F1 code is generally portable and `no_std`;
- capacities and caller-owned buffers are represented explicitly;
- worker channels are bounded;
- native/vendor dependencies are mostly quarantined in adapters;
- Slint is a thin frontend;
- package-local tests and Criterion benchmarks follow Cargo conventions.

The integrated prompt-to-stream and exact TinyLlama conversation loops now exist through E0, E1, and Slint. Phase 8 historically proved a second local product, but that experiment bundled engine, format, source, and device into an accidental product axis and duplicated E1 worker/tooling ownership. ADR-0013 supersedes that shape: Candle is the sole local engine, with immutable Hugging Face Hub Safetensors on CPU as the current composition.

The architecture closure also extracted `corrective-workflow`, adopted the `domain`/`platform`/`adapters`/`runtime`/`apps` physical taxonomy, and made runtime/platform roles and runtime composition edges fail closed. Phase 9 restored a small Candle baseline, hardened lifecycle ownership, adopted an exact domain DAG, split mixed-responsibility internals, moved custom maintenance policy to `tools/xtask`, separated mandatory and exploratory lints, and reconciled current documentation. That phase is closed.

## 3. Decisions this plan makes

These decisions remove ambiguity for implementation agents. They remain revisable through an ADR when evidence contradicts them.

### 3.1 Keep `application-runtime` as the frontend-facing façade

Its purpose is valid: Slint, Tauri, a CLI, and a native Leptos host should reuse the same application commands, state, persistence, model lifecycle, and normalized events.

The plan does **not** remove that façade. It narrows and strengthens it.

### 3.2 Do not make the façade generic over every service

Do not turn `ApplicationRuntime` into a public type with many storage, resolver, tokenizer, backend, clock, and transport type parameters. Cold-path replacement points may use coarse trait objects or closed enums. Hot inference paths remain statically dispatched.

### 3.3 Run sampling next to model execution

The inference worker should own the high-frequency prefill/decode/sample scheduler. The UI and `application-runtime` must not submit one command per generated token. Per-token command/event round trips would tie throughput to frontend polling and create avoidable channel churn.

Recommended ownership:

- `application-runtime`: prompt preparation, tokenizer ownership, context selection, public generation state, text decoding, frontend-facing output;
- `inference-runtime`: model/sequence ownership, prefill, logits, sampling, stop-token matching, bounded scheduling, cancellation boundaries, request cleanup;
- `host-runtime`: bounded command transport and pull-oriented token/text accumulators;
- frontend: frame-aligned pulls and presentation only.

### 3.4 Prove completion mode before general chat templating

The first real model slice may use an explicitly labelled direct-completion prompt. It must not pretend to be a model-independent chat template.

After the generation loop works, add model-compatible prompt rendering and conversation context. Do not hardcode one vendor template while claiming general chat support.

### 3.5 Candle CPU is the current local product target

The native application is composed around Candle, immutable Hugging Face Hub artifacts, Safetensors, and the Hugging Face tokenizer. Candle is the sole current local execution engine. A new model format or device does not by itself justify another engine or E0 ownership architecture.

GPU support and Candle-native GGUF/quantized loading are separate deferred work. Neither is part of the current product.

### 3.6 Keep folder movement out of the first vertical slice

During the first vertical slice, do not rename folders merely to look more conventional. Folder movement without an ownership change creates churn without improving the dependency graph.

That constraint ended after the Phase 6 product milestone. The workspace now uses `domain`, `platform`, `adapters`, `runtime`, and `apps`; the move preserves the existing logical dependency roles and is enforced as the current layout rather than supported through permanent legacy aliases.

### 3.7 Use an exact reviewed domain DAG

The first vertical slice kept F1 peers isolated while responsibilities stabilized. Phase 9 replaced that temporary universal ban with the exact reviewed acyclic policy in ADR-0015. Every domain production edge now requires a source/target/kind rationale, and `domain-contracts` inclusion requires a real backend/runtime crossing or stable use by at least two distinct domains.

### 3.8 Separate component and cross-crate measurements

A benchmark for one stable crate-owned operation remains in that crate's conventional `benches/` directory and is created only with a real production-code measurement that answers a named question. No placeholder benchmark directories are created.

Cross-crate E0/E1 and product-level measurements belong in the future exact root-workspace package `benchmarks/runtime` (`runtime-benchmarks`). The package is a non-production outer consumer of exact reviewed public production APIs; production packages never depend on it. ADR-0018 owns workspace, artifact, and fixture policy.

## 4. Scope guardrails

The first streamed generation milestone is complete. From Phase 7 onward, do not pull later research tracks into the active phase merely because the new conversation surface makes them imaginable. In particular, do not add:

- hosted-provider, peer-routing, or browser transport implementation during Phase 7;
- a general workflow system, long-term-memory system, or tool/permission framework as part of chat integration;
- a second local engine, unsupported model format, or local-file product without a separate reviewed decision;
- multi-model residency in the application façade;
- GPU execution before its explicit device/build/test phase;
- new model architecture families;
- speculative micro-crates;
- broad folder renames without a new ownership/dependency reason;
- performance annotations without measurements;
- hard wall-clock benchmark thresholds on shared CI runners.

The existing corrective workflow may receive correctness fixes, but Phase 7 must not turn its fixed six-stage behavior into the universal workflow architecture. New future boundaries require the same ownership/lifecycle/reuse evidence as existing runtime roles.

## 5. Operating rules for agents

Every work package should follow these rules.

1. Read, in order:
   - `docs/agent/execution/analyzer.md`;
   - this plan;
   - `docs/README.md` once Phase 0 creates it;
   - the component document relevant to the package.
2. Run the repository baseline command before editing. During early phases this is `cargo run --locked --bin llm-app -- verify`; after the xtask migration it is `cargo xtask verify`.
3. Do not mix phases in one change unless the later change is required to make the earlier one compile.
4. Preserve public APIs unless the work package explicitly authorizes an API change.
5. Add tests for every new invariant and every reproduced failure.
6. Do not claim allocation-free, portable, backend-neutral, chat-compatible, or GPU-capable behavior unless a named test or measurement supports it.
7. Update the canonical status document in the same change as the implementation.
8. Record architectural decisions in an ADR rather than silently changing doctrine.
9. Keep pull requests reviewable: one invariant, one subsystem slice, or one clearly bounded migration at a time.
10. Leave the workspace compiling and the quality gate passing at every merge point.

## 6. Phase map

| Phase | Outcome | Hard gate |
|---|---|---|
| 0 | Documentation becomes a truthful execution input | No contradictory canonical claims or broken internal links |
| 1 | Reproducible CI and architecture enforcement | Required checks pass from a clean checkout |
| 2 | Runtime load/start/shutdown paths become transactional | Fault-injection cleanup tests pass |
| 3 | Backend-independent generation scheduler works | Deterministic fake backend streams and cancels correctly |
| 4 | Candle CPU completes a prompt-to-token loop | Real Candle smoke path produces tokens and cleans up |
| 5 | `application-runtime` exposes generation cleanly | Frontend-neutral integration tests pass |
| 6 | Slint is a usable streamed-completion product | User can generate, cancel, unload, and close safely |
| 7 | Conversation/context behavior is real | Budgeting, rendering, history, and stop tests pass |
| 8 | Historical GGUF parity experiment completed | Historical shared-suite evidence remains in Phase 8 history; current support is superseded by ADR-0013 |
| 9 | Candle-only architecture, lifecycle ownership, domain DAG, workspace tooling, modules, and lint policy are reconciled | One-worker E1, retained/retryable joins, reviewed DAG, virtual workspace/xtask, Rust-owned hygiene, stable mandatory lints, truthful docs, and exact-tree validation |
| 10 | Performance is measured end to end | TTFT, throughput, memory, cancellation, unload baselines exist |
| 11 | GPU execution is added without weakening CPU behavior | Device matrix and fallback tests pass |

---

# Phase 9 — Reconcile and simplify the architecture using integration evidence

**Status:** Complete on 2026-08-01. The Candle-only checkpoint closed first; the structural and lifecycle work below then closed the phase before Phase 10.

## Objective

Remove the accidental dual-engine/tooling architecture while preserving behavior proven by earlier phases. Establish a coherent Candle/Hub/Safetensors/CPU baseline, harden ownership failures found during review, and simplify structure only where current responsibilities justify it.

## Work package 9.1 — Restore one local execution composition (complete)

- ADR-0013 makes Candle the sole local execution engine.
- Engine, artifact source, model format, scalar, and device remain separate facts.
- Former native adapter/product, local-file identity/configuration, active-backend routing, dormant worker, placeholder variants, and product-specific UI branches are absent.
- `ModelSelection` remains Hugging Face repository plus revision and retains the resolved immutable Hub commit.
- Candle-native GGUF/quantization and GPU support remain separately reviewed future work.

## Work package 9.2 — Narrow `application-runtime` (complete)

- `corrective-workflow` remains outside E1 as an independent capability engine.
- E1 composes one `HostedRuntime<CandleLlamaSource>`, one inference thread, one Hub worker, one concrete tokenizer, request-local decoders, and redb persistence behind a non-generic façade.
- No second runtime, public plugin registry, or speculative `application-api` was introduced.

## Work package 9.3 — Preserve behavior and harden lifecycle ownership (complete)

- E0 remains backend-neutral at its contracts and statically dispatched in production token-sensitive work.
- Deterministic loaders and the committed Candle fixture retain download-free scheduler, transaction, cancellation, backpressure, cleanup, unload, and shutdown coverage.
- E1 retains resolution, completion, exact chat, context, regeneration, persistence, unload, disconnection, and Slint presentation behavior.
- Startup owns partially created inference state in a rollback guard and performs bounded reverse cleanup after Hub startup failure; rollback timeout retains the owner in a private retry quarantine rather than detaching it.
- Rejected incompatible models retain private handle/unload accounting through retry, proven disconnection, success, or observable exhaustion.
- Shutdown retains both join handles after timeouts and distinguishes running, stopping, stopped, and failed/retryable outcomes; only confirmed stop is idempotent success.

## Work package 9.4 — Reconcile operational tooling, CI, and documentation (complete)

- ADR-0014 keeps project-owned operational tooling Rust/Cargo-native.
- The temporary cleanup brief and its broad filename exemption were removed; every tracked operational surface is scanned.
- The opt-in E1 Candle/Hub example remains the only maintained network smoke; ordinary tests remain download-free.
- Ubuntu CI retains `build-essential` and Slint packages but not system CMake. Required clean-target validation forces non-FIPS AWS-LC through its CC builder and fails on CMake, Clang, Python/package-tool, or Python-distributed Hub CLI invocation.
- Current architecture, status, component, validation, execution, and frontend documentation is reconciled while Phase 8 remains historical.
- Commit and Git-tree provenance is logged by CI outside the commit rather than requiring a document to contain its own SHA.

## Work package 9.5 — Complete structural reconciliation (complete)

- ADR-0015 replaces the universal F1-peer ban with an exact reviewed acyclic domain DAG. The current graph contains the four real F1 → F0 edges; unreviewed peer edges still fail closed.
- `TaskId` moved from `domain-contracts` to `task-graph`; shared-foundation inclusion now requires a backend/runtime crossing or at least two stable distinct domain consumers.
- E0 runtime operations, E1 generation, task graph/artifact/state/error behavior, desktop presentation, and repository policy tooling were split by responsibility while atomic lifecycle transactions remained intact.
- ADR-0016 makes the root a virtual workspace, moves custom policy/composite verification to `tools/xtask`, and removes pass-through commands for ordinary Cargo operations.
- ADR-0017 keeps the stable selected Clippy set mandatory under `-D warnings` and moves the blanket nursery group to a separate scheduled, non-blocking report.

## Acceptance criteria

- Candle is the sole local engine; current source is immutable Hugging Face Hub Safetensors on CPU.
- E1 owns one inference worker/thread plus one Hub worker and one resident model.
- No dead second-product API, routing, worker, UI, fixture, manifest, lockfile, or selected-graph path remains.
- Startup rollback, rejected-model cleanup ownership, retryable shutdown joins, completion, exact chat, context/regeneration, backpressure, cancellation, unload, and persistence are covered.
- The domain graph is exact, justified, and acyclic; `domain-contracts` is not used to evade dependency review.
- The root is virtual, `cargo xtask verify` is the sole composite gate, and one-step operations use Cargo directly.
- Mandatory and exploratory lints have separate acceptance semantics.
- Ordinary tests are download-free; the external E1 smoke is explicit, Rust-native, exact-revision, and opt-in.
- Repository hygiene, architecture, clean forbidden-tool compilation, portability, local links, dependency policy, and the canonical gate pass.
- Current documents describe the corrected tree; completed Phase 8 and the first Candle-only Phase 9 checkpoint remain clearly historical.

---

# Pre-Phase 10 closure — lifecycle, benchmark architecture, and fixture policy

**Status:** Complete on 2026-08-01. This closure establishes the baseline for Phase 10 but records no Phase 10 measurement.

- E0 names failed-cleanup disposition `RetainUntilProcessExit`: after structured cleanup exhaustion it deliberately forgets the runtime, terminates the worker, and relies on process exit for reclamation rather than invoking unverified implicit backend destruction.
- E1 distinguishes running, stopping, cleanly stopped, retryable failure, and sticky terminal failure. Join timeout remains retryable; a retained E0 cleanup failure or unproven endpoint disconnection can never become success merely because handles are absent.
- Component benchmark placement, the future exact `benchmarks/runtime` package role, root workspace/lock/target ownership, dependency direction, and generated-artifact policy are established by ADR-0018 and fail-closed tooling tests.
- The prior Candle fixture provenance was not sufficiently established. Its bytes were replaced by a Rust/Cargo-generated deterministic synthetic fixture with explicit hashes, architecture, licensing, and non-claims.
- No `benchmarks/runtime` package, placeholder `benches/` directory, Phase 10 harness, performance optimization, or performance result was added.

# Phase 10 — Build a meaningful performance program

**Status:** Not started; next implementation phase.

## Objective

Measure product behavior before applying low-level optimization doctrine.

## Work package 10.1 — Expand the existing sampling benchmark

This is the mandatory component measurement. Keep it in `crates/domain/sampling/benches/` and execute the production sampler. Cover:

- greedy;
- default top-k/top-p;
- min-p;
- repetition penalty with varied histories;
- approximately 8k, 32k, and 128k vocabularies;
- sampler-only timing with setup outside measurement;
- full restore-plus-sample pipeline as a separately named benchmark;
- stop matching.

Every case states the regression or performance question it answers. The benchmark name must make clear whether input restoration is inside the measured region.

Tokenizer encode/streaming-decode, context-planner, output-accumulator, and isolated Candle prefill/decode microbenchmarks are conditional rather than checklist requirements. Before adding any one of them, document:

1. the named question;
2. why the cross-crate system measurement cannot answer it;
3. the stable production operation being measured;
4. the setup excluded from the measured region.

Create its crate-local `benches/` directory only in the same change as that justified benchmark. Do not implement a benchmark-only copy of production logic.

## Work package 10.2 — Add one cross-crate runtime/system harness

Create exactly the dedicated root-workspace package:

```text
benchmarks/runtime
```

The package name is `runtime-benchmarks`. Add the exact root member and manifest in the same change before running Cargo. It uses the root lockfile and target directory, declares `publish = false`, has no build script, and depends only on exact reviewed public production APIs and controlled fixtures. No production package depends on it.

The mandatory harness measures the current Candle CPU product path sufficiently to answer:

- time to first token;
- steady-state decode throughput;
- prompt prefill throughput where it can be separated honestly;
- output backpressure behavior;
- cancellation latency;
- model load and unload latency;
- repeated load/generate/unload stability.

Do not add GPU, GGUF, hosted, peer, browser, workflow, memory-system, or multi-model variants. Real-model runs follow ADR-0018: explicit external identifier, immutable revision, local cache or explicit local artifact path, opt-in execution, no ordinary-CI download, and no repository redistribution.

## Work package 10.3 — Record reproducible environment, lifecycle, and memory evidence

Every controlled baseline records:

- commit and Git tree;
- Rust/Cargo/LLVM and Criterion versions;
- target triple and build profile/features;
- CPU model, core/thread policy, OS/kernel, power mode, and relevant environment controls;
- model identifier, immutable revision or synthetic-fixture hash;
- prompt/context sizes and generation settings;
- warm-up/sample configuration.

Measure controlled cancellation and load/unload lifecycle behavior plus host memory before load, after load, during generation, after unload, and across repetition. State the memory observation method and its limitations; do not claim native allocation freedom from Rust allocator evidence.

Shared CI compiles all benchmark targets and catches API drift. It does not run statistical measurements or enforce wall-clock thresholds. Raw Criterion data, generated reports, caches, profiles, and dumps remain under the shared root `target`; only curated summaries enter canonical documentation.

## Work package 10.4 — Optimize only measured bottlenecks

Optimization is a later Phase 10 change, not part of harness establishment. Use the system/component evidence plus profiling or generated-code inspection before adding:

- `#[inline(always)]`;
- custom unsafe code;
- manual SIMD;
- alternative collections;
- lock-free structures;
- data-layout rewrites;
- custom allocators.

Preserve the existing zero-allocation project-owned hot-path goal where it is already useful, but report upstream adapter allocations honestly.

## Acceptance criteria

- The existing sampling benchmark is expanded and its measured regions are named precisely.
- One `benchmarks/runtime` harness measures TTFT, decode throughput, controlled lifecycle, and memory behavior through public production APIs.
- Environment and artifact identity are reproducible.
- Conditional microbenchmarks exist only with a named question and a documented reason the system harness is insufficient.
- The package uses root workspace/lock/target ownership, `publish = false`, no build script, and no incoming production dependency.
- Raw generated output and model caches remain untracked under root `target` or outside the repository.
- Shared CI compiles but does not time-gate the suite.
- Any optimization change cites a same-environment baseline and resulting measurement.

---

# Phase 11 — Add GPU execution

## Objective

Introduce GPU support as an adapter/device capability without redesigning the application or weakening CPU fallback.

## Work package 11.1 — Define supported device matrix

Select explicit targets, for example:

- Candle CPU;
- Candle CUDA on supported Linux/Windows environments;
- Candle Metal where supported.

Do not expose a generic “GPU” option without identifying backend and device kind.

## Work package 11.2 — Add feature and build matrix

Introduce deliberate Cargo features for device backends. Avoid assuming `--all-features` is valid when CUDA/Metal or mutually exclusive native configurations exist.

CI should use an explicit matrix. Hardware-required runtime tests may be optional/labelled, while CPU fallback and feature compilation remain mandatory.

## Work package 11.3 — Implement device discovery and admission

Add:

- device enumeration;
- stable device identifiers;
- backend/device compatibility reporting;
- GPU memory planning;
- model and sequence memory admission;
- clear unsupported-device failures;
- deterministic resource synchronization and unload;
- CPU fallback policy chosen by the user, not silently applied after an incompatible selection.

## Work package 11.4 — Expose device selection through E1

`application-runtime` should expose a frontend-neutral device summary and selection. Slint maps it to widgets without importing backend libraries.

## Work package 11.5 — Measure GPU behavior

Record:

- load time;
- TTFT;
- token throughput;
- host and device memory;
- cancellation latency;
- unload/synchronization duration;
- CPU comparison;
- transfer and fallback behavior.

## Acceptance criteria

- GPU inference actually executes on the selected device rather than merely compiling GPU features.
- CPU behavior remains covered and available.
- Unsupported combinations fail before partial model residency where possible.
- Device memory is released on cancellation, unload, shutdown, and contract failure.
- UI/device labels accurately reflect execution.

---

# Future track — Add peer and hosted model execution

This track begins only after application conversation semantics and the local
composition boundary are stable enough to expose a coarse model-execution seam.
It does not turn E0 into a network/provider abstraction.

This is not Phase 12 and it does not depend on GPU support. It may begin when conversation semantics are stable and a real second execution/deployment need proves the coarse seam; Phase 11 is not a prerequisite.

Define one application-facing execution contract for:

- target identity and capability discovery;
- complete generation request admission;
- supported message/prompt input form;
- context and token-accounting semantics;
- sampling and tool capabilities;
- cancellation intent and target guarantees;
- bounded streamed output;
- usage and terminal results.

The local implementation delegates complete requests to E0. Provider adapters own
authentication, vendor DTOs, response translation, rate/error normalization, and
provider-specific capabilities. Peer execution uses the same semantic boundary but
routes through the node protocol rather than a hosted-provider client.

External execution is explicit. The application must show when context leaves the
user's machines, which target receives it, and which capabilities or guarantees
differ. Credentials never enter model context or conversation history.

Peer networking assumes existing reachability through LAN, WireGuard, Tailscale,
NetBird, or equivalent infrastructure. This project owns peer identity/discovery,
capability advertisement, routing, and its application protocol; it does not become
a VPN implementation.

Acceptance requires that local, peer, and hosted targets can satisfy the same
conversation/workflow intent without provider SDK or transport types entering E1
domain state, and without representing remote services as E0 backends.

When the first non-local execution target is implemented, review `task-graph::ModelPolicy`. Its current `PreferredBackend(BackendId)` vocabulary refers to a compiled local inference backend. Do not repurpose `BackendId` to mean a provider, peer, deployment, or generic execution target merely to reuse the existing enum. Move or replace that selection policy only when the real execution contract supplies the required vocabulary.

---

# Future research tracks — preserve direction without pre-committing architecture

These tracks come from [the project vision](../../vision.md). They are intentionally unnumbered and unordered. Recording them prevents the larger direction from disappearing, but it does not authorize speculative crates, protocols, or public interfaces. Promote one into an implementation phase only when product evidence and a concrete acceptance scenario make the boundary testable.

- **Composable prompt/work workflows.** Explore pre-generation enrichment, concurrent narrow observers, post-processing, and feedback/revision paths that can be reordered, bypassed, or combined. `corrective-workflow` remains one concrete six-stage capability; it must not expand into the universal workflow runtime by accident.
- **Long-term memory and active-context repair.** Preserve raw provenance while allowing the active representation of earlier information to be corrected, condensed, replaced, or retrieved. The moving-window/toroidal-grid idea is a research direction, not a selected storage structure. Any memory runtime must earn an independent ownership/lifecycle boundary.
- **Tools, permissions, authority, and trust.** Model access to machine capabilities should be explicit, narrow, inspectable, revocable, and purpose-specific. Peer authentication must not imply broad trust, and credentials/authority must not become ordinary model context.
- **Long-lived node and multiple frontends.** A TUI, desktop app, headless service, or later browser client should share application semantics. Interface lifetime and node/service lifetime remain separate; a terminal or window closing must not define the lifetime of a node intended to keep serving work.
- **System-native integration.** The longer-term OS/capability experiments may inform llm-app, but current contracts should not be distorted around a speculative custom operating system. First prove useful capability boundaries on normal hosts.

For all of these tracks, prefer explicit coarse contracts at real ownership boundaries over a universal agent/service abstraction.

---

# 12. Parallel work and dependencies

The critical path is:

```text
0 -> 1 -> 2 -> 3 -> 4 -> 5 -> 6 -> 7 -> 8 -> 9 -> 10 -> 11
```

The peer/hosted execution track and the research tracks above are not additional links in this numeric critical path. They start from their own evidence/review triggers.

Limited parallel work is safe:

- license files, link checks, and CI scaffolding may proceed in parallel after Phase 0 defines canonical docs;
- transactional runtime tests can begin while CI is being installed, but should merge after the baseline gate exists;
- Slint layout exploration may occur after E1 generation API shapes are documented, but behavior must not be duplicated in the presenter;
- benchmark design may be drafted early, but implementation and interpretation wait for the vertical slice;
- workflow internal module splitting may happen after Phase 6 if it does not alter the generation critical path.

Unsafe parallel work:

- building the UI generation state machine before E1 owns it;
- exposing an unsupported model format or device in the UI before compatibility and generation evidence;
- introducing GPU features before explicit feature-matrix CI;
- moving all crates while generation integration is changing dependencies;
- extracting application crates before a second composition proves the seam.

# 13. Pull-request/work-package template

Each agent-created change should include this information in its description:

## Context

- plan phase and work-package ID;
- problem being solved;
- current invariant or failure.

## Scope

- files/crates intentionally changed;
- public API changes;
- explicit non-goals.

## Design

- ownership changes;
- capacity/allocation behavior;
- error and rollback behavior;
- cancellation/shutdown implications;
- relevant ADR.

## Verification

- commands run;
- tests added;
- fault-injection cases;
- model/device fixture used, when applicable;
- benchmark result, only when performance is claimed.

## Documentation

- canonical status updated;
- component guide updated;
- ADR added or amended when the architectural decision changed.

# 14. Definition of done for the first product milestone

The first product milestone is complete only when all of the following are true:

- documentation is internally consistent and CI-enforced;
- the architecture validator checks the actual graph and fails closed;
- model load and sequence creation are transactional;
- one supported Candle CPU model produces streamed text;
- sampling runs in the inference scheduler;
- output is pulled in bounded batches rather than emitted per token;
- the application façade owns prompt/generation state;
- Slint contains no duplicated backend or generation orchestration;
- cancellation cleans up the request;
- unload releases the model after generation;
- normal application closure performs bounded shutdown;
- a deterministic fake-backend suite covers failures and invariants;
- a documented real-model smoke path is reproducible;
- baseline TTFT, throughput, cancellation, memory, and unload observations exist.

This milestone does **not** require another model format, general chat templates, multiple resident models, remote clients, or GPU support.

# 15. Traceability from `analyzer.md`

| Analyzer finding | Addressed in |
|---|---|
| Central generation loop absent | Phases 3–6 |
| `application-runtime` is a valid façade but concrete/growing | Architecture closure and Phases 5, 8, 9; ADR-0013 removes duplicate local composition |
| Candle/HF/redb composition at E1 | Phase 8 historical review and Phase 9 one-consumer correction |
| Corrective workflow dominates E1 | Architecture closure extraction; Phase 9 keeps E1 narrow |
| Single-model state conflicts with `maximum_models` | Phase 5.6 |
| Model-load cleanup bypass | Phase 2.1 |
| Sequence/request commit not rollback-safe | Phase 2.2–2.3 |
| Folder taxonomy is unconventional but understandable | Closed by ADR-0009 before Phase 7 |
| F1-to-F1 ban is too absolute | Phase 9.5 and ADR-0015 |
| `domain-contracts` junk-drawer pressure | Phase 9.5 and ADR-0015 |
| Sampling benchmark placement is correct | Decision 3.8 and Phase 10 |
| Sampling benchmark coverage is narrow | Phase 10.1 |
| Root runner is an xtask in disguise | Phase 9.5 and ADR-0016 |
| Wrapper commands reimplement Cargo | Phase 9.5 and ADR-0016 |
| `cargo test --all-targets` selects benches | Phase 1.1 |
| Validator not required in CI | Phase 1.1 |
| Validator ignores external dependencies | Phase 1.2 |
| Unknown paths become applications | Phase 1.2 |
| Dependency kind is ignored | Phase 1.2 |
| Validator test matrix is partial | Phase 1.2 |
| Large source modules | Phases 2.5 and 9.5 |
| Explicit shutdown not guaranteed by `Drop` | Phases 2.4 and 6.3 |
| Deadline overflow | Phase 2.4 |
| Accidental README model heading | Phase 0.2 |
| Broken documentation links | Phases 0 and 1 |
| Unverifiable status claims | Phase 0.4 |
| Missing license files | Phase 1.3 |
| Missing supply-chain checks | Phase 1.3 |
| `no_std` claims lack target checks | Phase 1.4 |
| Inaccurate Rust performance guidance | Phase 0.3 |
| CPU-only despite device vocabulary | Phase 11 after CPU milestones |

# 16. Expected repository shape after the plan

The exact result depends on evidence gathered during integration, but a likely stable shape is:

```text
Cargo.toml                        # virtual workspace
.cargo/config.toml
LICENSE
NOTICE
TRADEMARKS.md
CONTRIBUTING.md
deny.toml

branding/
  README.md

docs/
  README.md
  architecture.md
  rules.md
  decisions/
  execution/
    analyzer.md
    execution-plan.md
  project/
  knowledge/

crates/
  domain/
    domain-contracts
    tokenization
    context-planner
    sampling
    task-graph
  platform/
    host-runtime
  adapters/
    candle-backend
    hf-hub
    hf-tokenizer
    redb-storage
  runtime/
    inference-runtime
    corrective-workflow
    application-runtime
    # local-model runtime only if a future independent lifecycle/consumer proves the split
  apps/
    desktop-slint
    # optional CLI used to prove frontend reuse

tools/
  xtask

benchmarks/
  runtime                            # only after the vertical slice
```

This structure is intentionally conservative. It adds boundaries only when the running product demonstrates a reason for them.

# 17. Final execution priority

When trade-offs arise, use this order:

1. native resource correctness;
2. end-to-end product behavior;
3. cancellation, backpressure, and deterministic cleanup;
4. truthful public/application API;
5. reproducible tests and CI;
6. understandable module and dependency structure;
7. measured performance;
8. additional model formats, execution targets, and devices;
9. speculative generality.

The project should preserve its strong low-level discipline, but every new abstraction must now justify itself against a running generation loop.
