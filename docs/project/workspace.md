# Workspace boundaries

This document is the concrete workspace inventory: crate placement, current members, production and observer dependency edges, and generated-code lint boundaries. Architectural rationale and layer responsibilities live in [project architecture](architecture.md).

## Physical layout

```text
milkdrift/
├── .cargo/
│   └── config.toml
├── Cargo.toml                 virtual workspace manifest
├── tools/
│   └── xtask/                 custom architecture, hygiene, and verify tooling
│       ├── src/
│       └── tests/
├── benchmarks/
│   └── runtime/               runtime-benchmarks non-production measurement observer
├── docs/
└── crates/
    ├── domain/
    │   ├── domain-contracts/
    │   ├── tokenization/
    │   ├── context-planner/
    │   └── sampling/
    ├── platform/
    │   └── host-runtime/
    ├── adapters/
    │   ├── candle-backend/
    │   ├── hf-tokenizer/
    │   ├── hf-hub/
    │   └── redb-storage/
    ├── runtime/
    │   ├── inference-runtime/
    │   └── application-runtime/
    └── apps/
        └── desktop-slint/
```

The root has no package target. `.cargo/config.toml` maps `cargo xtask` to the locked `tools/xtask` package; product execution vectors remain under `crates/apps/`.

## Current members

```text
tools/xtask
benchmarks/runtime
crates/domain/domain-contracts
crates/domain/tokenization
crates/domain/context-planner
crates/domain/sampling
crates/platform/host-runtime
crates/adapters/candle-backend
crates/adapters/hf-tokenizer
crates/adapters/hf-hub
crates/adapters/redb-storage
crates/runtime/inference-runtime
crates/runtime/application-runtime
crates/apps/desktop-slint
```

Each member declares one nonempty present responsibility beside its role. Missing,
non-string, or empty responsibility metadata fails architecture and verification
planning.

Every tracked non-fixture package manifest must appear in the root workspace, and every member declares one explicit `[package.metadata.milkdrift] role`. Role strings and compatible direct-child locations are documented in [project architecture](architecture.md); omitted members and missing, unknown, or misplaced roles fail closed. Runtime packages must also be reachable from an `application` role through actual normal Cargo edges, so build/development dependencies and workspace membership cannot certify inactive runtime scope.

`benchmarks/runtime` is the root-workspace member whose package name is `runtime-benchmarks` and role is `benchmark-observer`. It uses the root `Cargo.lock`, declares `publish = false`, and has no nested workspace/lockfile, build script, Cargo custom-build target, or build dependencies. No workspace package has an incoming dependency edge to it. Unknown or unclassified benchmark manifests fail closed, while a future explicitly classified observer can be added without changing a sole-package constant.

## Responsibility-based source organization

Several larger responsibilities are split into private or crate-internal modules without creating new workspace layers:

- `inference-runtime` separates admission, execution, cleanup, memory/accounting, inspection, unload, and shutdown around one `InferenceRuntime` registry;
- E1 generation separates admission, the inference/text bridge, bounded output, and generation settings inside `application-runtime`;
- the desktop presenter separates callback binding, control synchronization, model mapping, and bounded output/conversation presentation.

These are responsibility and maintainability boundaries. Visibility remains controlled by each crate root, so the file splits do not by themselves assert new public APIs or independently deployable components.

## Workspace-local production dependency edges

```text
tokenization        -> domain-contracts
context-planner     -> domain-contracts
sampling            -> domain-contracts
candle-backend      -> domain-contracts
hf-tokenizer        -> tokenization + domain-contracts
host-runtime        -> domain-contracts
inference-runtime   -> host-runtime + sampling + domain-contracts
application-runtime -> context-planner + tokenization + domain-contracts
                    + candle-backend + hf-hub-adapter + hf-tokenizer
                    + redb-storage + host-runtime + inference-runtime
desktop-slint       -> application-runtime
```

`xtask`, `hf-hub-adapter`, and `redb-storage` have no workspace-local production dependencies. `xtask` uses reviewed external `cargo_metadata`, `serde_json`, and `toml` dependencies to inspect typed Cargo metadata and exact test-target declarations.

### Non-production observer dependency edges

The complete dependency set for `runtime-benchmarks` is:

```text
workspace-local normal:
  runtime-benchmarks -> application-runtime
  runtime-benchmarks -> candle-backend
  runtime-benchmarks -> domain-contracts
  runtime-benchmarks -> host-runtime
  runtime-benchmarks -> inference-runtime
external normal:
  runtime-benchmarks -> serde
  runtime-benchmarks -> serde_json
  runtime-benchmarks -> sha2
external development:
  runtime-benchmarks -> criterion
```

These are measurement-observer edges outside the production graph even where Cargo classifies them as `normal`: the package consumes reviewed public production APIs but is never part of product execution or composition. It has no build dependencies and no incoming normal, build, or development edge from any production, tooling, test, or application package.

`application-runtime/src/local.rs` is a private internal composition boundary, not a workspace member. It owns one `HostedRuntime<CandleLlamaSource>` and one inference worker thread. E1 separately owns one bounded Hub worker, one `HfTokenizer`, request-local `HfOwnedStreamingDecoder` values, and one resident-model lifecycle. [ADR-0013](../agent/decisions/0013-candle-only-local-execution.md) records this composition. There is no `application-api` package.

`desktop-slint` has no production import of Candle, Hugging Face adapter source types, redb, host channels, or inference commands. It maps E1's repository/revision selection, state, events, and model metadata to Slint presentation.

Production code may not acquire an upward dependency. Platform and adapter crates do not import runtimes or applications, and same-role runtime peers are denied. Ordinary legal normal/build edges follow the generic role DAG and are not duplicated in Rust. Workspace-local development dependencies remain separately reviewed; the current exact exception is `inference-runtime -> candle-backend` for executable E0 compatibility and CUDA hardware suites.

`inference-runtime`, `application-runtime`, and `host-runtime` declare
`runtime-foundation`, `runtime-application`, and `platform` respectively in their
manifests. Their identities are not inferred from directory position or matched
in a package registry.

## Architecture and verification registration

`cargo xtask architecture` uses locked typed Cargo metadata, fails closed on unknown roles/locations, missing responsibilities, unreachable runtimes, and unresolved local path targets, distinguishes dependency kinds, derives the actual domain DAG, validates exact exception records bidirectionally, and applies the generic role rules documented in [dependency policy](dependency-policy.md).

The accepted direct-child roots are `crates/domain`, `crates/platform`, `crates/adapters`, `crates/runtime`, `crates/apps`, `tools`, and `benchmarks`. A package is accepted only when its manifest role is compatible with its root; path placement never supplies a missing role. Deeper grouping requires a deliberate structural change rather than accidental prefix acceptance.

Maintained Cargo bench targets are also package-owned metadata. The current complete inventory is:

```text
runtime-benchmarks / runtime
sampling           / sampling_pipeline
```

Architecture and hygiene compare those registrations bidirectionally with Cargo targets and the owning manifest. Every maintained target requires exactly one explicit `[[bench]]` entry with `harness = false`; a missing target, implicit target, harnessed target, duplicate registration, non-bench registration, or newly discovered unregistered bench fails before compilation. `cargo xtask verify` emits one exact `cargo bench --locked -p PACKAGE --bench TARGET --no-run` command per sorted registration and never builds every workspace library as a release bench harness.

Local developer Cargo commands normally use the root target. CI and clean acceptance use one explicitly named isolated `CARGO_TARGET_DIR` per job and remove it reliably; package-local targets and nested benchmark locks remain prohibited. A separate shared benchmark-support package still requires two real consumers and an ownership review.

## Generated-code lint boundary

Workspace-owned source denies unsafe code. Most pure crates additionally use `#![forbid(unsafe_code)]`.

The workspace-level lint is `deny`, not `forbid`, because Slint-generated Rust applies a narrow local `allow(unsafe_code)` around generated vtable implementation. `forbid` cannot be lowered by generated code and would reject that expansion. This exception does not permit unsafe blocks in project-authored source.
