> **Historical document.** This recovered draft does not describe the current workspace or roadmap. It is retained only as architecture history. Follow the [active execution plan](../agent/execution/execution-plan.md), [current architecture](../architecture.md), and [current implementation status](implementation-status.md).

NOTE: Due to loss of context the implementation plan was recovered from an earlier draft of a chat conversation.

---

I’ll treat this as the reviewed implementation plan only. The concrete Rust syntax for `domain-contracts` will be deferred until the next prompt, but the complete API surface and ownership model are defined below.

# Revised Workspace and Implementation Plan

## 1. Architectural constraints

The workspace will follow four physical layers:

```text
crates/
├── features/
├── adapters/
├── engines/
└── apps/
```

These directories are architectural boundaries, not Rust dependency mechanisms by themselves. The dependency graph will also be enforced through workspace metadata, CI checks, and crate-level dependency policies.

The permitted dependency direction is:

```text
apps
  ↓
engines
  ↓
adapters and features
  ↓
features
```

Additional restrictions:

* Feature crates may depend only on other explicitly lower-level feature crates.
* Adapter crates may depend on feature crates, but never on engines or apps.
* Engine crates may depend on features and adapter contracts.
* Application crates may compose all lower layers.
* Backends must never import runtime or UI types.
* No crate may depend upward.
* Cross-adapter dependencies are prohibited.
* Circular dependencies are architectural failures.

This preserves the required directed acyclic graph while avoiding both a monolithic core crate and unnecessary one-type micro-crates.

---

# 2. Revised workspace structure

```text
workspace/
├── Cargo.toml
├── rust-toolchain.toml
├── deny.toml
├── crates/
│   ├── features/
│   │   ├── domain-contracts/
│   │   ├── tokenization/
│   │   ├── context-planner/
│   │   ├── sampling/
│   │   └── task-graph/
│   │
│   ├── adapters/
│   │   ├── candle-backend/
│   │   ├── gguf-backend/
│   │   ├── hf-tokenizer/
│   │   ├── hf-hub/
│   │   ├── redb-storage/
│   │   └── host-runtime/
│   │
│   ├── engines/
│   │   ├── inference-runtime/
│   │   └── task-orchestrator/
│   │
│   └── apps/
│       ├── desktop-slint/
│       └── cli-runner/
│
├── tests/
│   ├── lifecycle/
│   ├── compatibility/
│   └── performance/
│
├── benches/
│   ├── tokenization/
│   ├── context-planning/
│   ├── sampling/
│   └── generation/
│
└── docs/
    ├── architecture.md
    ├── lifecycle.md
    ├── memory-model.md
    └── backend-contract.md
```

The workspace remains shallow. Folder placement communicates architectural role, while crate names remain concise enough to use naturally in source code.

---

# 3. Feature crates

## `domain-contracts`

This is the stable vocabulary and backend boundary.

It contains cohesive types related to:

* identifiers;
* model metadata;
* model lifecycle;
* generation requests;
* sequence ownership;
* memory requirements;
* execution capabilities;
* backend errors;
* cancellation;
* inference events;
* backend traits.

It should not contain:

* model implementations;
* tokenization algorithms;
* chat templates;
* scheduler policy;
* filesystem paths tied to one OS;
* channels;
* thread handles;
* database records;
* Slint types;
* Candle or llama.cpp types.

Target configuration:

* `no_std` by default;
* optional `alloc`;
* no mandatory serialization framework;
* no runtime executor;
* no dynamic error objects;
* no `String`-based error classification;
* no dynamic dispatch in per-token operations.

This crate consolidates related contracts so identifiers, metadata, memory descriptors, generation state, and backend interfaces do not become separate micro-crates.

## `tokenization`

Owns token-oriented algorithms and abstractions:

* token sequence buffers;
* incremental decode state;
* token sink contracts;
* text sink contracts;
* special token handling;
* tokenizer capability descriptions;
* tokenizer error enums;
* tokenizer traits.

The tokenizer API will use generic sink parameters. Encoding, decoding, and incremental emission will be statically dispatched.

No tokenizer hot-path method will accept:

* `dyn TokenSink`;
* `dyn TextSink`;
* boxed callbacks;
* heap-allocating iterators;
* implicit output vectors.

Callers provide pre-allocated output storage.

The Hugging Face tokenizer implementation remains in `adapters/hf-tokenizer`.

## `context-planner`

Owns:

* typed context entries;
* provenance;
* priority;
* persistence policy;
* token estimates;
* selected and rejected entry sets;
* reserved generation budget;
* deterministic eviction policy;
* context-plan validation.

It does not render prompts and does not know about any backend.

Its outputs reference existing content rather than clone message bodies.

Planning may allocate during cold-path setup where necessary, but the repeated planning path should support caller-owned scratch storage and bounded collections.

## `sampling`

Owns backend-independent sampling logic:

* temperature;
* top-k;
* top-p;
* min-p;
* repetition penalties;
* token suppression;
* stop matching;
* seeded random selection;
* constrained-token masks.

The sampling API operates over caller-provided mutable logit slices and pre-allocated scratch memory.

No sample step allocates.

Sampling configuration is immutable or prepared before generation begins. Runtime state such as repetition history, random state, and stop-matcher state is stored in a pre-allocated generation workspace.

## `task-graph`

Owns generic allocation-free bounded directed-work mechanics:

* task node identifiers;
* dependency integrity and acyclicity;
* caller-owned opaque operation metadata;
* bounded attempt and task status transitions;
* deterministic ready discovery;
* cancellation and blocked descendants;
* identity-only external/producer/consumer artifact provenance.

It does not own model/backend selection, token or byte budgets, artifact media or
roles, corrective operations, threads, models, compilers, or inference calls.
Tasks may produce zero, one, or many artifacts; capability-specific constraints
remain above this graph boundary.

This keeps orchestration representation separate from orchestration execution.

---

# 4. Adapter crates

## `candle-backend`

Implements the backend contracts using Candle.

Responsibilities:

* model metadata inspection;
* weight loading;
* device selection;
* tensor allocation;
* model-family-specific execution;
* KV-cache creation;
* prefill;
* decode;
* backend synchronization;
* deterministic resource release;
* footprint reporting.

Internal allocations are permitted during model loading and sequence creation.

The decode loop must not allocate. All reusable tensors, logits buffers, temporary activation storage where controllable, and sequence state must be prepared beforehand.

Backend-specific types remain private.

## `gguf-backend`

Implements the same contracts through the GGUF/llama.cpp boundary.

Responsibilities include:

* FFI containment;
* native resource lifetime management;
* context creation;
* batch buffer ownership;
* prefill and decode;
* cancellation polling;
* synchronization;
* resource destruction.

All unsafe code stays inside this adapter and must have explicit soundness arguments. It must not leak raw native pointers into engines or applications. This follows the requirement that unsafe code remain narrowly contained behind safe abstractions.

## `hf-tokenizer`

Adapts Hugging Face Tokenizers to the generic tokenizer contracts.

Any temporary allocations forced by the upstream library must be isolated and measured. The adapter should provide a prepared or reusable encoding path where the upstream API permits it.

The engine does not depend directly on Hugging Face types.

## `hf-hub`

Owns:

* remote model resolution;
* local cache lookup;
* downloading;
* integrity verification;
* revision selection;
* manifest construction.

It produces a backend-neutral model artifact description.

It does not load a model into memory.

## `redb-storage`

Owns durable user-space persistence:

* conversations;
* application configuration;
* model catalogue metadata;
* prompt templates;
* task histories;
* summaries;
* download state.

Persistent schemas are explicitly versioned and converted into domain values.

Database records are not reused as engine-domain types.

## `host-runtime`

Contains standard-library infrastructure shared by desktop applications and engines:

* thread creation;
* bounded queues;
* clocks;
* cancellation primitives;
* filesystem-backed artifact access;
* host memory observations;
* thread affinity where supported.

`flume` may be used internally here.

No `flume::Sender` or `Receiver` enters the feature-level public API.

---

# 5. Engine crates

## `inference-runtime`

This is the sole owner of loaded backend instances during normal application execution.

Responsibilities:

* model registry;
* model lifecycle state machine;
* admission control;
* active sequence tracking;
* request scheduling;
* cancellation;
* draining;
* model unloading;
* memory-budget enforcement;
* event batching;
* backend invocation.

It does not:

* download models;
* store conversations;
* render UI;
* implement tokenization algorithms;
* implement tensor operations;
* own task-graph semantics.

The lifecycle states will be:

```text
Absent
Loading
Ready
Active
Draining
Unloading
Failed
```

Transitions are explicit and validated.

A model handle contains only identity and generation information. It does not own or reference the backend model.

The runtime registry owns model instances exclusively.

### Model unloading policy

The runtime supports three explicit policies:

* reject unloading while active;
* cancel active work and unload;
* drain active work and unload.

Logical unload completion is distinct from operating-system or device-driver memory reclamation.

The runtime guarantees:

* the model is unreachable through public handles;
* no sequence remains attached;
* all backend operations have completed;
* the backend resource owner has been dropped.

It does not claim that CUDA, Metal, or another driver immediately returns all reserved memory to the operating system.

## `task-orchestrator`

Consumes generic `task-graph` mechanics through a higher workflow/capability schema
and delegates execution to typed targets.

Responsibilities:

* dependency resolution;
* ready-task discovery;
* artifact routing;
* model policy selection;
* corrective-review cycles;
* retry accounting;
* compiler or validator adapter invocation;
* workflow completion.

It should initially use an explicit graph executor rather than ECS.

A future Bevy-based scheduler may be implemented behind an orchestration boundary, but Bevy ECS will not become a dependency of:

* `domain-contracts`;
* `inference-runtime`;
* inference backends;
* tokenization;
* sampling;
* context planning.

This prevents an optional scheduling strategy from becoming the workspace’s central ownership model.

---

# 6. Application crates

## `desktop-slint`

Owns:

* Slint components;
* window lifecycle;
* operating-system dialogs;
* user configuration;
* model selection;
* command submission;
* rendering streamed output;
* user cancellation;
* desktop-specific notifications.

It sends coarse commands to `inference-runtime`.

It receives batched output events. It does not receive an event for every internal tensor operation or every intermediate sampling stage.

The UI cannot access backend instances.

## `cli-runner`

Provides a minimal execution environment for:

* lifecycle testing;
* model compatibility testing;
* benchmarks;
* scripted generation;
* backend diagnosis;
* regression reproduction.

The CLI should be implemented before the Slint interface because it exposes runtime correctness without event-loop or rendering complexity.

---

# 7. `domain-contracts` API boundary

The concrete Rust definitions will be written after this plan is reviewed. The crate will contain the following exact conceptual interfaces.

## Identifier types

Distinct transparent identifiers will exist for:

* model;
* model instance generation;
* sequence;
* generation request;
* task;
* artifact;
* backend;
* device.

Identifiers will not be interchangeable aliases.

Model handles will combine:

* model identity;
* runtime generation.

The generation field prevents a stale handle from addressing a newly loaded model that reused the same logical identifier.

## Model source description

The contract will represent model inputs without embedding desktop filesystem assumptions.

Supported source concepts:

* immutable memory region;
* named artifact reference;
* backend-owned source descriptor;
* collection of weight artifacts plus metadata.

Filesystem paths and Hugging Face repository identifiers belong in adapter-layer conversion types.

## Backend capability description

A backend reports immutable capabilities such as:

* supported model architecture;
* supported scalar or quantization formats;
* maximum sequence length;
* batch support;
* concurrent sequence support;
* device category;
* prefill support;
* incremental decode support;
* cache reuse support;
* cancellation granularity.

The engine uses these capabilities for admission control and scheduling.

## Memory planning types

The boundary includes:

* static model weight footprint;
* estimated host working memory;
* estimated device working memory;
* KV-cache bytes per token;
* temporary workspace requirement;
* required alignment;
* maximum sequence count;
* maximum batch size.

Backend-provided estimates are explicit estimates, not guarantees.

Observed resource usage may be reported separately when available.

## Model loader contract

The model loader is a coarse-grained cold-path boundary.

Its responsibilities:

* validate a source;
* calculate or report a load plan;
* load a model using a provided configuration;
* return a concrete backend model instance or a typed backend error.

Dynamic dispatch may be used at this boundary by the host runtime when selecting among backend implementations because model loading is coarse-grained and not part of the token loop.

The backend implementation itself remains concrete internally.

## Loaded model contract

A loaded model provides:

* immutable metadata;
* backend and device identity;
* resource footprint;
* sequence-creation validation;
* sequence creation;
* explicit backend synchronization;
* shutdown preparation where required.

A loaded model does not expose its tensors or native handles.

Creating a sequence is a cold-path operation and may allocate according to the previously accepted sequence plan.

## Sequence contract

A sequence owns all request-specific inference state:

* KV cache;
* position state;
* backend scratch buffers;
* logits output;
* batch descriptors;
* backend sequence handle.

The sequence API exposes:

* reset;
* prefill;
* one-step decode;
* cancellation observation;
* completion state;
* token position;
* cache usage.

Prefill and decode methods are statically dispatched inside the backend worker.

No `dyn Trait` appears in their per-token parameters.

## Prefill contract

Prefill accepts:

* a borrowed flat token slice;
* an explicit position or sequence offset;
* a borrowed mutable workspace prepared earlier.

It returns compact metadata describing:

* tokens consumed;
* resulting sequence position;
* whether logits are available;
* backend completion status.

It must not construct an owned token vector.

## Decode contract

Decode accepts:

* the next token or prepared token batch;
* a mutable caller-owned logits slice;
* a mutable prepared workspace.

It returns compact decode metadata.

The call performs no heap allocation.

The logits buffer size and scalar representation are validated before entering the repeated loop.

## Generation workspace contract

Before generation begins, the runtime prepares a workspace containing:

* logits;
* filtered-logit scratch;
* candidate indices;
* repetition history;
* stop-matcher state;
* decode byte buffer;
* event batching buffer;
* backend-specific scratch storage where allowed by the contract.

The workspace is reused for the complete request.

No generation step creates a new vector, string, box, reference-counted pointer, or channel message allocation.

## Cancellation contract

Cancellation is represented as a small shared state with documented memory ordering.

The contract supports:

* cancellation requested;
* cancellation observed;
* optional reason code.

Cancellation polling is cheap enough to occur between decode steps.

No string allocation is used for cancellation reasons.

## Event contract

Inference events are typed values representing:

* request accepted;
* prefill completed;
* text chunk produced;
* usage update;
* request completed;
* request cancelled;
* request failed.

Text events borrow or reference a runtime-owned batch buffer during internal processing. The host adapter may copy the chunk when crossing into the UI event loop, but the inference hot path does not allocate one message per token.

## Error model

Errors are dense enums separated by domain:

* model loading;
* unsupported capability;
* invalid configuration;
* insufficient memory;
* invalid sequence state;
* backend execution;
* cancellation;
* synchronization;
* resource shutdown.

Backend-native details may be converted into:

* stable category;
* backend-specific numeric code;
* optional cold-path diagnostic text under an `alloc` or `std` feature.

Control flow never depends on parsing diagnostic strings.

This follows the project requirement for typed error propagation and avoidance of unchecked failure handling.

---

# 8. Static dispatch strategy

Static dispatch is mandatory within repeated operations:

* token encoding sink writes;
* incremental decoding;
* context selection;
* prompt rendering;
* logit processing;
* sampling;
* prefill;
* token decode;
* stop matching;
* generation event accumulation.

These boundaries will use generics, associated types, or concrete backend worker types.

Dynamic dispatch is permitted only at coarse boundaries where its cost is negligible relative to the operation:

* backend selection;
* model loading request routing;
* storage-provider selection;
* application service composition;
* optional task-executor selection.

Even at those boundaries, dynamic dispatch is not mandatory. An application may instead use an enum over compiled backend implementations.

This applies the project rule prohibiting dynamic dispatch in hot paths while avoiding excessive generic propagation across the entire desktop application.

---

# 9. Zero-allocation inference strategy

The zero-allocation requirement applies after a generation request enters its prepared state.

The lifecycle is divided into two phases.

## Preparation phase

Allocations are permitted for:

* loading weights;
* creating tensor storage;
* initializing a sequence;
* calculating buffer sizes;
* allocating logits and scratch buffers;
* preparing the sampler;
* reserving event buffers;
* compiling templates;
* tokenizing the initial prompt where the selected tokenizer requires allocation.

Every allocation needed for repeated inference must complete here.

## Execution phase

No heap allocation is permitted during:

* model prefill loops;
* decode loops;
* logit processing;
* sampling;
* repetition tracking;
* stop-sequence detection;
* incremental UTF-8 assembly;
* runtime event accumulation.

Execution operates on:

* flat slices;
* fixed-capacity buffers;
* pre-sized vectors whose capacity cannot grow;
* backend-owned tensor arenas;
* reusable batch descriptors.

Debug builds will include capacity invariants before generation starts.

Performance tests will use an allocation-counting test allocator to fail any test that allocates inside the measured generation region.

The design follows the requirement for flat contiguous memory, predictable iteration, and allocation-free hot paths.

---

# 10. User-space floating-point and acceleration policy

The desktop implementation will freely use:

* `f32` and backend-required scalar formats;
* LLVM auto-vectorization;
* architecture-specific SIMD through backend implementations;
* CUDA;
* Metal;
* other safe user-space acceleration APIs.

The runtime will not manually save or restore FPU or SIMD state.

It will not perform MMIO.

It will not contain volatile register access.

Raw device interaction belongs to upstream user-space drivers or narrowly scoped backend FFI adapters.

Hardware-specific optimization will be driven by measurement. Iterator-based implementations remain the default unless profiling demonstrates a failed optimization or measurable bottleneck, consistent with the project’s profiling and inlining rules.

---

# 11. Dependency enforcement

The directory layout alone is insufficient. The following enforcement will be added:

* workspace dependency declarations centralized at the root;
* `cargo-deny` rules for duplicate and prohibited dependencies;
* CI validation of the crate dependency graph;
* a prohibited-edge configuration;
* feature-matrix compilation;
* `no_std` checks for feature crates;
* adapter isolation tests;
* public API checks preventing backend types from leaking;
* optional semver API checking for `domain-contracts`.

Examples of prohibited edges:

```text
features → adapters
features → engines
features → apps

adapters → engines
adapters → apps

engines → apps

candle-backend → gguf-backend
gguf-backend → candle-backend
redb-storage → inference-runtime
```

---

# 12. Implementation sequence

## Phase 1: contracts and invariants

Finalize:

* ownership rules;
* lifecycle transitions;
* memory descriptors;
* backend capabilities;
* sequence semantics;
* error taxonomy;
* cancellation semantics;
* zero-allocation boundary.

Then implement `domain-contracts`.

## Phase 2: portable feature layer

Implement:

* tokenization contracts;
* context planner;
* sampling;
* task graph.

All crates receive focused unit tests and `no_std` builds where applicable.

## Phase 3: reference backend

Implement Candle CPU inference first.

Required lifecycle test:

```text
load
→ create two sequences
→ prefill independently
→ interleave decode
→ cancel one sequence
→ complete the other
→ release both sequences
→ unload model
```

No UI or database is introduced before this passes.

## Phase 4: runtime ownership

Implement `inference-runtime` with:

* exclusive model registry ownership;
* bounded command queues;
* bounded event queues;
* admission control;
* cancellation;
* draining;
* unloading.

The runtime is tested through the CLI.

## Phase 5: desktop adapters

Add:

* Hugging Face tokenizer;
* Hugging Face Hub resolution;
* `redb` persistence;
* Slint application.

## Phase 6: additional backend

Implement the GGUF adapter against the same compatibility suite.

No GGUF-specific behavior may require changes to context planning, sampling, task graphs, or UI state.

## Phase 7: orchestration

Implement typed corrective workflows:

```text
draft
→ compile or validate
→ normalize diagnostics
→ review
→ revise
→ validate again
```

Artifacts are passed by identifier. Full transcripts are not duplicated into every task.

## Phase 8: optimization and enforcement

Add:

* allocation detection;
* latency benchmarks;
* throughput benchmarks;
* cache-footprint analysis;
* backend memory accounting;
* queue backpressure tests;
* cancellation latency tests;
* unload determinism tests.

Optimization changes require benchmark evidence.

---

# 13. Acceptance criteria before application development

The architecture is ready for Slint application work only when all of the following hold:

1. Backends can be selected without exposing backend-native types.
2. A model can be loaded and unloaded through an explicit state machine.
3. Stale handles are rejected through generation validation.
4. Two sequences can execute independently against one loaded model.
5. Cancellation does not corrupt another sequence.
6. Decode performs no heap allocation after preparation.
7. Context input is fully inspectable before rendering.
8. Buffer exhaustion produces typed errors rather than implicit growth.
9. Backend errors are normalized without string parsing.
10. The CLI can reproduce every lifecycle transition.
11. Feature crates compile without desktop adapters.
12. The dependency graph contains no upward or cross-adapter edges.

The next implementation step, after this plan is approved, is the complete compile-ready `domain-contracts` crate API with concrete generic trait definitions, associated types, lifecycle types, typed errors, and feature gating.

This plan incorporates the static-dispatch, consolidated workspace, user-space acceleration, and zero-allocation requirements without allowing the desktop runtime to contaminate the portable contracts.



The implementation plan is thoroughly approved. The phase-gated allocation strategy and state-machine-driven lifecycle management provide a solid foundation for execution.

Before you generate the Rust syntax for Phase 1 (`domain-contracts`), you must incorporate safeguards for four specific architectural danger zones:

### 1. Mandatory Timeout on "Drain Active Work" Policy
- **Risk:** Unloading via `drain` will block indefinitely if a sequence hangs, gets stuck in a multi-agent loop, or encounters a slow consumer.
- **Required Guardrail:** The `drain` policy must enforce a configurable hard timeout (e.g., N milliseconds). If active work fails to complete within the window, the state machine must automatically escalate to force-canceling active requests to guarantee deterministic memory reclamation.

### 2. Pre-Allocated Buffer Bounds and Graceful Yielding
- **Risk:** Because zero-allocation hot paths strictly forbid dynamic buffer resizing[cite: 4], attempting to write tokens or logits beyond pre-allocated capacity will panic or silently overwrite memory.
- **Required Guardrail:** The `domain-contracts` error taxonomy must include a `CapacityExhausted` variant[cite: 3, 4]. All sequence state functions must validate available slice capacity before executing a decode pass, gracefully halting generation with `FinishReason::BufferExhausted` rather than failing unchecked or crashing[cite: 3].

### 3. Preventing Monomorphization Bloat (I-Cache Eviction)
- **Risk:** Over-using generic type parameters in the core generation loop causes LLVM to generate a unique copy of the entire hot loop for every permutation of sampler, tokenizer, and stop-matcher. This bloats binary size and triggers instruction cache (I-Cache) evictions that degrade performance[cite: 4].
- **Required Guardrail:** Limit generic type parameters in hot functions strictly to essential types (e.g., `BackendSequence`)[cite: 4]. Process auxiliary operations like sampling or stopping criteria over flat, pre-allocated mutable slices (`&mut [f32]`) or concrete function pointers[cite: 4].

### 4. Slint UI Event Churn Mitigation
- **Risk:** High-frequency event pushes from the inference engine can flood Slint's event loop with token updates, consuming CPU cycles on layout recalculations and starving the inference runtime.
- **Required Guardrail:** The `host-runtime` adapter must implement frame-aligned or pull-based batching. The UI adapter will pull accumulated text chunks from a thread-safe ring/bounded channel on its native frame clock (e.g., 60 Hz / 16 ms ticks), decoupling token production frequency from rendering frequency.

Please incorporate these guardrails into the conceptual design and generate the complete, compile-ready Rust syntax for `domain-contracts`.

You're free to execute the plan. Please remain thorough, read the docs and provide complete implementations.
