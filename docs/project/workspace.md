# Workspace boundaries

This document is the concrete workspace inventory: crate placement, current members, production dependency edges, and generated-code lint boundaries. Architectural rationale and layer responsibilities live in [project architecture](architecture.md).

## Physical layout

```text
llm-app/
├── Cargo.toml
├── src/main.rs
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
    │   ├── gguf-backend/
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

The root package is the native Rust maintenance runner. Product execution vectors remain under `crates/apps/`.

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
crates/adapters/gguf-backend
crates/adapters/hf-tokenizer
crates/adapters/hf-hub
crates/adapters/redb-storage
crates/runtime/inference-runtime
crates/runtime/corrective-workflow
crates/runtime/application-runtime
crates/apps/desktop-slint
```

Each domain and runtime crate owns a coherent, independently testable responsibility. None exists merely to hold one identifier, data structure, or callback.

## Production dependency edges

```text
candle-backend      -> domain-contracts
gguf-backend        -> domain-contracts
host-runtime        -> domain-contracts
hf-tokenizer        -> tokenization -> domain-contracts
inference-runtime   -> host-runtime + domain-contracts
corrective-workflow -> task-graph + domain-contracts
application-runtime -> inference-runtime + selected platform/adapters/domain/capability engines
desktop-slint       -> application-runtime + slint
```

`desktop-slint` does not import Candle, Hugging Face, redb, host channels, or inference commands directly. Slint types remain in the application crate; E1 public events expose stable application/domain values rather than vendor types.

Production code may not acquire an upward dependency. Platform and production adapter crates do not import runtimes or applications; production adapters do not import one another. Development dependencies are reviewed separately and may cross production direction only for an explicitly named compatibility test or benchmark.

`inference-runtime` is registered as E0, `corrective-workflow` as a capability engine, `application-runtime` as E1, and `host-runtime` as the current platform package. Runtime and platform roles are not inferred from directory position. E1 may depend on capability engines; capability engines cannot depend on E1. Any production edge from a runtime to platform/adapters or another runtime additionally requires an exact reviewed composition entry.

## Architecture enforcement

The validator uses typed Cargo metadata, fails closed on unknown workspace locations and unresolved local path targets, distinguishes dependency kinds, requires explicit runtime-role classification, and applies the dependency rules documented in [dependency policy](dependency-policy.md).

The accepted roots are `crates/domain`, `crates/platform`, `crates/adapters`, `crates/runtime`, and `crates/apps`. Legacy `features`/`engines` paths are no longer classified. Adapter packages remain direct children of `crates/adapters` until a later structural decision explicitly permits deeper grouping.

Its purpose is to enforce the real graph rather than infer architecture from folder names.

## Generated-code lint boundary

Workspace-owned source denies unsafe code. Most pure crates additionally use `#![forbid(unsafe_code)]`.

The workspace-level lint is `deny`, not `forbid`, because Slint and `self_cell` generate Rust that applies a narrow local `allow(unsafe_code)` inside private generated-code modules. `forbid` cannot be lowered by generated code and would reject those valid expansions. This exception does not permit unsafe blocks in project-authored source.
