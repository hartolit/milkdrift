# Workspace boundaries

This document is the concrete workspace inventory: crate placement, current members, production dependency edges, and generated-code lint boundaries. Architectural rationale and layer responsibilities live in [project architecture](architecture.md).

## Physical layout

```text
llm-app/
├── .cargo/
│   └── config.toml
├── Cargo.toml                 virtual workspace manifest
├── tools/
│   └── xtask/                 custom architecture, hygiene, and verify tooling
│       ├── src/
│       └── tests/
├── benchmarks/
│   └── runtime/               future cross-crate harness; not yet created
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

The root has no package target. `.cargo/config.toml` maps `cargo xtask` to the locked `tools/xtask` package; product execution vectors remain under `crates/apps/`.

## Current members

```text
tools/xtask
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

`benchmarks/runtime` is intentionally absent from the current member list. Phase 10 will add it as `runtime-benchmarks` only with the first real cross-crate/system harness. It is the sole recognized package path under `benchmarks/`; unknown benchmark manifests fail closed.

## Responsibility-based source organization

Several larger responsibilities are split into private or crate-internal modules without creating new workspace layers:

- `task-graph` separates graph structure/validation (including its owned `TaskId`), artifact-flow validation, runtime state transitions, and errors;
- `inference-runtime` separates admission, execution, cleanup, memory/accounting, inspection, unload, and shutdown around one `InferenceRuntime` registry;
- E1 generation separates admission, the inference/text bridge, bounded output, and generation settings inside `application-runtime`;
- the desktop presenter separates callback binding, control synchronization, model mapping, and bounded output/conversation presentation.

These are responsibility and maintainability boundaries. Visibility remains controlled by each crate root, so the file splits do not by themselves assert new public APIs or independently deployable components.

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

`xtask`, `hf-hub-adapter`, and `redb-storage` have no workspace-local production dependencies. `xtask` depends externally on the reviewed `cargo_metadata` crate.

The future `runtime-benchmarks` package is outside the production graph. It may depend only on exact reviewed public production APIs required by implemented measurements. No production, tooling, test, or application package may depend on it, including through development or build dependencies. Its exact outgoing edges will be registered with the harness rather than pre-authorized.

`application-runtime/src/local.rs` is a private internal composition boundary, not a workspace member. It owns one `HostedRuntime<CandleLlamaSource>` and one inference worker thread. E1 separately owns one bounded Hub worker, one `HfTokenizer`, request-local `HfOwnedStreamingDecoder` values, and one resident-model lifecycle. [ADR-0013](../agent/decisions/0013-candle-only-local-execution.md) records this composition. There is no `application-api` package.

`desktop-slint` has no production import of Candle, Hugging Face adapter source types, redb, host channels, or inference commands. It maps E1's repository/revision selection, state, events, and model metadata to Slint presentation.

Production code may not acquire an upward dependency. Platform and production adapter crates do not import runtimes or applications; production adapters do not import one another. Development dependencies are reviewed separately. The current workspace-local development exception is `inference-runtime -> candle-backend` for executable E0 compatibility tests.

`inference-runtime` is registered as E0, `corrective-workflow` as a capability engine, `application-runtime` as E1, and `host-runtime` as the current platform package. Runtime and platform roles are not inferred from directory position. Any production edge from a runtime to platform/adapters or another runtime additionally requires an exact reviewed composition entry.

## Architecture enforcement

`cargo xtask architecture` uses typed Cargo metadata with the committed lockfile, fails closed on unknown workspace locations and unresolved local path targets, distinguishes dependency kinds, requires explicit tooling/runtime/platform roles, and applies the dependency rules documented in [dependency policy](dependency-policy.md).

The accepted product roots are `crates/domain`, `crates/platform`, `crates/adapters`, `crates/runtime`, and `crates/apps`; `tools/xtask` is the only accepted tooling package. `benchmarks/runtime` is the only reserved benchmark package path and is not a current member. Other benchmark/tooling locations and legacy product paths are not classified. Adapter packages remain direct children of `crates/adapters` until a later structural decision explicitly permits deeper grouping.

When Phase 10 creates `benchmarks/runtime`, its path and manifest must be added to root `workspace.members` in the same change before any Cargo command targets that manifest. It uses the root `Cargo.lock` and root `target`, declares `publish = false`, and has no nested workspace, nested lockfile, custom build target, or `build.rs`. A shared benchmark-support package requires two real consumers and a separate ownership review.

## Generated-code lint boundary

Workspace-owned source denies unsafe code. Most pure crates additionally use `#![forbid(unsafe_code)]`.

The workspace-level lint is `deny`, not `forbid`, because Slint-generated Rust applies a narrow local `allow(unsafe_code)` around generated vtable implementation. `forbid` cannot be lowered by generated code and would reject that expansion. This exception does not permit unsafe blocks in project-authored source.
