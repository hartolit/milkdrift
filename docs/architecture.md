# Architectural Blueprint: The Modular Philosophy

This document is a reusable architecture blueprint for Rust workspaces and similar modular systems. It teaches how to choose a workspace topology, what each layer is responsible for, and which coupling failures the structure is intended to prevent.

Project-specific crate names, exact dependency edges, temporary constraints, supported backends, and product state belong in the project's applied architecture and ADRs. The architecture selection below is the one intentionally project-specific field in this reusable blueprint: when the blueprint is reused, select the model that fits that project.

## Core ideology: defeating the monolith

A modular architecture exists to keep **independent reasons to change independent**. A monolith is not merely a large crate; it is a unit that accumulates unrelated domain logic, infrastructure, orchestration, and presentation until changes in one concern force knowledge of the others.

The opposite failure is excessive fragmentation. A workspace with dozens of tiny crates that have no independent ownership, reuse, portability, or lifecycle can be harder to understand than a cohesive crate with internal modules.

The goal is therefore not maximum crate count. The goal is **cohesive ownership with explicit dependency direction**:

- domain logic can be understood without knowing the application shell;
- infrastructure can be replaced without contaminating portable logic;
- stateful orchestration has one clear owner;
- frontends compose behavior instead of reimplementing it;
- dependency cycles are treated as design failures rather than solved with global state;
- crate boundaries exist for a reason that survives refactoring.

---

## Active architecture selection

*Mark the architecture model used by the current project with `[x]`. Change only this selection when reusing the blueprint for another project; specialize the concrete graph in that project's architecture document.*

- [ ] **Model A: Standard Workspace** — focused small-to-medium systems
- [x] **Model B: Layered Workspace** — large or infrastructure-heavy systems

### Choosing a model

| Signal | Model A: Standard Workspace | Model B: Layered Workspace |
|---|---|---|
| Product scope | Focused application or tool | Multiple domains or substantial product surface |
| External infrastructure | Small or tightly bounded | Several vendors, runtimes, databases, networks, FFI, or OS services |
| Execution environments | Usually one primary runner | Multiple frontends, hosts, devices, or deployment modes |
| Portable logic | Useful, but not the dominant design pressure | Must remain isolated from infrastructure and host concerns |
| Orchestration | One cohesive engine is usually enough | Resource ownership and application use cases may need distinct engine layers |
| Cost of extra boundaries | Likely greater than the benefit | Paid back by dependency quarantine and replaceability |

Prefer Model A when it is sufficient. Choose Model B when infrastructure pressure, portability, multiple execution boundaries, or complex lifecycle ownership would otherwise leak through the whole workspace.

Do not choose Model B because the project merely feels "important." Layers are useful only when they isolate real change boundaries.

---

## Model A: Standard Workspace

*Optimized for focused applications, single-purpose tools, compact services, games with a cohesive runtime, and bare-metal systems where additional infrastructure layers would add ceremony without isolating a real dependency boundary.*

### Crate ontology

#### Feature crates: reusable building blocks

Feature crates own a coherent capability or domain: a parser, scheduler, protocol, renderer subsystem, storage format, math kernel, or domain model.

A feature crate should:

- expose a narrow public API around its domain;
- avoid depending on the application runner;
- avoid importing unrelated sibling domains merely for convenience;
- remain independently testable;
- be designed so that reuse outside the current runner is plausible, even if publication is never planned.

A feature may depend on a genuinely lower-level feature when that dependency represents a stable concept. The important rule is a reviewed acyclic graph, not a blanket prohibition on every feature-to-feature edge.

#### Engine or core crates: composition and state ownership

The engine connects features into a running system. It owns cross-feature orchestration, shared runtime state, scheduling, lifecycle coordination, and other concerns that only exist when the product is assembled.

The engine should **not** become the place where every feature implementation lives. When a body of logic has a coherent API, independent tests, and a reason to exist without the engine, it belongs in a feature crate or internal module with that ownership.

#### App or runner crates: environment boundary

Runners are thin execution vectors such as a desktop binary, server process, CLI, firmware entry point, or test host.

They own environment-specific concerns such as:

- process or device startup;
- configuration acquisition;
- command-line or environment parsing;
- event-loop integration;
- top-level logging and exit reporting;
- construction of the engine and concrete dependencies.

They should not duplicate feature algorithms or engine state machines.

### Typical dependency shape

```text
app / runner
     ↓
   engine
     ↓
 feature crates
```

Additional downward feature edges are allowed when the resulting graph remains cohesive and acyclic.

### When Model A stops fitting

Move toward Model B when the engine starts importing many unrelated vendor SDKs, databases, network clients, OS integrations, or native runtimes; when several frontends need the same use cases; or when portable domain logic can no longer compile independently because infrastructure types have become part of its vocabulary.

---

## Model B: Layered Workspace

*Optimized for expansive applications that need strict infrastructure quarantine, multiple execution environments, complex resource ownership, or portable logic that must survive changes in vendor/runtime implementation.*

Model B separates **what the system means**, **how external systems implement it**, **who owns runtime state**, and **where the process is presented to the environment**.

### Crate ontology

#### Feature crates: pure logic and contracts

Feature crates own domain vocabulary, algorithms, and contracts that do not require application or infrastructure knowledge.

Typical responsibilities include:

- strongly typed domain identifiers and state;
- pure validation and planning;
- tokenization/sampling-style algorithms;
- scheduling or graph algorithms without host I/O;
- traits or data contracts implemented by infrastructure;
- bounded data structures whose semantics are infrastructure-independent.

Features should be portable where the domain permits it. `no_std` is valuable when it serves real targets, but it is not a purity badge and does not by itself prove allocation-free behavior.

A feature must not depend upward on engines, applications, UI frameworks, vendor runtimes, databases, filesystem/network clients, or OS transport implementations. Dependencies between feature crates are acceptable only when they express a real lower-level relationship and preserve an explicit DAG; unrelated features should not become mutually aware.

#### Adapter crates: infrastructure quarantine

Adapters translate external systems into project-owned contracts. They contain the dependencies whose types and lifecycle rules should not infect portable logic.

Typical adapter concerns include:

- vendor inference or graphics runtimes;
- C/C++ FFI;
- filesystem and network clients;
- databases and persistence engines;
- platform channels, clocks, and OS services;
- serialization formats tied to external systems;
- framework-specific integration that implements a lower-level contract.

An adapter should expose project-owned types at its boundary whenever practical. Vendor errors, native pointers, framework objects, and implementation-specific state should remain behind the adapter.

Adapters normally depend on the feature contracts they implement. They do not depend upward on engines or applications. Cross-adapter dependencies are a warning sign: if two adapters need shared behavior, that behavior usually belongs in a lower contract/module or in the engine that composes them.

#### Engine crates: stateful orchestrators

Engines own state and coordinate work across features and adapters. This is where lifecycle, admission, scheduling, transactions, cancellation, cleanup, and application use cases live when those concerns require shared runtime ownership.

An engine should answer a clear ownership question such as:

- Who exclusively owns this native resource?
- Who advances this state machine?
- Who coordinates these adapters into one use case?
- Who decides when work is admitted, cancelled, drained, or released?

Do not create a new engine merely to create another abstraction layer. Engine proliferation increases coordination and dependency pressure when several crates share one lifecycle. Split an engine when there is evidence of independent ownership, lifecycle, reuse, deployment, or a stable boundary between resource-level orchestration and application-level use cases.

#### App or runner crates: presentation and process boundary

Applications own the environment-facing shell:

- event loops and UI framework objects;
- process startup and shutdown entry points;
- platform path selection;
- environment and command-line I/O;
- transport endpoints when the application itself is the host;
- mapping user intent into coarse engine/application operations;
- presentation of state and results.

Applications should remain thin. They do not reimplement lifecycle state machines, drive backend hot loops one step at a time, or construct vendor-specific internals when an engine/application boundary already owns that composition.

### Typical dependency shape

```text
apps / runners
      ↓
    engines
     ↙   ↘
adapters  features
    ↓       ↑
    └───────┘
  project-owned contracts
```

Read the arrows as dependency direction, not runtime call direction. Engines may depend on both features and selected adapters. Adapters depend on lower project contracts. Features never depend upward on adapters, engines, or apps.

A concrete project may refine this into tiers such as foundation features, resource-owning engines, and application-use-case engines. Those tiers belong in the project's applied architecture rather than this reusable blueprint.

---

## Universal API boundaries and coupling laws

These laws apply to both workspace models.

### Explicit public APIs

A crate boundary should hide implementation details rather than merely relocate files. Keep internal helpers and state private or crate-visible. Public APIs expose the smallest stable vocabulary required by real consumers.

A public type is a coupling commitment. Do not export vendor types, UI types, native pointers, storage schemas, or orchestration internals through a boundary that claims to be reusable or implementation-neutral.

### Acyclic dependency graph

Workspace dependencies form a directed acyclic graph. A cycle means two units do not have a coherent ownership direction.

Do not solve cycles by introducing hidden globals, callback registries with implicit ownership, or a "common" crate that becomes a dumping ground for unrelated types. Instead, identify the concept both sides actually depend on and place that concept at a lower stable boundary—or merge units that are not truly independent.

### Dependency injection over hidden state

Pass required capabilities through constructors, parameters, handles, or explicit service boundaries. Hidden global state obscures ownership, complicates tests, and makes lifecycle ordering implicit.

Dependency injection does not require turning every service into a trait object or making every application type generic. Use the simplest boundary that supports the required substitution.

### Ownership before convenience

For stateful resources, identify one owner and make lifecycle transitions explicit. Shared references can be useful, but `Arc<T>` is not a substitute for deciding who may load, mutate, drain, destroy, or release a resource.

Multi-step operations should define prepare/validate/commit semantics when partial publication could leave inconsistent state. Cleanup failure must not be treated as successful release merely because the initiating operation failed.

### Bounded growth where growth affects correctness

Queues, histories, workspaces, output buffers, retry loops, and other accumulating state need explicit capacity or backpressure when unbounded growth could violate memory, latency, or shutdown guarantees.

"Bounded" should describe observable behavior: what happens at capacity, who retains ownership, and how progress resumes.

### Composition belongs near the boundary that owns it

Features implement domain behavior. Adapters implement infrastructure. Engines coordinate lifecycle and use cases. Applications present the result.

If a frontend must know backend sequence state, or a pure feature must know database/runtime details, composition has crossed the wrong boundary.

---

## Crate extraction test

Before creating a crate, ask:

1. **Ownership:** does this unit own a coherent concept or resource?
2. **Change reason:** can it evolve for reasons independent of its current parent?
3. **Dependency isolation:** does the boundary prevent heavy or platform-specific dependencies from spreading?
4. **Portability:** does it need a different `std`/target/dependency profile?
5. **Reuse:** is there a realistic second consumer or standalone use?
6. **Lifecycle:** does it have an independently meaningful initialization, operation, or cleanup boundary?
7. **Verification:** does a crate boundary create a useful independent test, benchmark, lint, or build boundary?

Several strong "yes" answers justify a crate. If the only reason is file size or aesthetic symmetry, prefer an internal module.

The inverse test matters too: if two crates constantly change together, exchange large internal vocabularies, and cannot be used or reasoned about independently, the boundary may be artificial.

---

## Structural anti-patterns and lessons learned

### Monolithic core or god engine

**Symptom:** one crate owns domain algorithms, vendor implementations, persistence, UI-facing DTOs, and lifecycle orchestration.

**Why it hurts:** unrelated changes collide, portable code inherits heavy dependencies, tests require the entire system, and ownership becomes implicit.

**Response:** split along stable domain, infrastructure, lifecycle, or deployment boundaries—not arbitrary file counts.

### Micro-crate hell

**Symptom:** identifiers, one data structure, one callback, or tiny pieces of one lifecycle each receive their own package.

**Why it hurts:** import/API noise increases, the dependency graph becomes harder to reason about, and changes that should be local cross several package boundaries.

**Response:** consolidate concepts that share one owner and reason to change. A cohesive contract crate is better than a constellation of one-type packages.

### Leaky adapter

**Symptom:** vendor tensors, database handles, FFI pointers, framework errors, or OS-specific types appear throughout feature and engine APIs.

**Why it hurts:** the implementation becomes the architecture. Replacing the vendor requires invasive changes and portable layers can no longer stand alone.

**Response:** translate at the adapter boundary into project-owned contracts and stable error categories.

### Orchestration hidden in features

**Symptom:** a supposedly pure feature starts loading files, starting workers, choosing concrete backends, managing global state, or coordinating unrelated domains.

**Why it hurts:** reuse and portability disappear, and ownership is split between the feature and engine.

**Response:** keep the feature deterministic where practical and move cross-domain lifecycle coordination to an engine.

### Frontend-driven hot loops

**Symptom:** the UI or transport submits one command for every token, frame, packet, or internal backend step.

**Why it hurts:** throughput and correctness become coupled to presentation cadence, channel pressure, and frontend stalls.

**Response:** place the high-frequency loop beside the state/resource owner and expose coarse commands plus bounded/pull-oriented output.

### Premature abstraction

**Symptom:** every service receives a trait, generic parameter, factory, and indirection before a second implementation or consumer exists.

**Why it hurts:** the abstraction surface grows faster than the evidence defining it.

**Response:** keep concrete composition until replacement pressure is real; extract coarse seams where a second implementation, test boundary, or deployment mode demonstrates the need.

### Shared-types dumping ground

**Symptom:** unrelated types migrate into a foundation crate solely to satisfy a layer rule.

**Why it hurts:** the lowest layer becomes a semantic monolith and changes for every domain.

**Response:** lower only vocabulary that genuinely has multiple stable consumers or crosses a real architectural boundary. Revisit an over-restrictive dependency rule instead of laundering dependencies through "common" types.

---

## Performance, portability, and unsafe boundaries

Architecture constrains where costs and risks may appear; it does not make performance claims true by declaration.

- Prefer readable idiomatic code until a named path is measured.
- Static dispatch and preallocation are valuable in measured hot loops; coarse cold-path boundaries may reasonably use dynamic dispatch or allocation.
- `no_std` expresses standard-library independence, not allocation freedom or embedded correctness.
- Allocation-free claims require a defined measured region and must distinguish project allocations from vendor/native/driver behavior.
- Inlining attributes, custom layouts, smaller integer types, lock-free structures, and other micro-optimizations require workload-specific evidence.
- Unsafe or FFI code belongs behind the narrowest boundary that can state and enforce its safety preconditions. Safe project types should prevent invalid native state from escaping that boundary.

Component benchmarks establish component behavior. End-to-end latency, throughput, memory, cancellation, and shutdown claims require system-level measurements.

---

## Applying the blueprint to a project

The selected model defines the **shape of reasoning**, not the final crate graph.

For each project:

1. select Model A or Model B above;
2. write an applied architecture that names the actual crates/layers and allowed dependency direction;
3. record important project-specific choices and rejected alternatives in ADRs;
4. enforce critical dependency rules in tooling where practical;
5. keep current product support and temporary execution state out of this reusable blueprint;
6. revisit boundaries when integration evidence shows that ownership or reuse differs from the original assumption.

A good architecture document should teach a new contributor **why the graph has its shape**, while the project architecture should tell them **what the graph is today**.
