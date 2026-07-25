# Architecture principles

This document defines reusable architectural principles for modular systems software. Project-specific layers, crate names, supported devices, backend choices, and temporary product constraints belong in `project/architecture.md` and project ADRs.

## Rule classes

Architectural statements should be classified rather than treated as equally permanent:

- **Hard invariant** — required for correctness, safety, or an intentionally enforced boundary.
- **Current decision** — the selected design for the present system; revisable when evidence changes.
- **Performance hypothesis** — a claim that requires a named benchmark, profile, allocation test, or generated-code inspection.
- **Style preference** — a consistency default that may yield to clearer code.
- **Temporary constraint** — an explicit simplification used while a product slice or integration is being proved.

This classification prevents preferences and unmeasured optimization folklore from becoming accidental architecture law.

## Ownership and lifecycle

Stateful resources should have a clear owner. Handles exposed across boundaries should carry the minimum identity or capability required by callers rather than creating shared ownership by default.

Lifecycle-sensitive operations should use explicit states and transitions when ordering matters. Resource creation, admission, cancellation, drain, cleanup, unload, and shutdown should define:

- who owns the resource at each state;
- when state becomes externally visible;
- what happens when an intermediate step fails;
- which cleanup operations are mandatory;
- which operations are bounded and where progress depends on an external dependency.

Do not destroy native or stateful resources while another execution path may still hold access to them. Multi-step admission should validate and reserve before publishing state, then commit atomically or retain enough ownership to perform explicit rollback.

Use explicit capacities where unbounded queues, workspaces, histories, or output accumulation could violate memory, latency, or lifecycle guarantees.

## Dependency direction

The production dependency graph should be acyclic and reflect ownership rather than folder aesthetics.

A useful layered model is:

```text
applications / presentation
          ↓
use-case orchestration
          ↓
resource-owning engines
          ↓
adapters and portable domain logic
```

The exact graph is project-specific, but the underlying rules are reusable:

- lower-level portable logic does not import application or infrastructure concerns;
- adapters quarantine vendor, FFI, filesystem, network, database, and OS-specific dependencies;
- orchestration depends downward on the contracts and implementations it composes;
- applications own process, event-loop, environment, and presentation concerns;
- cross-layer exceptions are explicit, narrow, and reviewable rather than implicit shortcuts.

Folder names are communication tools, not dependency mechanisms. Move a crate or module only when ownership, reuse, lifecycle, or dependency evidence justifies the change.

## Boundaries and public APIs

Public APIs should expose the minimum stable vocabulary needed by consumers. Vendor types, native pointers, transport details, UI framework types, and persistence formats should not leak through boundaries that claim to be portable or implementation-neutral.

Prefer cohesive modules before extracting micro-crates. A new crate is justified by independent ownership, reuse, lifecycle, portability, dependency isolation, or a meaningful test/build boundary—not by a numerical target for crate count.

Dependency injection is preferred over hidden global state. Use generics where static dispatch or type relationships materially matter; do not spread type parameters through public cold-path APIs without a concrete reuse or performance reason.

## Unsafe and native boundaries

Project-authored unsafe code should be denied where practical. When FFI, generated code, or a low-level primitive requires unsafe code:

- keep the exception in the narrowest module that requires it;
- state the safety preconditions for authored unsafe operations;
- expose a safe boundary that prevents invalid pointers, lifetimes, aliasing, or vendor error representation from escaping;
- do not treat generated or third-party unsafe implementation details as permission for unrelated authored unsafe code.

## Performance claims

Readable, idiomatic code is the default. Optimize a named hot path only when evidence identifies the cost.

Static dispatch, preallocation, compact layouts, inlining hints, cold annotations, custom collections, and specialized data structures are tools rather than universal rules. Their value depends on workload, target, compiler, and surrounding code generation.

An allocation-free claim requires a defined measured region. `no_std`, caller-owned output, or bounded APIs do not prove that upstream libraries, native runtimes, drivers, or allocators perform no allocation.

Component benchmarks prove component behavior only. End-to-end latency, throughput, cancellation, memory, and shutdown behavior require system-level evidence.

## Shutdown and uncooperative dependencies

Explicit shutdown is preferred when resource release or worker termination can fail, time out, or produce observable results. Blocking `Drop` should not be the primary shutdown protocol for resources whose cleanup needs coordination.

A bounded caller cannot make an arbitrary in-process dependency forcibly cancellable. If an operation may not return, choose and document one of these strategies:

- require a backend contract with bounded/cooperative safe points;
- split work into bounded chunks controlled by the owner;
- isolate the untrusted or uncooperative work in a process whose termination delegates final reclamation to the operating system.

Timeouts define caller patience; they do not by themselves prove physical resource reclamation.

## Project specialization

Apply these principles in `project/architecture.md`. Record project-specific decisions and rejected alternatives in ADRs. Keep temporary product constraints and support claims out of this reusable document.
