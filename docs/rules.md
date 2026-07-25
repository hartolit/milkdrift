# Engineering rules

These rules are reusable defaults for production Rust and systems engineering. Project-specific toolchain versions, crate graphs, support matrices, active phases, and validation evidence belong under `project/`.

## Hard invariants

### Production behavior

- Merged production paths must be complete, compile-ready, and honest about unsupported behavior.
- Do not present placeholder results, fabricated backend behavior, silent truncation, or unfinished branches as working product functionality.
- Tests may and should use deterministic fakes, fixtures, stubs, and fault injection when real dependencies cannot reliably reproduce failure paths.
- Add focused regression coverage for new invariants and reproduced failures when they can be exercised deterministically.

### Errors and resource handling

- Use typed outcomes for recoverable failures; panic is not ordinary control flow.
- Validate bounds, capacities, identities, and checked arithmetic before publishing externally visible state.
- Multi-step resource operations must either commit completely or preserve enough ownership and accounting to perform explicit cleanup.
- Cancellation, drain, unload, and shutdown contracts must identify their safe points and describe behavior when a dependency does not cooperate.
- Secrets, credentials, and private tokens must not be hardcoded or committed.

### Unsafe and native code

- Deny project-authored unsafe code by default; exceptions require a deliberate boundary and review.
- Authored unsafe operations document their safety preconditions and why those preconditions hold.
- Generated code and third-party macro exceptions remain confined to the narrowest module that needs them.
- Raw native pointers, invalid borrowing relationships, and vendor-specific errors do not escape safe adapter boundaries.

## Change discipline

- Use the repository's pinned toolchain and lockfile when they exist.
- Follow the stable language/edition idioms supported by that toolchain; "modern" does not mean unstable or unnecessarily novel.
- Preserve public APIs unless the scoped change intentionally authorizes a break.
- Record architectural decisions in ADRs instead of silently changing project doctrine.
- Update the canonical status when support, limitations, or validation provenance changes.
- Keep a change reviewable around one invariant, subsystem slice, migration, or clearly coupled set of edits.

## Performance evidence

- Optimize a named hot path only after a benchmark, allocation gate, profile, or generated-code inspection identifies the cost.
- Static dispatch and preallocated buffers are strong defaults for measured token/tensor-style loops, not blanket requirements for cold service boundaries.
- Do not claim allocation-free, portable, backend-neutral, device-capable, or protocol-compatible behavior without a named test or measurement defining the scope.
- Compiler attributes and data-layout changes are hints and tradeoffs, not guarantees; compare before and after on the same relevant toolchain and workload.
- Shared-CI wall-clock timing is observational unless the environment is controlled. Deterministic correctness and allocation tests may still be hard gates.

## Style preferences

- Names communicate domain meaning; standard local abbreviations are acceptable when their meaning is obvious.
- Comments explain non-obvious intent, invariants, safety, or tradeoffs rather than narrating syntax or serving as a changelog.
- Prefer cohesive modules and crates over both god modules and one-type micro-crates.
- Prefer typed configuration for policy values that genuinely vary. Local constants are appropriate for genuinely local invariants.
- Favor readable idiomatic Rust on cold paths. Introduce complex type-state, custom collections, or service abstractions only when they prevent demonstrated errors, isolate dependencies, or support a real consumer.

## Documentation

Follow [documentation conventions](conventions.md). Preserve technical rationale and evidence, but keep current status, historical execution, project architecture, and reusable knowledge in their respective owners.

## Experimental work

Spikes and experimental branches may use shortcuts to answer a clearly identified question. They must be labelled as experiments and may not be merged or documented as supported production behavior until converted into tested, reviewable implementation.
