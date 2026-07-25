# Workspace boundaries

This document is the concrete workspace inventory: crate placement, current members, production dependency edges, and generated-code lint boundaries. Architectural rationale and layer responsibilities live in [project architecture](architecture.md).

## Physical layout

```text
llm-app/
├── Cargo.toml
├── src/main.rs
├── docs/
└── crates/
    ├── features/
    │   ├── domain-contracts/
    │   ├── tokenization/
    │   ├── context-planner/
    │   ├── sampling/
    │   └── task-graph/
    ├── adapters/
    │   ├── candle-backend/
    │   ├── gguf-backend/
    │   ├── host-runtime/
    │   ├── hf-tokenizer/
    │   ├── hf-hub/
    │   └── redb-storage/
    ├── engines/
    │   ├── inference-runtime/
    │   └── application-runtime/
    └── apps/
        └── desktop-slint/
```

The root package is the native Rust maintenance runner. Product execution vectors remain under `crates/apps/`.

## Current members

```text
.
crates/features/domain-contracts
crates/features/tokenization
crates/features/context-planner
crates/features/sampling
crates/features/task-graph
crates/adapters/candle-backend
crates/adapters/gguf-backend
crates/adapters/host-runtime
crates/adapters/hf-tokenizer
crates/adapters/hf-hub
crates/adapters/redb-storage
crates/engines/inference-runtime
crates/engines/application-runtime
crates/apps/desktop-slint
```

Each feature and engine crate owns a coherent, independently testable domain. None exists merely to hold one identifier, data structure, or callback.

## Production dependency edges

```text
candle-backend      -> domain-contracts
gguf-backend        -> domain-contracts
host-runtime        -> domain-contracts
hf-tokenizer        -> tokenization -> domain-contracts
inference-runtime   -> host-runtime + domain-contracts
application-runtime -> inference-runtime + selected adapters/features
desktop-slint       -> application-runtime + slint
```

`desktop-slint` does not import Candle, Hugging Face, redb, host channels, or inference commands directly. Slint types remain in the application crate; E1 public events expose stable application/domain values rather than vendor types.

Production code may not acquire an upward dependency. Production adapters do not import one another. Development dependencies are reviewed separately and may cross production direction only for an explicitly named compatibility test or benchmark.

The exact F0/F1 and E0/E1 meanings are defined in [project architecture](architecture.md), avoiding a second copy of that rationale here.

## Architecture enforcement

The validator uses typed Cargo metadata, fails closed on unknown workspace locations and unresolved local path targets, distinguishes dependency kinds, and applies the external dependency rules documented in [dependency policy](dependency-policy.md).

Its purpose is to enforce the real graph rather than infer architecture from folder names.

## Generated-code lint boundary

Workspace-owned source denies unsafe code. Most pure crates additionally use `#![forbid(unsafe_code)]`.

The workspace-level lint is `deny`, not `forbid`, because Slint and `self_cell` generate Rust that applies a narrow local `allow(unsafe_code)` inside private generated-code modules. `forbid` cannot be lowered by generated code and would reject those valid expansions. This exception does not permit unsafe blocks in project-authored source.
