# Engineering Rules

**Status:** Standing implementation policy  
**Use:** Reference this file when asking a contributor or agent to clean up, refactor, or implement code.

## Purpose

This document governs **implementation quality and architectural discipline**. It does not define product goals, domain concepts, or the current system architecture; those belong in the project’s vision and architecture documents.

Its purpose is practical:

> Leave one simple, coherent, complete, and extensible implementation behind.

A task is not complete because a diff exists or tests pass. It is complete when the intended design is applied throughout its scope, the previous design is removed, and the result is demonstrably better.

## Contents

1. [Core rules](#1-core-rules)
2. [Rust design guide](#2-rust-design-guide)
3. [Code organization](#3-code-organization)
4. [Compatibility](#4-compatibility)
5. [Definition of done](#5-definition-of-done)
6. [Agent execution procedure](#6-agent-execution-procedure)

---

## 1. Core rules

### 1.1 Work with intent

Understand the problem before choosing a mechanism. State the desired rule, its owner, and its scope.

Do not optimize for:

- the smallest diff;
- editing only the named file;
- introducing a visible abstraction;
- preserving accidental structure;
- checking a task off a list.

Optimize for the codebase that remains after the work is finished.

### 1.2 Keep one coherent design

Each concept, policy, state, and lifecycle must have:

```text
one owner
    ↓
one canonical representation
    ↓
one normal operation path
```

Equivalent implementations in several locations are competing sources of truth. Choose one owner, migrate every applicable caller, and delete the alternatives.

Different entry points may adapt input differently, but they must not apply different validation, defaults, construction, business rules, or lifecycle behavior.

### 1.3 Prefer the simplest complete solution

Use the fewest concepts, states, layers, and exceptions that fully preserve required behavior.

Existing complexity is not a reason to keep complexity. A broad replacement is better than wrapping a needlessly complicated design in another layer.

Remove layers that only:

- forward calls;
- rename values;
- mirror another type;
- translate between two internal representations;
- preserve an obsolete path.

Simplicity does not justify weakening correctness, safety, boundedness, or required behavior.

### 1.4 Abstract shared meaning

Consolidate repeated code when it implements the same rule or lifecycle, especially repeated:

- validation and bounds;
- state transitions;
- registration and dispatch;
- retry, cancellation, and cleanup;
- authorization or admission policy;
- serialization envelopes;
- error conversion;
- test contracts.

Do not abstract code merely because a few lines look similar. A useful abstraction gives one shared rule one owner.

A good abstraction must reduce at least one of these:

- duplicated code or decisions;
- public concepts;
- valid call paths;
- error paths;
- knowledge needed for the next implementation;
- risk that implementations behave differently.

Use the smallest suitable mechanism:

```text
function
  → function with closure
  → private trait
  → public trait
```

### 1.5 Fully materialize every adopted abstraction

An abstraction is incomplete while equivalent code remains within its declared scope.

A full implementation must update every applicable area:

| Area | Required result |
| --- | --- |
| Producers | Create or call only the new form. |
| Consumers | Depend on the new interface. |
| Construction | Use one factory, builder, or composition path. |
| Configuration | Builds the new design without legacy alternatives. |
| Persistence and wire forms | Use the intended current representation. |
| Errors and observability | Follow the new owner consistently. |
| Tests and fixtures | Exercise the canonical path and shared contract. |
| Examples and documentation | Show only the current approach. |

Before editing, search the whole workspace for equivalent types, helpers, literals, validation, and construction. After editing, search again and remove every superseded path and bypass.

Do not declare matching code out of scope merely because it is in another file or crate when it belongs to the abstraction’s responsibility.

Two paths may remain only when they enforce genuinely different semantics, ownership, safety, lifecycle, or measured performance requirements. Name and test that distinction.

### 1.6 Make boundaries earn their cost

> A crate must enforce at least one real boundary: distinct semantic ownership, an independently versioned durable or wire contract, an adapter port implemented elsewhere, isolation of platform/security/heavy dependencies, or consumers that need it without an adjacent higher layer. Otherwise it should be a module.

A crate must not exist only because a concept has a name, a diagram has a box, a file became large, or a future consumer might appear.

High-level policy owns the interface. Mechanism-specific code implements it. Framework, database, transport, operating-system, and provider types must not leak through a general interface unless that mechanism is explicitly the subject of the interface.

Avoid cyclic dependencies and ownerless “common” crates.

### 1.7 Fully adopt adapters

> A prospective adapter crate is not complete until at least one production composition root uses it as the sole implementation path. Parallel local implementations are forbidden; either fully adopt the adapter across the codebase or remove it atomically.

Once adopted:

- production code must not instantiate the underlying mechanism directly;
- duplicate factories and mappings must be removed;
- fallback local implementations must be removed;
- mechanism-specific construction and translation must live in the adapter;
- the composition root must select the concrete adapter explicitly.

### 1.8 Design narrow interfaces

An interface should expose what its consumer needs and hide how the work is performed.

A good interface:

- has one clear responsibility;
- uses meaningful input and output types;
- prevents invalid calls where practical;
- exposes typed failures;
- does not leak internal mechanism types;
- has the smallest public surface required by real consumers;
- permits another valid implementation without unrelated changes.

Public APIs are commitments. Keep implementation details private. Tests do not justify making internals public.

Avoid large data bags and broad service interfaces. Group fields and methods by the owner that validates and uses them. Pass each subsystem only its own policy and dependencies.

### 1.9 Make invalid states difficult to represent

Boundary input may be invalid. Internal values should not remain “probably valid.”

Use private fields, fallible constructors, `TryFrom`, `FromStr`, newtypes, and enums so that a constructed value is ready for use without a later `validate()` call.

Prefer explicit states:

```rust
pub enum FeatureMode {
    Disabled,
    Enabled(EnabledFeatureConfig),
}
```

instead of a boolean combined with conditionally required `Option` fields.

Do not use `None`, zero, empty text, and maximum integers as undocumented variants of “disabled,” “unknown,” or “unlimited.”

### 1.10 Make ownership and lifecycle visible

Every thread, task, process, connection, store, temporary file, registration, permit, lease, and queue must have one visible owner responsible for startup, use, failure, and shutdown.

Prefer, in order:

1. immutable data;
2. unique mutable ownership;
3. one owner thread or task with bounded messages;
4. one lock protecting one coherent invariant;
5. atomics for independent scalar facts.

Do not default to `Arc<Mutex<_>>`. Do not hold locks while invoking external code. Long-running work must have explicit cancellation and joining or an equally strong ownership mechanism.

Use RAII guards or owned handles when a resource must be released exactly once.

### 1.11 Treat bounds and nondeterminism as design concerns

Queues, retries, pages, buffers, concurrency, recursion, retention, document size, and shutdown waits must be bounded when unbounded growth or waiting is possible. Define what happens when a bound is reached.

Time, randomness, identifiers, environment access, filesystem discovery, network state, and external responses should enter through clear owning boundaries when they affect important behavior or tests.

Do not scatter direct system calls with inconsistent fallback behavior across the codebase.

### 1.12 Prove and enforce the rule

Tests should prove behavior and invariants, not restate an implementation.

Every open interface with multiple implementations should have one reusable conformance suite. Each implementation runs that suite plus mechanism-specific tests.

Repeatedly violated rules should become tooling or CI checks where practical, including:

- dependency direction;
- forbidden imports or direct mechanism access;
- public API growth;
- obsolete paths;
- configuration validity;
- formatting, linting, tests, documentation, and dependency audits.

Passing tests does not excuse duplicate architecture, partial adoption, or an unnecessarily complex interface.

---

## 2. Rust design guide

### 2.1 Choose mechanisms by meaning

| Need | Preferred mechanism | Completion condition |
| --- | --- | --- |
| Shared deterministic operation | Function | Matching copies are migrated or removed. |
| One local step varies | Closure parameter | Variation remains local; no unnecessary trait. |
| Closed set of meanings or states | `enum` | Variants are handled exhaustively. |
| Open family of implementations | Consumer-owned trait | Intended implementations use it and pass one conformance suite. |
| Compile-time polymorphism | Generic / `impl Trait` | Reuse or performance improves without spreading needless generic complexity. |
| Runtime-selected implementations | `dyn Trait` at a composition boundary | Dynamic choice stays at the boundary where practical. |
| Constrained primitive | Newtype with private field | Raw values cannot bypass validation in the owned scope. |
| Many optional construction choices | Fallible builder | `build()` returns a complete valid value. |
| Legal operations change by phase | Selective typestate | It removes an important invalid transition. |
| Resource ownership | RAII guard / owned handle | Release happens exactly once on every path. |
| Repeated declarations must stay synchronized | Small declarative macro | All matching declarations adopt it and expansion remains understandable. |
| Cohesive internal responsibility | Private module | Ownership remains with the parent. |
| Independently enforced boundary | Crate | It satisfies the crate rule in section 1.6. |

Do not create a trait for every struct or a macro for ordinary business logic. Do not duplicate a shared rule merely to avoid an appropriate trait, function, closure, or macro.

### 2.2 Constants, defaults, and configuration are different

| Kind of value | Representation | Meaning |
| --- | --- | --- |
| Semantic invariant | `const`, associated constant, enum, or invariant-bearing type | Cannot vary without changing meaning. |
| Unconditional hard ceiling | Owner-defined `const` | No valid deployment may exceed it. |
| Safe ordinary baseline | `Default` | One complete valid choice within allowed limits. |
| Operator or deployment choice | Strict configuration | May vary after validation. |
| Per-operation choice | Required request or policy field | Must be explicit for that operation. |
| External observation or behavior | Injected dependency or port | Comes from time, environment, I/O, or a host. |

The normal relationship is:

```text
hard ceiling >= configured value >= minimum valid value
                         ↑
              Default is one safe choice
```

Rules:

- Own each default once; do not copy its literal through tests, templates, and examples.
- `Default::default()` must return a complete, safe value.
- Do not implement `Default` for required identity, secrets, broad authority, or ambiguous intent.
- Use configuration only for choices an operator should make.
- Human-authored Rust application configuration should normally use TOML.
- Use one accepted format per boundary unless an explicit compatibility contract requires more.
- Parse configuration once into validated, normalized, owner-specific construction plans.
- Do not pass raw configuration throughout the application.
- Keep inert options, live dependencies, and mutable service state as separate types.

```text
configuration text
    → private input type
    → validated effective values
    → owner-specific plans
    → live components
```

### 2.3 Use standard traits deliberately

- `From`: infallible and meaning-preserving conversion only.
- `TryFrom`: validation or fallible conversion.
- `FromStr`: validated textual parsing.
- `Default`: one safe, complete baseline.
- `Clone` / `Copy`: duplication is semantically safe.
- `Serialize` / `Deserialize`: an intentional external representation exists.
- `Display`: human presentation, not persistence.
- `Debug`: safe diagnostics; redact sensitive values.
- `Deref`: pointer-like transparency, not general convenience.

Deriving a trait changes what callers may legitimately do. Derive it intentionally.

---

## 3. Code organization

### 3.1 Organize by responsibility

A file or module should communicate one cohesive responsibility. Create submodules when a file contains separable invariants, lifecycle owners, state machines, protocol families, algorithms, or platform implementations.

Do not split large files into arbitrary `part1`, `helpers`, or forwarding modules. That moves complexity without organizing it.

### 3.2 Remove boilerplate before moving it

When size comes from repeated validation, conversion, registration, dispatch, case handling, or test setup, first determine whether a function, trait, closure, data table, or macro can remove the repetition.

Do not distribute the same boilerplate across several smaller files.

### 3.3 Review large files

A production file approaching roughly 1,000 lines requires a cohesion review. It may remain large only when it still owns one clear responsibility, proximity improves understanding, and repeated mechanics have already been removed.

A successful module split leaves:

```text
small parent owner or façade
    ├── clearly named responsibility
    ├── clearly named responsibility
    └── clearly named responsibility
```

Avoid wildcard re-exports, production `use super::*`, and `include!` as substitutes for real structure.

### 3.4 Keep composition explicit

Composition roots create concrete implementations and connect them to consumer-owned interfaces. Domain and library code must not discover implementations through ambient globals, hidden registries, or service locators unless runtime discovery is an explicit product requirement.

---

## 4. Compatibility

Compatibility is required only when an explicit supported contract says so.

Do not retain obsolete:

- APIs or type aliases;
- schema readers or writers;
- configuration formats;
- feature flags;
- factories and adapters;
- fallback execution paths;
- deprecated modules;
- dual representations.

Internal development history is not a compatibility promise. Restart durability and replay of currently supported data are separate requirements and must not be used to justify indefinite support for obsolete development formats.

When a design changes:

```text
replace canonical design
    → migrate all current producers and consumers
    → update tests, fixtures, examples, and documentation
    → delete the old design and compatibility path
```

Do not deprecate what can be removed safely.

---

## 5. Definition of done

A change is complete only when every applicable row is true.

| Check | Required result |
| --- | --- |
| Intent | The problem, intended rule, owner, and scope are clear. |
| Correctness | Required behavior and failure behavior are implemented. |
| Simplicity | No simpler complete design is being avoided to reduce the diff. |
| Ownership | Each concept and policy has one owner. |
| Coherence | One canonical representation and operation path remain. |
| Abstraction | Shared rules use the smallest suitable abstraction. |
| Adoption | Every applicable producer, consumer, and composition path uses it. |
| Removal | Superseded code, aliases, fallbacks, configuration, and tests are deleted. |
| Interface | Public APIs are minimal, typed, and do not leak unrelated mechanisms. |
| Lifecycle | Resources, cancellation, shutdown, and bounds have explicit outcomes. |
| Evidence | Tests and checks prove the rule and important failure cases. |
| Documentation | Canonical examples and documentation show only the resulting design. |
| Final search | No conflicting implementation remains in the declared scope. |

A change is incomplete when the new and old designs both remain valid paths for the same responsibility.

---

## 6. Agent execution procedure

### Before editing

1. Read the relevant implementation, tests, configuration, composition roots, and owning documentation.
2. Identify the responsibility being changed and its intended owner.
3. Search the whole workspace for equivalent implementations, callers, defaults, and bypasses.
4. Define the complete migration scope.
5. Choose the simplest complete design and the smallest suitable Rust mechanism.

### During implementation

1. Implement the canonical owner and interface.
2. Migrate every applicable producer and consumer.
3. Route production construction through one composition path.
4. Make bypasses private or remove them.
5. Remove duplicated policy and boilerplate through appropriate abstractions.
6. Delete the superseded implementation rather than preserving speculative compatibility.

### Before finishing

1. Search again for old types, helpers, literals, factories, readers, call paths, and terminology.
2. Confirm that the new design is used throughout its declared scope.
3. Confirm that no production-local implementation competes with an adopted adapter.
4. Run all relevant quality, contract, failure, and architecture checks.
5. Review the final structure from the perspective of a new contributor.
6. Report what became canonical, what was removed, and what evidence proves completion.

Do not finish merely because the requested edit exists. Finish when one clear, complete, and extensible implementation remains.
