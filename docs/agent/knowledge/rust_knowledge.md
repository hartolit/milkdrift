# Rust Performance and Systems Notes

This document is evidence-oriented guidance, not a set of universal language laws. The normative [architecture](../architecture.md), [engineering rules](../rules.md), and accepted ADRs take precedence. Apply an optimization only to a named path and retain the measurement or code-generation evidence that motivated it.

## Start with the workload

Classify work before optimizing it:

- **Hot path:** repeated token, tensor, buffer, scheduler, or polling work whose cost affects a product metric.
- **Cold path:** startup, configuration, model loading, repository access, persistence, error reporting, and shutdown coordination.

Readable idiomatic Rust is the default for both. Hot paths may justify preallocation, specialized layouts, static dispatch, or bulk operations after evidence identifies the bottleneck. Cold-path allocations and coarse dynamic dispatch are usually dominated by I/O or model-loading cost.

## Memory layout and allocation

Contiguous access often improves cache and prefetch behavior, but the best layout depends on which fields are consumed together. Array-of-structs, struct-of-arrays, and hybrid layouts are alternatives to measure against the real loop.

An ECS does not automatically provide a desired SoA layout and is not required for data-oriented design. Adopt an ECS only when its entity/query/scheduling model solves a product problem; do not introduce it merely to obtain contiguity.

Allocation-free claims need a defined scope:

1. identify the measured region;
2. allocate and prepare all caller-owned state before entering it;
3. include an allocation gate where the Rust global allocator is the relevant allocator;
4. separately account for native libraries, drivers, memory maps, and upstream internals.

`no_std` does not imply allocation-free behavior, and caller-owned output does not prove that an adapter performs no internal allocation.

Bulk slice operations such as `copy_from_slice`, `fill`, and iterator-based loops are good starting points. LLVM may lower them to vectorized or library operations, but this is target- and context-dependent rather than guaranteed.

## Dispatch and type-state

Generics and associated types are appropriate where monomorphization helps token/tensor loops or where types encode a real family relationship. `dyn Trait` adds an indirect call and prevents some inlining, but it does not universally “destroy branch prediction.” Trait objects are reasonable for coarse cold-path services such as storage, artifact resolution, clocks, application ports, and backend selection.

Type-state can prevent invalid transitions at compile time, especially for APIs with a small stable state graph. It also increases type and API complexity. Prefer runtime state with explicit checked transitions when state is dynamic, persisted, data-driven, or crosses transport boundaries.

## Const evaluation

A `const fn` may be evaluated in a const context. Calling it at runtime behaves like calling an ordinary function; marking it `const` does not automatically move work into `.rodata` or remove runtime CPU work.

Use `const fn` when callers benefit from compile-time construction or validation and the API can satisfy const restrictions. Use profiles or generated assembly to establish runtime effects.

## Inlining, cold paths, and branch layout

`#[inline]`, `#[inline(always)]`, and `#[cold]` are compiler hints, not layout or performance guarantees.

- Start without attributes.
- Use `#[inline]` across crate boundaries when a small function must be visible for optimization and evidence supports it.
- Reserve `#[inline(always)]` for rare cases confirmed by code generation or measurement; it can increase code size and instruction-cache pressure.
- Use `#[cold]` when a path is genuinely rare and measurement or generated code shows a useful layout effect.

Profile-guided optimization can provide branch-frequency information at build scale. Avoid encoding guessed branch likelihood throughout application code.

## Atomics and locks

Choose atomic ordering from the synchronization proof:

- `Relaxed` is valid when only atomicity and modification order matter, such as an independent statistic.
- Release/acquire establishes publication and observation of related memory.
- `SeqCst` adds a single global order and is appropriate when the algorithm relies on it, not merely as a substitute for reasoning.

A relaxed atomic still has atomicity guarantees; it does not synchronize unrelated memory. Document which writes a release publishes and which acquire observes.

Lock-free code is not automatically faster or simpler. Prefer the simplest synchronization primitive that satisfies contention, latency, cancellation, and ownership requirements. Do not hold a lock across unbounded I/O or backend work. Measure contention before replacing a lock with a more complex algorithm.

## `no_std` errors

Modern Rust provides `core::error::Error`. A `no_std` crate may implement it without using `std` or heap allocation.

Typed error enums remain useful because they support allocation-free classification and programmatic recovery. Avoid using strings as the only error taxonomy. Owned diagnostics are reasonable at cold application/adapter boundaries where allocation is acceptable; keep stable categories separate from vendor text.

## ABI and data size

Argument and return classification depends on the target ABI, field types, alignment, available registers, optimization, inlining, and calling context. There is no universal 16-byte Rust struct rule.

Do not shrink semantically correct integer types or redesign APIs solely to satisfy a guessed register threshold. For a measured hot call boundary:

1. select the actual deployment target and ABI;
2. inspect optimized generated code;
3. compare by-value and by-reference forms with a representative benchmark;
4. include code size and aliasing effects in the tradeoff.

Use an explicit supported ABI such as `extern "C"` at an FFI boundary. Rust’s native ABI is not a stable cross-language contract.

## Unsafe, volatile access, and assembly

Unsafe code requires a local proof of pointer validity, alignment, initialization, aliasing, lifetime, and concurrency assumptions relevant to the operation. Keep the operation and its proof close together behind a safe API.

Volatile access is for memory locations whose reads or writes are externally observable, commonly MMIO. It does not provide atomicity or inter-thread synchronization. Atomics coordinate threads but do not replace the volatile semantics required by a hardware interface.

Inline assembly is target-specific and should be isolated behind a narrow API. Declare inputs, outputs, clobbers, and only the options whose contracts are actually true. Naked functions and interrupt ABIs require target-specific review and tests; they are not general application optimization tools.

## Floating point and SIMD

Application and model-inference code may use floating point and SIMD normally. Kernel or interrupt code must follow the selected platform’s calling convention and context-switch policy for extended register state. Avoid universal claims about save costs: architecture, enabled features, lazy/eager state management, and OS policy determine the actual behavior.

## Evidence checklist

A performance change should record:

- the product or component metric being improved;
- the target, toolchain, profile, and hardware;
- representative input sizes and configuration;
- before-and-after distributions, not only one sample;
- profiler or generated-code evidence connecting the change to the cost;
- correctness, allocation, and memory effects;
- whether the result is component-local or end-to-end.
