# Workspace boundaries

This document is the concrete workspace inventory: crate placement, current members, production dependency edges, and generated-code lint boundaries. Architectural rationale and layer responsibilities live in [project architecture](architecture.md).

## Physical layout

```text
llm-app/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── main.rs
│   └── hygiene.rs
├── tests/
├── docs/
└── crates/
    ├── domain/
    │   ├── domain-contracts/
    │   ├── tokenization/
    │   ├── context-planner/
    │   ├── sampling/
    │   └── task-graph/
    ├── platform/
    │   └── host-runtime/
    ├── adapters/
    │   ├── candle-backend/
    │   ├── hf-tokenizer/
    │   ├── hf-hub/
    │   └── redb-storage/
    ├── runtime/
    │   ├── inference-runtime/
    │   ├── corrective-workflow/
    │   └── application-runtime/
    └── apps/
        └── desktop-slint/
```

The root package is the native Rust maintenance runner and architecture/hygiene validator. Product execution vectors remain under `crates/apps/`.

## Current members

```text
.
crates/domain/domain-contracts
crates/domain/tokenization
crates/domain/context-planner
crates/domain/sampling
crates/domain/task-graph
crates/platform/host-runtime
crates/adapters/candle-backend
crates/adapters/hf-tokenizer
crates/adapters/hf-hub
crates/adapters/redb-storage
crates/runtime/inference-runtime
crates/runtime/corrective-workflow
crates/runtime/application-runtime
crates/apps/desktop-slint
```

Each domain and runtime crate owns a coherent, independently testable responsibility. None exists merely to hold one identifier, data structure, or callback.

## Workspace-local production dependency edges

```text
tokenization        -> domain-contracts
context-planner     -> domain-contracts
sampling            -> domain-contracts
task-graph          -> domain-contracts
candle-backend      -> domain-contracts
hf-tokenizer        -> tokenization + domain-contracts
host-runtime        -> domain-contracts
inference-runtime   -> host-runtime + sampling + domain-contracts
corrective-workflow -> task-graph + domain-contracts
application-runtime -> context-planner + tokenization + domain-contracts
                    + candle-backend + hf-hub-adapter + hf-tokenizer
                    + redb-storage + host-runtime + inference-runtime
desktop-slint       -> application-runtime
```

`application-runtime/src/local.rs` is a private internal composition boundary, not a workspace member. It owns one `HostedRuntime<CandleLlamaSource>` and one inference worker thread. E1 separately owns one bounded Hub worker, one `HfTokenizer`, request-local `HfOwnedStreamingDecoder` values, and one resident-model lifecycle. [ADR-0013](../agent/decisions/0013-candle-only-local-execution.md) records this composition. There is no `application-api` package.

`desktop-slint` has no production import of Candle, Hugging Face adapter source types, redb, host channels, or inference commands. It maps E1's repository/revision selection, state, events, and model metadata to Slint presentation.

Production code may not acquire an upward dependency. Platform and production adapter crates do not import runtimes or applications; production adapters do not import one another. Development dependencies are reviewed separately. The current workspace-local development exception is `inference-runtime -> candle-backend` for executable E0 compatibility tests.

`inference-runtime` is registered as E0, `corrective-workflow` as a capability engine, `application-runtime` as E1, and `host-runtime` as the current platform package. Runtime and platform roles are not inferred from directory position. Any production edge from a runtime to platform/adapters or another runtime additionally requires an exact reviewed composition entry.

## Architecture enforcement

The validator uses typed Cargo metadata with the committed lockfile, fails closed on unknown workspace locations and unresolved local path targets, distinguishes dependency kinds, requires explicit runtime/platform roles, and applies the dependency rules documented in [dependency policy](dependency-policy.md).

The accepted roots are `crates/domain`, `crates/platform`, `crates/adapters`, `crates/runtime`, and `crates/apps`. Legacy paths are not classified. Adapter packages remain direct children of `crates/adapters` until a later structural decision explicitly permits deeper grouping.

## Generated-code lint boundary

Workspace-owned source denies unsafe code. Most pure crates additionally use `#![forbid(unsafe_code)]`.

The workspace-level lint is `deny`, not `forbid`, because Slint-generated Rust applies a narrow local `allow(unsafe_code)` around generated vtable implementation. `forbid` cannot be lowered by generated code and would reject that expansion. This exception does not permit unsafe blocks in project-authored source.
