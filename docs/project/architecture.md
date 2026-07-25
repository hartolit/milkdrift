# llm-app project architecture

This document applies the reusable [architecture principles](../architecture.md) to llm-app. Accepted ADRs record the rationale for important project decisions; [workspace boundaries](workspace.md) owns the exact crate inventory and dependency edges.

## Layer model

The repository uses four physical categories:

```text
crates/features/    portable contracts and algorithms
crates/adapters/    infrastructure and vendor implementations
crates/engines/     stateful orchestration and resource ownership
crates/apps/        process, event-loop, and presentation boundaries
```

The intended production direction is:

```text
apps
  ↓
E1 application-runtime
  ↓
E0 inference-runtime
  ↓
adapters and features
  ↓
domain-contracts
```

This diagram expresses ownership and permitted composition, not a requirement that every crate traverse every layer. Exact edges and development-dependency exceptions are documented in [workspace boundaries](workspace.md) and enforced by the repository architecture validator.

## Feature tiers

`domain-contracts` is the F0 shared foundation. It owns vocabulary that genuinely crosses backend/engine or multiple-feature boundaries: typed identities, capacities, model/sequence contracts, lifecycle transitions, and output records.

`tokenization`, `context-planner`, `sampling`, and `task-graph` are F1 algorithm crates. The currently enforced production policy permits F1 → F0 and rejects F1 → F1. This is a project constraint rather than a universal Rust rule; do not push unrelated vocabulary into F0 merely to evade the graph.

Portable feature code does not import engines, applications, vendor runtimes, frontend toolkits, or filesystem/network/database/OS transport implementations. Portability claims are scoped in [portability](portability.md).

## Engine tiers

E0 `inference-runtime` exclusively owns loaded model generations, backend sequences, request admission, generation workspaces, sampling execution, cancellation, draining, cleanup quarantine, accounting, and unload.

E1 `application-runtime` is the frontend-neutral application façade and current native composition root. It owns artifact resolution, tokenizer validation, persistence, prompt/text orchestration, normalized application state/events, unload behavior, and application use cases.

E1 may depend on E0; E0 may not depend on E1. Frontends use E1 instead of composing E0 and adapters independently. This decision is recorded in [ADR-0001](../decisions/0001-application-runtime-facade.md).

Cold, coarse replacement points may use trait objects or closed enums when a real consumer requires replacement. Token-sensitive model execution stays statically dispatched where measurement and ownership justify it.

## Adapters

Adapters own vendor, native, persistence, network, filesystem, and host integration details. They do not depend on engines or applications, and production adapters do not depend on one another.

Candle is the currently composed application inference backend. GGUF/llama.cpp exists at the adapter/E0 compatibility boundary but is not yet composed through E1 or the UI. Backend-specific capabilities and limitations belong in the respective backend guides; product support belongs in [implementation status](implementation-status.md).

## Frontend and deployment boundary

`desktop-slint` owns the native event loop, presentation, platform path selection, and UI command mapping. It does not own model tensors, token scheduling, persistence implementation, Hub implementation, or inference lifecycle policy.

A native Slint, Tauri, CLI, or similar process can call E1 directly. A browser-only frontend requires an explicit transport to a native or remote host because the browser cannot directly own the project's native model runtimes, database, threads, and filesystem paths.

The frontend presents state and pulls bounded output. It does not issue one inference command per generated token. Generation scheduling lives beside model execution as recorded in [ADR-0003](../decisions/0003-generation-scheduling-ownership.md).

## Lifecycle and resource policy

Model and sequence values are exclusively owned by E0 rather than shared through public `Arc<Model>`-style ownership. Public handles carry identity and generation safety, not ownership of native model state.

Admission validates capacities and accounting before state becomes visible. Cleanup failure does not imply release: unresolved model or sequence ownership remains quarantined and accounted until explicit cleanup succeeds or its retry policy is exhausted. Detailed behavior belongs in [inference runtime](inference-runtime.md) and [model lifecycle](lifecycle.md).

Explicit bounded shutdown is required for normal operation; blocking `Drop` is not the primary shutdown protocol. See [ADR-0006](../decisions/0006-explicit-bounded-shutdown.md).

## Current product constraints

The current product boundary is intentionally narrower than the lower-level contracts:

- CPU is the supported device class;
- Candle Llama/Safetensors is the composed E1 inference path;
- GGUF is not yet an E1/UI selection;
- E1 exposes one resident model at a time;
- direct completion is implemented before general chat/history rendering;
- GPU execution and browser/remote transport are not supported product paths.

These are current project constraints, not reusable architecture rules. The authoritative integration matrix and active execution position are in [implementation status](implementation-status.md).

## Enforcement

The architecture validator loads typed Cargo metadata, fails closed on unknown workspace locations and unresolved local path targets, distinguishes dependency kinds, tests the layer matrix, and applies reviewed external dependency policy. [Dependency policy](dependency-policy.md) documents the enforced repository and supply-chain rules.

Project-authored source denies unsafe code. Generated-code or FFI exceptions are narrow and contained; [workspace boundaries](workspace.md) records the current generated-code lint boundary.
