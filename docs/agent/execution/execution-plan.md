# LLM App Execution Plan

**Intended repository path:** `docs/agent/execution/execution-plan.md`
**Companion analysis:** `docs/agent/execution/analyzer.md`
**Plan status:** Active execution baseline
**Prepared:** 2026-07-22

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
    -> GGUF parity and composition cleanup
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

The first integrated prompt-to-stream generation loop now exists through E0, E1, and Slint. The immediate problem is semantic integration: turn a proven direct completion into conversation behavior without binding conversation state to Candle/local execution, duplicating application state in frontends, or collapsing context planning and model-specific rendering into one catch-all runtime.

The pre-Phase 7 architecture closure also extracted `corrective-workflow`, adopted the `domain`/`platform`/`adapters`/`runtime`/`apps` physical taxonomy, and made runtime/platform roles and runtime composition edges fail closed. Phase 7 builds on those boundaries rather than reopening them without new evidence.

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

### 3.5 Candle CPU is the first product target

The existing native application is already composed around Candle, Hugging Face artifacts, and the Hugging Face tokenizer. Use that path to prove the first slice. Then run the same application-level generation contract against GGUF.

GPU support is deferred until CPU correctness, cancellation, output backpressure, and system benchmarks exist.

### 3.6 Keep folder movement out of the first vertical slice

During the first vertical slice, do not rename folders merely to look more conventional. Folder movement without an ownership change creates churn without improving the dependency graph.

That constraint ended after the Phase 6 product milestone. The workspace now uses `domain`, `platform`, `adapters`, `runtime`, and `apps`; the move preserves the existing logical dependency roles and is enforced as the current layout rather than supported through permanent legacy aliases.

### 3.7 Replace absolute layer doctrine with an approved DAG later

Do not change the F0/F1 policy during the first vertical slice unless it blocks integration. Record exceptions rather than pushing unrelated vocabulary into `domain-contracts`. After the slice, replace the universal F1-to-F1 ban with a reviewed acyclic dependency policy.

### 3.8 Component benchmarks remain with their crates

`crates/domain/sampling/benches/sampling_pipeline.rs` is correctly located. Cross-crate and end-to-end benchmarks should be added as a dedicated benchmark workspace package after the generation path exists.

## 4. Scope guardrails

The first streamed generation milestone is complete. From Phase 7 onward, do not pull later research tracks into the active phase merely because the new conversation surface makes them imaginable. In particular, do not add:

- hosted-provider, peer-routing, or browser transport implementation during Phase 7;
- a general workflow system, long-term-memory system, or tool/permission framework as part of chat integration;
- GGUF product selection before Phase 8 proves tokenizer/generation parity;
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
| 8 | GGUF reaches behavioral parity | Shared generation suite passes for Candle and GGUF |
| 9 | Architecture is simplified using integration evidence | Dependency policy, modules, façade, and tooling are coherent |
| 10 | Performance is measured end to end | TTFT, throughput, memory, cancellation, unload baselines exist |
| 11 | GPU execution is added without weakening CPU behavior | Device matrix and fallback tests pass |

---

# Phase 0 — Establish documentary truth

## Objective

Make repository documentation safe for humans and coding agents to treat as authoritative.

## Work package 0.1 — Create a documentation authority map

Create `docs/README.md` with four document classes:

1. **Normative architecture** — current enforced boundaries and invariants.
2. **ADRs** — decisions, alternatives, and consequences.
3. **Execution and status** — analyzer, this plan, and verified current state.
4. **Component guides** — runtime, backend, frontend, and workflow behavior.

State which document wins when two documents conflict. Recommended precedence:

```text
current ADR
  > current architecture document
  > current status document
  > component guide
  > historical implementation plan
  > knowledge notes
```

Keep `docs/project/` initially to avoid a large path migration. Index it clearly rather than moving every file at once.

## Work package 0.2 — Correct the root README

Required corrections:

- remove the accidental `HauhauCS/Gemma4-12B-QAT-Uncensored-HauhauCS-Balanced` heading;
- describe the repository as CPU-only today;
- state precisely that Candle is the currently composed application backend;
- state that GGUF exists at the adapter/E0 compatibility boundary but is not yet available through E1/UI;
- distinguish direct completion, chat generation, and planned behavior;
- fix links to `docs/project/...`;
- stop presenting an old phase sequence as the current roadmap;
- point to `docs/agent/execution/execution-plan.md` and the canonical status page.

## Work package 0.3 — Rewrite architectural doctrine as evidence-based rules

Update `docs/architecture.md`, `docs/rules.md`, and `docs/agent/knowledge/rust_knowledge.md`.

Correct at least these statements:

- `const fn` enables const evaluation; it does not automatically move runtime work to `.rodata`;
- `core::error::Error` exists on modern Rust;
- dynamic dispatch is prohibited only in measured hot paths, not in every architectural boundary;
- `#[inline]`, `#[inline(always)]`, and `#[cold]` are hints, not guarantees;
- ECS does not automatically provide the desired SoA layout;
- the “16-byte struct limit” is not a universal ABI law;
- test fakes and fault-injection backends are allowed;
- production code must be complete, but experimental branches and spikes may exist without being merged as fake behavior;
- crate counts are outcomes of ownership and reuse, not numerical quotas.

Classify rules as:

- hard invariant;
- current decision;
- performance hypothesis;
- style preference;
- temporary constraint.

## Work package 0.4 — Reconcile plans and status

The current `docs/project/implementation-plan.md` refers to structures that no longer match the workspace, while `implementation-status.md` makes validation claims without a reproducible commit/CI reference.

Choose one of the following and apply it consistently:

- mark the old implementation plan as historical and link to this plan; or
- rewrite it as a concise architecture history.

Create or rewrite a canonical current status page that records:

- exact supported backends and devices;
- what is wired through E0, E1, and the UI;
- which checks were run;
- the toolchain and commit used for the result;
- known limitations;
- the active phase from this plan.

A status claim such as “validated” must include either a CI run or a reproducible command and commit.

## Work package 0.5 — Add initial ADRs

Create `docs/agent/decisions/` and add:

- **ADR-0001:** `application-runtime` remains the frontend-neutral façade.
- **ADR-0002:** CPU Candle is the first vertical-slice backend.
- **ADR-0003:** generation scheduling lives beside model execution; frontends do not drive token steps.
- **ADR-0004:** direct completion precedes general chat-template support.
- **ADR-0005:** existing crate folders remain until ownership evidence justifies movement.
- **ADR-0006:** explicit bounded shutdown is required; blocking `Drop` is not the primary shutdown mechanism.

Each ADR must contain context, decision, rejected alternatives, consequences, and review trigger.

## Acceptance criteria

- `docs/README.md` identifies canonical documents and precedence.
- Every internal Markdown link resolves.
- The README contains no unsupported model/device claims.
- Documentation consistently says CPU-only.
- Historical plans are visibly historical.
- `rules.md` permits deterministic test doubles.
- No canonical document describes performance folklore as a universal language guarantee.
- The active status points to this execution plan.

---

# Phase 1 — Build a reproducible quality gate

## Objective

Make architecture, correctness, documentation, and repository hygiene enforceable rather than optional.

## Work package 1.1 — Add CI before restructuring tooling

Use the existing root runner initially so CI protection arrives before the xtask migration.

Required checks:

```text
cargo fmt --all -- --check
cargo run --locked --bin llm-app -- architecture
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo doc --workspace --no-deps --locked
cargo bench --workspace --no-run --locked
```

Do not use `cargo test --all-targets` as the normal correctness command because it selects benchmark targets. Compile benchmarks separately.

Add explicit platform jobs only where the native dependencies are expected to work. Begin with the primary development platform, then add Windows/macOS jobs after their native toolchains are documented.

## Work package 1.2 — Strengthen the architecture validator

The validator must:

- use the typed `cargo_metadata` API;
- fail closed on unknown workspace paths;
- require explicit classification for E0, capability, and E1 runtime roles; unknown runtime crates must fail closed;
- distinguish normal, build, and development dependencies;
- test the complete layer matrix;
- analyze the actual workspace graph in an integration test;
- report why an edge is forbidden and which policy rule applies;
- require exact review for production engine dependencies on adapters or other engines;
- enforce external dependency rules for portable crates;
- avoid treating runtime-folder placement as an implicit capability-engine role.

Initial external dependency policy:

- F0: no external production dependencies without an explicit exception;
- F1: only reviewed portable dependencies, currently including `libm` where required;
- adapters: vendor, filesystem, network, database, FFI, and host dependencies allowed;
- engines: dependencies appropriate to orchestration, but no frontend toolkit;
- apps: depend on E1 rather than E0/adapters in production;
- dev dependencies: separately reviewed and allowed for test compatibility or benchmarks.

Do not embed undocumented exceptions in code. Store the policy as inspectable data or document every exception next to the validator.

## Work package 1.3 — Add repository hygiene checks

Add:

- `LICENSE-MIT`;
- `LICENSE-APACHE`;
- `cargo-deny` policy for advisories, licenses, sources, and reviewed duplicates;
- a Markdown link checker;
- `cargo tree -d` as an audit report, not an automatic demand to eliminate every duplicate;
- generated-file and lockfile consistency checks where useful.

## Work package 1.4 — Define real portability targets

Select named targets through an ADR. Do not claim generic bare-metal support.

For each portable crate, document whether it supports:

- host `std` tests;
- `wasm32-unknown-unknown`;
- one selected `no_std` embedded target;
- allocation-free execution in project-owned code.

Add CI `cargo check` jobs for only the targets actually supported. Adapter, engine, and app crates are excluded from `no_std` target claims.

## Work package 1.5 — Record a clean baseline

Capture:

- toolchain version;
- command outputs;
- warnings;
- test count;
- benchmark compilation status;
- duplicate dependency report;
- binary size where readily available.

Store the summary in the current status document. Do not commit large generated logs unless they are needed for diagnosis.

## Acceptance criteria

- Required CI checks run on every change.
- A forbidden actual manifest edge fails CI.
- An unknown workspace location fails architecture validation.
- External infrastructure added directly to F0/F1 fails policy.
- Ordinary tests do not select benchmark targets.
- Licenses and dependency policy are present.
- Broken documentation links fail CI.
- Portability claims name concrete targets.

---

# Phase 2 — Repair transactional runtime safety

## Objective

Ensure rare backend or invariant failures do not bypass explicit native cleanup or leave registries inconsistent.

## Work package 2.1 — Transactional model loading

Refactor `InferenceRuntime::load_model` into prepare/validate/commit stages.

Required behavior:

1. inspect and plan;
2. reserve admission without publishing a resident slot;
3. load into a local uncommitted owner;
4. validate handle and metadata;
5. on any failure after native load, invoke `prepare_unload()` before drop;
6. publish the model slot and accounting only in an infallible final commit.

Use a rollback guard or equivalent local owner whose cleanup path is explicit and testable.

## Work package 2.2 — Transactional request start

Refactor `start_request` so sequence creation and registry insertion are atomic from the runtime's perspective.

Before mutation, preflight:

- request ID availability;
- sequence ID availability;
- model generation validity;
- model lifecycle eligibility;
- request and memory capacity;
- every map entry that could be occupied.

After native sequence creation:

- validate the returned sequence ID;
- start lifecycle state;
- commit slot and global indexes only after all fallible operations succeed;
- call `destroy_sequence()` on every abandoned sequence;
- restore all counters and reservations when commit does not occur.

## Work package 2.3 — Fault-injection backends

Add deterministic test implementations that can:

- return the wrong model handle;
- return mismatched metadata;
- return the wrong sequence ID;
- fail model cleanup;
- fail sequence destruction;
- report contradictory capacities;
- trigger occupied-index/invariant branches;
- count every cleanup call.

These are test doubles, not production mocks. They are required to prove defensive behavior that a normal backend should never trigger.

## Work package 2.4 — Shutdown correctness

- replace unchecked `Instant::now() + timeout` with validated bounds or `checked_add`;
- document explicit shutdown as mandatory for normal frontend closure;
- ensure the Slint runner invokes it on the normal exit path;
- test worker disconnection, shutdown timeout, join failure, active request cancellation, and unload failure;
- consider best-effort nonblocking disconnect in `Drop`, but do not add an unbounded blocking destructor.

## Work package 2.5 — Split runtime internals only as needed

While implementing transactions, split `inference-runtime/src/runtime.rs` internally if it improves invariant review:

```text
runtime/
  mod.rs
  model_registry.rs
  request_registry.rs
  transaction.rs
  operations.rs
  shutdown.rs
```

Do not create another crate for these modules.

## Acceptance criteria

- Loaded native models are explicitly cleaned when post-load validation fails.
- Created native sequences are explicitly destroyed when request commit fails.
- No registry or accounting mutation survives a failed transaction.
- Fault-injection tests verify cleanup counts and final snapshots.
- Shutdown deadline construction cannot overflow.
- All existing backend compatibility tests continue to pass.

---

# Phase 3 — Implement the backend-independent generation kernel

## Objective

Connect prefill, sampling, decode, cancellation, stop conditions, backpressure, terminal cleanup, and resource accounting without requiring a real model or UI.

The generation kernel must preserve the transactional ownership guarantees established in Phase 2. A generation request must never lose its model or sequence cleanup handle, silently release accounting, or erase the original failure when rollback or terminal cleanup also fails.

## Work package 3.0 — Define rollback and terminal cleanup semantics

Complete this work package before implementing the generation scheduler.

### Objective

Ensure that failed rollback or terminal cleanup does not lose the backend resource handle, corrupt resource accounting, or erase the failure that originally caused cleanup.

The same cleanup state machine must support:

* failed model admission;
* failed sequence admission;
* prefill failure;
* decode failure;
* sampling failure;
* cancellation;
* EOS completion;
* token-limit completion;
* stop-sequence completion;
* output failure;
* drain timeout escalation;
* shutdown.

### Preserve primary and cleanup failures

A rollback or terminal transition may have two independently important failures:

* the primary operation, validation, or generation failure;
* the cleanup failure encountered while releasing the affected resource.

Do not replace the primary failure with the cleanup failure.

Add an allocation-free structured representation capable of retaining both failures. It should identify at least:

* the primary operation;
* the primary failure;
* the cleanup operation;
* the cleanup failure.

The representation must distinguish cases such as:

* backend contract violation followed by model-unload failure;
* backend contract violation followed by sequence-destruction failure;
* prefill failure followed by sequence-destruction failure;
* decode failure followed by sequence-destruction failure;
* cancellation followed by sequence-destruction failure;
* shutdown failure followed by model-unload failure.

Do not introduce recursive boxed errors or heap-allocated error chains into the runtime error taxonomy merely to reproduce `std::error::Error` source chaining.

Stable error categories must remain suitable for translation by `application-runtime`.

### Retain resources whose cleanup failed

A model or sequence must not be dropped merely because its explicit cleanup operation failed.

Introduce explicit pending-cleanup or quarantined ownership for:

* loaded models that failed post-load validation and could not be prepared for unload;
* sequences that failed post-creation validation and could not be destroyed;
* committed generation sequences whose terminal destruction failed;
* models that could not unload during shutdown or maintenance.

Quarantined resources must not appear as normally usable entries in model, request, or sequence registries.

The runtime must retain the only cleanup handle until one of the following occurs:

* cleanup succeeds;
* the backend explicitly reports that retry is impossible and ownership has already been consumed;
* the process terminates.

Do not rely on `Drop` as an undocumented substitute for explicit backend cleanup.

### Maintain truthful accounting

Resources awaiting cleanup must remain included in host and device memory accounting until cleanup succeeds.

Snapshots and diagnostics should distinguish at least:

* normally resident models;
* active requests and sequences;
* resources pending cleanup;
* poisoned or degraded models;
* total accounted host memory;
* total accounted device memory.

A failed cleanup must not make capacity appear available for new work when the backend may still own the resource.

A quarantined sequence must continue to count against its model’s sequence and memory capacity until destruction succeeds.

A quarantined model must continue to count against runtime model and memory capacity until unload preparation succeeds and the model is released.

### Define degraded-state behavior

Define how the runtime behaves after cleanup failure.

At minimum:

* the affected resource cannot accept new work;
* a model with a quarantined sequence cannot be unloaded as though no sequences remain;
* cleanup retries are explicit and bounded;
* maintenance and shutdown process pending-cleanup resources;
* successful cleanup removes the quarantined resource and releases its accounting;
* repeated cleanup failure remains observable;
* no resource is destroyed or unloaded again after confirmed successful cleanup;
* normal request registries remain free of partially committed work.

Prefer poisoning only the affected model when that is safe. Poison the entire runtime only when backend state can no longer be isolated or trusted.

The runtime must expose enough state for `application-runtime` to report that cleanup is pending or that the model is degraded.

### Clarify cleanup retry contracts

Update backend contracts to state whether failed cleanup operations are safe to retry.

For retryable sequence destruction:

* `destroy_sequence()` must borrow the sequence;
* failure must leave the sequence value valid for a later cleanup attempt;
* success must be the only transition that permits the runtime to release the sequence and its accounting.

For retryable model unloading:

* `prepare_unload()` failure must leave the model valid for a later cleanup attempt;
* success must establish that dropping or consuming the model is safe.

If a backend cannot provide retryable cleanup, it must expose that limitation explicitly. The runtime must not infer retry safety from implementation details.

### Define bounded retry policy

Cleanup retries must not form an unbounded busy loop.

Define:

* when cleanup is retried;
* the maximum attempts per maintenance quantum;
* whether retries use a deadline, attempt limit, or both;
* how shutdown interacts with unresolved cleanup;
* how repeated failure is surfaced;
* whether new requests may continue on unaffected models.

Retries must remain responsive to control commands and shutdown deadlines.

### Reuse one terminal cleanup state machine

Admission rollback, normal completion, cancellation, generation failure, drain escalation, and shutdown must use the same underlying cleanup transition rules.

A request may enter terminal cleanup only once.

A sequence destruction operation may be attempted more than once only after a previous attempt failed and ownership was retained.

After successful destruction:

* no later cleanup attempt is permitted;
* sequence accounting is released exactly once;
* request accounting is released exactly once;
* terminal state is published exactly once.

### Fault-injection coverage

Add deterministic tests for:

* primary model validation failure plus unload failure;
* primary sequence validation failure plus destruction failure;
* preservation of both failure classifications;
* retention of failed model cleanup ownership;
* retention of failed sequence cleanup ownership;
* retained memory accounting after cleanup failure;
* retained sequence capacity after cleanup failure;
* rejection of new work against a poisoned or quarantined model;
* successful cleanup on a later maintenance attempt;
* repeated cleanup failure;
* bounded cleanup retries;
* no second destruction after cleanup succeeds;
* no second accounting release after cleanup succeeds;
* shutdown with pending model cleanup;
* shutdown with pending sequence cleanup.

Update Phase 2 fault-injection assertions that currently treat failed explicit cleanup as though the resource were completely released.

### Acceptance criteria

* A failed rollback never loses the only cleanup handle.
* Primary and cleanup failures are both observable.
* Pending-cleanup resources remain included in capacity and memory accounting.
* Pending-cleanup resources cannot serve normal work.
* Cleanup retry behavior is bounded and deterministic.
* Successful cleanup releases ownership and accounting exactly once.
* Failed cleanup does not silently fall back to unverified `Drop` behavior.
* Admission rollback and generation terminal paths use the same cleanup state machine.
* Work packages 3.1 through 3.5 can rely on defined terminal ownership semantics.

## Work package 3.1 — Define the minimum generation request

Add an internal runtime-level generation configuration containing only proven requirements:

* request and sequence identity;
* prompt token storage;
* maximum generated tokens;
* model sequence capacity;
* sampling configuration and seed;
* EOS token set;
* token-based stop sequences;
* scheduler quantum;
* output capacity policy.

Keep frontend-oriented settings separate. `application-runtime` will translate its public `GenerationSettings` into this runtime form.

Do not put display strings, frontend DTOs, repository paths, tokenizer objects, decoded text, or UI state in E0 generation contracts.

Define explicit finish reasons for at least:

* EOS;
* generated-token limit;
* stop sequence;
* cancellation;
* capacity exhaustion;
* output backpressure yield;
* backend failure;
* sampling failure;
* cleanup pending;
* runtime shutdown.

A yielded request must remain distinguishable from a terminal request.

A request whose generation work is complete but whose sequence cleanup is pending must not be reported as fully released.

## Work package 3.2 — Allocate generation workspaces before the hot loop

At request admission, allocate or reserve all request-owned storage:

* logits;
* sampling indices;
* repetition mask or epoch storage;
* prompt-token history;
* generated-token history;
* stop-matcher state;
* any backend-required prefill workspace;
* any backend-required decode workspace;
* bounded token-output storage;
* terminal and cleanup state.

Capacity failures must occur before generation begins or produce a documented graceful finish or yield.

No unchecked resize is allowed inside the decode loop.

The request-admission transaction must validate:

* prompt length;
* maximum generation length;
* total sequence capacity;
* logits capacity;
* sampling workspace capacity;
* repetition-history capacity;
* stop-sequence matcher capacity;
* output accumulator capacity;
* host-memory capacity;
* device-memory capacity;
* model lifecycle;
* model poison or quarantine state;
* request and sequence identity availability.

Workspace allocation must not publish request, sequence, lifecycle, or accounting state until all validation and backend sequence creation have succeeded.

If backend sequence creation succeeds but later validation fails, Work package 3.0 cleanup semantics apply.

## Work package 3.3 — Add a bounded generation scheduler to the inference worker

The worker loop should alternate between:

1. checking control commands;
2. advancing active generation by a bounded quantum;
3. processing unload deadlines, pending cleanup, and maintenance;
4. flushing or publishing available output state.

The initial quantum may be one token for correctness. Later tuning may use a small token or time budget.

Required scheduler properties:

* cancellation is checked before every backend step;
* unload and drain commands remain responsive;
* pending-cleanup work receives bounded maintenance opportunities;
* one request cannot monopolize the worker indefinitely;
* output backpressure yields rather than allocates or blocks permanently;
* yielded requests retain all required state;
* prefill occurs at most once per admitted request;
* decode advances monotonically;
* sampling occurs inside E0;
* generated tokens are recorded before the next decode step;
* usage counters remain correct across prefill and decode;
* backend errors become stable runtime or application failures;
* terminal publication and cleanup follow the state machine from Work package 3.0.

Every request must transition through explicit states such as:

* admitted;
* prefill pending;
* decoding;
* yielded for output;
* cancellation requested;
* terminal cleanup;
* cleanup pending;
* completed;
* failed.

Equivalent names are acceptable, but implicit combinations of booleans should be avoided when they permit contradictory states.

### Cancellation

Cancellation must be checked:

* before prefill;
* after prefill and before sampling;
* before each decode step;
* before each sampling step;
* after returning from a backend step;
* before resuming from output backpressure.

Cancellation latency is bounded by:

* the configured scheduler quantum;
* one currently executing backend operation;
* the worker’s control-command polling cadence.

Cancellation must not skip terminal cleanup.

### Backpressure

When token output capacity is full:

* generation yields;
* no additional backend decode step is performed;
* no token is discarded;
* no token is emitted twice;
* the request remains resumable;
* the worker remains responsive to cancellation, unload, and shutdown.

Backpressure must not allocate additional storage or block the worker indefinitely.

### Terminal cleanup

Every terminal path enters the cleanup state machine exactly once.

A successful sequence destruction is recorded at most once.

If destruction fails:

* the sequence remains runtime-owned;
* sequence and memory accounting remain retained;
* the generation result retains the primary outcome;
* the cleanup failure is also observable;
* bounded cleanup retries occur through maintenance;
* no new work is admitted against a poisoned model when that cannot be done safely.

A request must not disappear from all runtime-owned state while its backend sequence may still exist.

### Fairness

The scheduler must use a bounded policy that prevents starvation.

At minimum:

* each runnable request receives an opportunity to advance;
* yielded requests do not spin while output remains full;
* cleanup retries do not monopolize the worker;
* drain and shutdown deadlines are checked between quanta;
* a failing request does not prevent unrelated healthy requests from progressing unless the model or runtime must be poisoned.

## Work package 3.4 — Add pull-oriented token output

Do not emit one channel event per token.

Add a bounded token accumulator analogous to the existing text output accumulator. The inference worker writes token IDs and terminal or yield records; the application layer pulls batches on its own cadence.

The accumulator must:

* allocate only during cold initialization;
* expose borrowed batches;
* preserve request identity;
* represent token ranges;
* represent yielded state;
* represent generation-terminal state;
* represent cleanup-pending state;
* represent fully released terminal state;
* provide a monotonic cursor;
* use nonblocking producer behavior;
* turn full capacity into `OutputBackpressure`;
* retain allocations after each pull.

Keep the existing text accumulator for frontend-facing decoded text. Do not misuse UTF-8 byte ranges to represent token IDs.

### Cursor and delivery invariants

The token-output cursor must:

* advance monotonically;
* never reuse a token range;
* allow consumers to detect missed or stale reads;
* preserve ordering within a request;
* preserve terminal ordering after the final token;
* distinguish generation completion from resource cleanup completion.

Pulling output must not:

* allocate in proportion to token count;
* copy the entire accumulated history;
* advance generation directly;
* perform tokenization or detokenization inside E0;
* release backend resources.

### Backpressure recovery

After the application layer consumes output:

* capacity becomes reusable without reallocating;
* the generation request becomes runnable again;
* the next scheduler quantum resumes from the exact prior state;
* no token is regenerated merely because publication was delayed.

## Work package 3.5 — Build a deterministic fake model

Create a small test backend with a fixed vocabulary and deterministic logits.

It should support scenarios such as:

* greedy output sequence;
* seeded stochastic output;
* EOS completion;
* token-limit completion;
* stop-sequence completion;
* cancellation before prefill;
* cancellation between decode steps;
* cancellation while output is backpressured;
* output backpressure and resume;
* backend prefill failure;
* backend decode failure;
* sampling failure;
* capacity exhaustion;
* drain timeout escalation;
* sequence destruction failure;
* sequence destruction failure after a generation failure;
* successful cleanup retry;
* repeated cleanup failure;
* model poisoning or quarantine;
* unaffected request progress when isolation permits it.

The test backend should be small enough for ordinary CI and must not download model files.

Its behavior must be controllable through deterministic fault-injection configuration rather than timing-sensitive races.

The fake model should expose counters for at least:

* model loads;
* model unload attempts;
* sequence creation;
* sequence destruction attempts;
* successful sequence destruction;
* prefill calls;
* decode calls;
* sampling opportunities;
* active native resources;
* retained simulated memory.

These counters should make duplicate cleanup, leaked ownership, or incorrect scheduler advancement observable.

## Documentation

Update:

* `crates/runtime/inference-runtime/README.md`;
* `docs/project/inference-runtime.md`;
* `docs/project/implementation-status.md`;
* runtime lifecycle and failure-taxonomy documentation;
* backend contract documentation;
* any architecture diagrams that describe request ownership.

Document:

* the generation request lifecycle;
* scheduler fairness and quantum behavior;
* cancellation guarantees;
* backpressure behavior;
* terminal finish reasons;
* pending-cleanup ownership;
* cleanup retry semantics;
* model or runtime poisoning policy;
* accounting behavior during cleanup failure;
* the distinction between generation completion and resource release.

Do not describe GPU execution, real-model generation, frontend streaming, or tokenizer integration as complete during this phase.

## Validation

Run the current canonical root verification command:

```text
cargo run --locked --bin llm-app -- verify
```

Also run the relevant CI-equivalent checks required by the repository, including:

```text
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
git diff --check
```

Run configured portability and link checks when changes affect portable contracts, feature crates, documentation links, or target-specific compilation.

Do not report `cargo xtask verify` as required during this phase. The xtask migration remains deferred to its planned tooling phase.

## Acceptance criteria

* A prompt token sequence generates deterministic token output through the hosted runtime.
* Sampling is invoked inside E0 rather than by the UI.
* Prefill occurs once and decode advances through bounded scheduler quanta.
* Cancellation latency is bounded by the configured scheduler quantum and one backend step.
* Backpressure pauses and resumes generation without losing or duplicating tokens.
* One request cannot monopolize the worker indefinitely.
* Control commands, drain deadlines, maintenance, and cleanup remain responsive.
* Greedy and seeded stochastic generation are deterministic under the fake backend.
* EOS, token limit, stop sequence, cancellation, capacity exhaustion, and backend failure produce stable finish reasons.
* Every terminal path enters the cleanup state machine exactly once.
* Successful sequence cleanup releases ownership and accounting exactly once.
* Failed cleanup preserves the sequence cleanup handle.
* Failed cleanup preserves both the primary outcome and cleanup failure.
* Pending-cleanup resources remain included in capacity and memory accounting.
* Pending-cleanup resources cannot incorrectly serve new work.
* Cleanup retries are bounded and deterministic.
* A backend generation failure followed by a cleanup failure preserves both failures and does not publish the resource as released.
* Tests cover greedy, stochastic, EOS, token limit, stop, cancellation, backpressure, backend error, cleanup error, retry, drain escalation, and capacity outcomes.
* The UI is not involved in advancing token steps.
* No real model download is required for ordinary Phase 3 tests.
* Documentation accurately distinguishes completed functionality from later-phase work.

---

# Phase 4 — Prove the Candle CPU vertical slice

## Objective

Run the generation kernel against the existing Candle Llama adapter and produce real tokens.

## Work package 4.1 — Verify Candle generation semantics

Confirm and test:

- prompt token positions;
- prefill final-position logits;
- decode token/position progression;
- vocabulary-sized logits capacity;
- EOS handling;
- scalar-type compatibility;
- sequence destruction;
- model unload after generation;
- cancellation between backend calls.

Fix adapter behavior only where the shared backend contracts require it. Do not special-case Candle behavior in the generic scheduler.

## Work package 4.2 — Add a real-model smoke path

Provide a non-default smoke test or example that runs against a small supported local/Hugging Face model selected by configuration or environment.

Rules:

- ordinary CI must not download a large model;
- the smoke path must identify the exact model revision and expected architecture;
- it must not use the accidental Gemma model heading as evidence of support;
- it must verify at least one generated token, cancellation, sequence cleanup, and unload;
- failures should distinguish missing fixture/configuration from runtime failure.

## Work package 4.3 — Establish the first system measurements

Record rough local measurements for diagnosis, not optimization claims:

- model load duration;
- prompt token count;
- time to first generated token;
- decode tokens per second;
- cancellation latency;
- unload duration;
- process memory before load, after load, during generation, and after unload.

These measurements identify gross integration errors. Formal benchmark infrastructure comes later.

## Acceptance criteria

- A supported Candle Llama model produces real continuation tokens through E0.
- Generation can be cancelled and the sequence is released.
- The loaded model can be unloaded after completion or cancellation.
- No frontend polling is required to drive backend decode, except pulling bounded output to relieve backpressure.
- The smoke procedure is documented and reproducible.

---

# Phase 5 — Expose generation through `application-runtime`

## Objective

Turn the working E0 loop into a cohesive, frontend-neutral product API without making E1 a generic type maze.

## Work package 5.1 — Add a narrow public generation API

Recommended public operations:

```text
start_generation(input, settings) -> RequestId
cancel_generation(request_id)
poll_event() -> Option<ApplicationEvent>
pull_output(callback or borrowed batch API)
```

Recommended public state:

- loaded model summary;
- active request summary;
- whether generation can start;
- whether cancellation is available;
- prompt/generated usage;
- last terminal reason;
- backend/device summary.

Do not expose `RuntimeCommand`, backend sequence types, Candle tensors, Hugging Face implementation types, or raw logits to frontends.

## Work package 5.2 — Add application-level settings

Create a stable `GenerationSettings` owned by E1. Initially include:

- maximum new tokens;
- temperature;
- top-k;
- top-p;
- min-p;
- repetition penalty/window;
- seed policy;
- explicit stop tokens/sequences where supported.

Validate settings before submitting work and translate them into the sampling/runtime types.

Do not re-export every type from the `sampling` crate as the application API.

## Work package 5.3 — Encode the first direct-completion prompt

Use the resolved `HfTokenizer` to encode the user prompt into pre-reserved token storage. Apply beginning/end token policy explicitly and test it against the selected model.

This first mode must be named or documented as direct completion. It is not yet a general conversation renderer.

## Work package 5.4 — Add owned streaming decode state

The Hugging Face streaming decoder currently borrows its tokenizer. E1 needs request-local decoder state that survives across output pulls.

Implement an owned request decode session in the adapter, using a safe self-referential owner where necessary, or another design that preserves upstream decode state without decoding the full token history repeatedly.

Requirements:

- one cold-path session construction per request;
- state preserved across tokens;
- no O(n²) full-history re-decode;
- capacity-aware text sink;
- correct special-token policy;
- clear cleanup at request completion.

## Work package 5.5 — Convert token batches to text batches

`application-runtime` pulls token batches from E0, advances the request-local streaming decoder, and writes text plus terminal records to a bounded application output accumulator.

State/event separation:

- high-frequency text is pulled in batches;
- low-frequency lifecycle/error transitions remain `ApplicationEvent`s;
- terminal output is represented consistently in both the output stream and application state;
- an output-capacity stall must not corrupt decoder state or lose a token.

Validate capacity before committing a token to the decoder/output path. If the upstream decoder cannot be rolled back, retain the token until sufficient output capacity is available.

## Work package 5.6 — Resolve the single-model configuration mismatch

For the initial product, make single-model residency explicit:

- remove or hide misleading E1 `maximum_models` generality; or
- set and document E0's configured maximum as one through E1.

Do not add multi-model UI/state as part of this phase.

## Work package 5.7 — Add application-level integration tests

Using the deterministic backend/tokenizer test composition, verify:

- start/cancel state transitions;
- token-to-text streaming;
- busy and invalid-operation errors;
- output backpressure;
- terminal reasons;
- unload while idle;
- unload while generating under reject/cancel/drain policies;
- worker disconnection;
- explicit shutdown.

## Acceptance criteria

- A frontend can start and cancel generation using only E1 APIs.
- A frontend never handles logits or backend sequence state.
- Generated text arrives in bounded pulled batches.
- Application state accurately represents active generation.
- Direct completion works without duplicating orchestration in Slint.
- The E1 public surface remains narrow and documented.

---

# Phase 6 — Deliver the first usable Slint product

## Objective

Replace the lifecycle-only window with an interface that can actually exercise the product.

## Work package 6.1 — Add the minimum generation interface

The first interface should contain:

- model repository/revision controls;
- resolve, load, and unload actions;
- prompt input;
- generated output view;
- generate button;
- cancel button;
- clear-output action;
- status and terminal reason;
- prompt/generated token counts;
- visible CPU/Candle backend label.

Do not add a complex settings panel before the basic path is stable. Sensible defaults may be used with a small expandable settings section.

## Work package 6.2 — Pull output on the frame clock

Extend the current frame timer so each frame:

1. drains a bounded number of low-frequency application events;
2. pulls one bounded output batch;
3. appends text to the presentation buffer;
4. synchronizes controls from `ApplicationState`.

Do not rebuild the entire displayed transcript for every token. Batch UI updates and preserve selection/scroll behavior.

## Work package 6.3 — Guarantee cancellation and shutdown

- cancellation remains enabled while generation is active;
- closing the window initiates bounded application shutdown;
- the UI reports when cancellation is pending at a backend boundary;
- unload controls follow the active request policy;
- no worker thread is silently detached on normal application exit.

## Work package 6.4 — Add presenter tests where practical

Keep logic out of Slint callbacks. Test presentation mapping for:

- enabling/disabling controls;
- state transitions;
- text batch application;
- terminal/error messages;
- cancellation state;
- unload after generation.

Use direct runtime/application tests for behavior that does not require rendering.

## Product acceptance scenario

A user can:

1. resolve a supported immutable model revision;
2. load it on CPU;
3. enter a prompt;
4. start generation;
5. see text arrive incrementally;
6. cancel an active request;
7. start another request after cleanup;
8. unload the model;
9. close the application without an orphaned worker.

Completion of this scenario is the first major product milestone.

---

# Phase 7 — Add real chat and context planning

## Objective

Turn direct completion into honest conversation behavior and connect the existing context planner to the product.

## Work package 7.1 — Define conversation-domain input

Add frontend-neutral conversation records with:

- stable message/response-attempt identity and order;
- role;
- UTF-8 content;
- provenance;
- retention/pinning policy;
- measured, generated-use, or conservative token estimate;
- response-attempt terminal state where applicable.

Keep UI widget types and backend-specific templates out of this domain representation.

Conversation identity must also be independent from execution location. Do not store local model handles, Candle sources, provider request DTOs, peer connections, or transport state in message records.

Conversation history and planner input are different representations. `ContextEntry` values are derived from conversation state for one planning request; they are not the canonical stored message type.

User messages become committed history immediately. Assistant generation is an active response attempt while text streams. Successful completion commits a normal assistant response. Cancellation or failure preserves any partial text plus terminal provenance for inspection, but that partial response is not silently eligible as ordinary successful context on the next turn.

Regeneration creates a new assistant response attempt for the same user turn. The prior attempt remains in raw history and is marked superseded for the active-context view. General arbitrary branching is outside this phase.

## Work package 7.2 — Define prompt rendering compatibility

Introduce a prompt-rendering boundary only after the first completion path works.

The initial implementation may live as an internal E1 module. Extract a crate only when at least two independent renderers or consumers justify it.

Supported options must be explicit:

- a known built-in renderer tied to verified immutable model artifacts/profile metadata; or
- a resolved model chat template with a tested rendering implementation.

Repository naming alone is not compatibility evidence. A built-in profile must bind its claim
to immutable resolved provenance or equally strong verified metadata. Do not silently apply a
Llama template to Gemma, Qwen, Mistral, an unreviewed revision, or an unknown model.

Extend artifact resolution where required, for example with tokenizer configuration or chat-template artifacts. Missing template metadata must produce a clear compatibility result rather than guessed formatting.

For the local-model path, renderer compatibility also owns the assistant-turn termination semantics required by that model/profile: required EOS tokens, token stop sequences, textual stop suffixes, or equivalent explicit policy. Phase 7 must test prompt formatting and termination together; a correct prompt with accidental/default stop behavior is not chat compatibility.

The rendered prompt and local stop policy are request material, not conversation history. Do not persist model-specific wrappers back into semantic messages.

This renderer belongs to the current local-model path. A future hosted target may accept structured messages or require different rendering; conversation semantics must not assume that every execution target consumes the same prompt string or exposes identical stop controls.

## Work package 7.3 — Connect context planning

For each request:

1. derive typed context planning units from conversation state;
2. keep a completed historical user/assistant turn atomic so one side cannot survive context selection without the other;
3. obtain or conservatively compute token estimates for each unit;
4. reserve output tokens;
5. run deterministic selection through `context-planner` without moving chat grouping policy into the generic planner;
6. expand selected units back into semantic records and render them in conversation order;
7. tokenize the final prompt;
8. verify the actual token count against the model capacity;
9. if estimates were insufficient, deterministically remove one selected non-pinned planning unit according to the planning policy before rendering/tokenizing again;
10. stop correction after at most the initially selected droppable-unit count plus one render/tokenize attempts.

Pinned system content and the current target user must either fit or produce `PinnedBudgetExceeded`. They must never be silently dropped. Selected/dropped diagnostics remain expressed in raw conversation-record identities even when planning operates on grouped historical turns.

A correction pass that produces the same selected set is a bug. If exact rendering still exceeds capacity after all droppable content has been removed, return an explicit capacity/rendering failure. Template overhead does not authorize dropping pinned semantic content or retrying indefinitely.

## Work package 7.4 — Add conversation state to E1

`application-runtime` owns reusable conversation semantics so frontends do not duplicate them. It coordinates context planning, rendering compatibility, and model execution without absorbing their algorithms or transport implementations. Add operations for:

- submit user message;
- regenerate the last response where policy allows;
- clear conversation;
- inspect selected/dropped context diagnostics;
- cancel active response.

Submitting or regenerating while another response attempt is active is rejected rather than creating overlapping conversation mutations. Clearing while a response is active is also rejected; the caller cancels first and observes a terminal response state before clearing.

Regeneration changes the active-context view by superseding the prior response attempt, but it never erases raw conversation provenance. Failed and cancelled attempts remain inspectable and carry explicit terminal state. The default active-context policy excludes unsuccessful/superseded assistant attempts unless a later explicit retention policy says otherwise.

Persistence of conversation history may be added only after in-memory semantics are stable.

The conversation state must remain valid if the eventual execution target changes from local E0 to a peer or hosted model.

## Work package 7.5 — Expand the UI into a chat surface

Replace the direct prompt/output presentation with conversation records while preserving the lifecycle controls. Batch assistant text updates rather than creating one UI event per token.

The frontend may display an active assistant response as it grows, then its successful/cancelled/failed terminal state. It does not decide whether that attempt becomes active future context; E1 owns that semantic decision.

## Acceptance criteria

- A known supported instruct model receives the correct prompt format.
- The same compatibility profile supplies tested assistant-turn termination behavior.
- Context planning affects real generation input.
- Actual token count cannot exceed model capacity.
- Pinned content is never silently discarded.
- Completed historical user/assistant turns are selected or dropped atomically.
- Exact-token correction is bounded, deterministic, and cannot retry an unchanged selection.
- Conversation history and assistant streaming are owned by E1, not duplicated in Slint.
- `ContextEntry` is derived planner input rather than the stored conversation identity.
- Cancelled and failed partial responses remain inspectable without silently becoming ordinary successful context.
- Regeneration preserves the superseded response for provenance while the active-context view uses the replacement.
- Conversation records contain no local-backend, provider-SDK, or transport identity.
- E0 remains the local inference engine rather than becoming a remote-service abstraction.
- Built-in chat compatibility is tied to immutable verified provenance; unknown or unreviewed revisions fail explicitly.

---

# Phase 8 — Reach GGUF parity and clean up native composition

## Objective

Make the second backend usable through the same application behavior and use that pressure to define the right composition boundary.

## Work package 8.1 — Provide a correct GGUF tokenization path

Do not pair an arbitrary Hugging Face tokenizer with a GGUF model based only on vocabulary size.

Implement either:

- a dedicated tokenizer adapter backed by llama.cpp/GGUF metadata; or
- a verified external tokenizer selected through immutable model metadata.

It must support prompt encoding and stateful streaming decode under the same portable tokenization contracts.

## Work package 8.2 — Add a closed native backend selection

Use a closed enum or coarse backend service boundary for the supported native set. Avoid genericizing the entire application façade.

This selection is specifically for local E0-backed execution. Hosted providers and peer nodes are execution targets above E0 and must not become variants of a native backend enum.

The application-visible selection should include:

- backend kind;
- source kind;
- device kind;
- model compatibility summary.

The frontend should not construct Candle/GGUF source types directly.

## Work package 8.3 — Decide the composition-root split

At this point there will be real evidence from two backends. Review whether concrete Candle, GGUF, Hugging Face, redb, and host types still dominate `application-runtime`.

If local-model concerns dominate E1, extract the proven local composition as a capability beneath the application façade rather than creating another application coordinator:

```text
application-runtime   frontend-neutral use cases, state, commands, events
        ↓
local-model runtime   Candle/GGUF/HF/E0 production composition
```

Possible later transport boundary:

```text
application-api       serializable DTOs for process/network clients
```

Do not create `application-api` until a separate process or browser client is actually being implemented.

Storage or Hub composition should move with the local-model capability only when its ownership belongs there; do not create a second catch-all merely to reduce E1's dependency count.

## Work package 8.4 — Run one shared backend suite

The same generation contract tests must run against Candle and GGUF for:

- load;
- start;
- prefill;
- greedy decode;
- seeded sampling where reproducibility is defined;
- EOS/token limit;
- cancellation;
- output backpressure;
- sequence cleanup;
- unload.

Backend-specific tests may supplement but not replace the shared suite.

## Work package 8.5 — Expose backend selection in Slint

Only after parity is proven, add a backend/source selector and accurately show the selected device and format.

## Acceptance criteria

- Both backends complete the same E1 generation scenario.
- Tokenization is model-compatible for both paths.
- The UI contains no backend construction logic.
- A backend switch does not duplicate application state machines.
- The composition decision is documented in an ADR.

---

# Phase 9 — Simplify the architecture using integration evidence

## Objective

Address structural concerns after the product loop has exposed which boundaries are real.

## Work package 9.1 — Replace the absolute F1 rule with an approved DAG

Review actual dependencies needed by tokenization, context planning, sampling, task graph, prompt rendering, and workflows.

Adopt these principles:

- the graph must remain acyclic;
- one feature may depend on another when the lower feature genuinely owns a stable concept;
- shared F0 types must cross a real engine/backend boundary or have multiple stable consumers;
- unrelated domain vocabulary must not be pushed into `domain-contracts` merely to satisfy a tier table;
- dependency direction is reviewed explicitly.

Consider, but do not automatically perform, a split such as:

```text
foundation-types
inference-contracts
```

Only split if current `domain-contracts` changes for unrelated reasons often enough to justify it.

## Work package 9.2 — Narrow `application-runtime`

- keep the extracted `corrective-workflow` capability outside E1 and do not re-export its internals from the application façade;
- make generation/model lifecycle the primary documented façade;
- separate local-model composition if Phase 8 justified it;
- keep new stateful capability engines outside E1 when their lifecycle/reuse boundary is independently proven.

## Work package 9.3 — Split oversized modules internally

Candidate splits:

```text
task-graph/
  graph.rs
  validation.rs
  artifact_flow.rs
  attempt.rs
  state.rs
  error.rs

inference-runtime/runtime/
  model_registry.rs
  request_registry.rs
  generation.rs
  transaction.rs
  operations.rs
  shutdown.rs
```

Use `pub(crate)` or `pub(super)` for internal helpers rather than accidental broad `pub` visibility.

## Work package 9.4 — Convert the maintenance runner to `xtask`

After the product path is stable:

```text
Cargo.toml                  virtual workspace
tools/xtask/Cargo.toml
tools/xtask/src/main.rs
.cargo/config.toml
```

Recommended aliases:

```toml
[alias]
xtask = "run -p xtask --"
bench-sampling = "bench -p sampling --bench sampling_pipeline"
```

Keep custom Rust code for architecture validation and other repository-specific logic. Use Cargo directly for ordinary `fmt`, `check`, `test`, `clippy`, and simple benchmark selection.

Remove the misleading product-like root binary name.

## Work package 9.5 — Review lint policy

Keep strong lints, but review whether every `clippy::nursery` warning should block CI across toolchain upgrades. Prefer an explicit stable set for mandatory policy and enable exploratory lints without necessarily denying them.

## Acceptance criteria

- Architecture rules describe a real DAG rather than a purity diagram.
- `domain-contracts` has a clear inclusion rule.
- E1 has a narrow, coherent public API.
- Large modules are split by invariant/responsibility, not arbitrary line count.
- `cargo xtask architecture` enforces the current policy.
- Simple commands are no longer needlessly reimplemented.

---

# Phase 10 — Build a meaningful performance program

## Objective

Measure product behavior before applying low-level optimization doctrine.

## Work package 10.1 — Expand component benchmarks

Keep component benchmarks beside their crates.

For sampling, cover:

- greedy;
- default top-k/top-p;
- min-p;
- repetition penalty with varied histories;
- approximately 8k, 32k, and 128k vocabularies;
- sampler-only timing with setup outside measurement;
- full restore-plus-sample pipeline as a separately named benchmark;
- stop matching.

Add appropriate component benchmarks for:

- tokenizer encode;
- streaming decode;
- context planning;
- output accumulator push/pull;
- backend prefill;
- backend decode.

## Work package 10.2 — Add a cross-crate benchmark package

Create a dedicated workspace member such as:

```text
benchmarks/runtime
```

It may depend on public runtime/application APIs and controlled fixtures. Measure:

- time to first token;
- steady-state tokens per second;
- prompt prefill throughput;
- cancellation latency;
- output backpressure behavior;
- model load/unload latency;
- peak/resident memory;
- repeated load/generate/unload stability;
- Candle versus GGUF on comparable models where meaningful.

## Work package 10.3 — Separate CI compilation from controlled baselines

Shared CI should compile benchmarks and catch API breakage. Stable performance baselines should run on named controlled hardware with:

- CPU/GPU model;
- OS/kernel;
- power mode;
- thread count;
- model/revision;
- prompt length;
- generation settings;
- build profile and features.

Do not fail ordinary CI because a shared runner was temporarily slower.

## Work package 10.4 — Optimize only measured bottlenecks

Use profiling and generated-code inspection before adding:

- `#[inline(always)]`;
- custom unsafe code;
- manual SIMD;
- alternative collections;
- lock-free structures;
- data-layout rewrites;
- custom allocators.

Preserve the existing zero-allocation project-owned hot-path goal where it is already useful, but report upstream adapter allocations honestly.

## Acceptance criteria

- Benchmarks distinguish component and system behavior.
- The sampling benchmark name states whether input restoration is measured.
- TTFT and decode throughput exist for the real product path.
- Memory returns to an expected range after unload.
- Optimization changes cite a baseline and resulting measurement.

---

# Phase 11 — Add GPU execution

## Objective

Introduce GPU support as an adapter/device capability without redesigning the application or weakening CPU fallback.

## Work package 11.1 — Define supported device matrix

Select explicit targets, for example:

- Candle CPU;
- Candle CUDA on supported Linux/Windows environments;
- Candle Metal where supported;
- llama.cpp GPU offload options where the chosen crate/build supports them.

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

This is not Phase 12 and it does not depend on GPU support. It may begin when conversation semantics are stable and a real second execution/deployment need proves the coarse seam; GGUF/composition work in Phases 8–9 is likely to provide useful evidence, but Phase 11 is not a prerequisite.

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
- implementing GGUF UI selection before tokenizer and generation parity;
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

This milestone does **not** require GGUF UI parity, general chat templates, multiple resident models, remote clients, or GPU support.

# 15. Traceability from `analyzer.md`

| Analyzer finding | Addressed in |
|---|---|
| Central generation loop absent | Phases 3–6 |
| `application-runtime` is a valid façade but concrete/growing | Architecture closure and Phases 5, 8, 9 |
| Candle/HF/redb lock-in at E1 | Phase 8 composition review |
| Corrective workflow dominates E1 | Architecture closure extraction; Phase 9 keeps E1 narrow |
| Single-model state conflicts with `maximum_models` | Phase 5.6 |
| Model-load cleanup bypass | Phase 2.1 |
| Sequence/request commit not rollback-safe | Phase 2.2–2.3 |
| Folder taxonomy is unconventional but understandable | Closed by ADR-0009 before Phase 7 |
| F1-to-F1 ban is too absolute | Phase 9.1 |
| `domain-contracts` junk-drawer pressure | Phase 9.1 |
| Sampling benchmark placement is correct | Decision 3.8 and Phase 10 |
| Sampling benchmark coverage is narrow | Phase 10.1 |
| Root runner is an xtask in disguise | Phase 9.4 |
| Wrapper commands reimplement Cargo | Phase 9.4 |
| `cargo test --all-targets` selects benches | Phase 1.1 |
| Validator not required in CI | Phase 1.1 |
| Validator ignores external dependencies | Phase 1.2 |
| Unknown paths become applications | Phase 1.2 |
| Dependency kind is ignored | Phase 1.2 |
| Validator test matrix is partial | Phase 1.2 |
| Large source modules | Phases 2.5 and 9.3 |
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
Cargo.toml                        # virtual workspace after xtask migration
.cargo/config.toml
LICENSE-MIT
LICENSE-APACHE
deny.toml

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
    gguf-backend
    hf-hub
    hf-tokenizer
    redb-storage
    # optional GGUF tokenizer adapter if required
  runtime/
    inference-runtime
    corrective-workflow
    application-runtime
    # native-runtime only if Phase 8 proves the split
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
8. additional backends and devices;
9. speculative generality.

The project should preserve its strong low-level discipline, but every new abstraction must now justify itself against a running generation loop.
