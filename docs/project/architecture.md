# llm-app project architecture

This project selects **[Model B: Layered Workspace](../architecture.md#model-b-layered-workspace)** from the reusable architecture blueprint. This document specializes that model for llm-app. Accepted [ADRs](../agent/decisions/README.md) record important decisions; [workspace boundaries](workspace.md) owns the exact crate inventory and dependency edges.

## Physical layout and logical roles

The repository keeps five physical categories:

```text
crates/domain/      portable contracts and algorithms
crates/platform/    process-host execution primitives
crates/adapters/    external, vendor, model, and persistence integrations
crates/runtime/     stateful orchestration and resource ownership
crates/apps/        process, event-loop, and presentation boundaries
```

Runtime crates have distinct logical roles rather than one undifferentiated tier:

```text
apps / transported frontends
            ↓
application-runtime (E1 application coordinator)
      ┌─────┴───────────────┐
      ↓                     ↓
capability engines     inference-runtime (E0 local inference)
      └──────────┬──────────┘
                 ↓
      platform / adapters / domain
                 ↓
          domain-contracts
```

E1 may coordinate capability engines and E0. A capability engine may use E0, platform services, adapters, and domain code when its own lifecycle requires them. Neither capability engines nor E0 depend on E1. Applications depend on E1 rather than reconstructing application state machines. Exact edges and reviewed development exceptions are documented in [workspace boundaries](workspace.md) and enforced by the architecture validator.

## Domain tiers

`domain-contracts` is the F0 shared foundation. It owns vocabulary that genuinely crosses backend/runtime or multiple-domain boundaries: typed identities, capacities, model/sequence contracts, lifecycle transitions, and output records.

`tokenization`, `context-planner`, `sampling`, and `task-graph` are F1 algorithm crates. The currently enforced production policy permits F1 → F0 and rejects F1 → F1. This is a project constraint rather than a universal Rust rule; do not push unrelated vocabulary into F0 merely to evade the graph.

Portable domain code does not import runtimes, applications, platform implementations, vendor libraries, frontend toolkits, or filesystem/network/database/OS transport implementations. Portability claims are scoped in [portability](portability.md).

## E0: local inference ownership

E0 `inference-runtime` exclusively owns local loaded model generations, backend sequences, request admission, generation workspaces, sampling execution, cancellation boundaries, draining, cleanup quarantine, accounting, and unload. Its contracts describe direct ownership of native model resources and token-step scheduling.

A hosted model API or another machine is not an E0 backend merely because it can produce text. Remote execution has different ownership, cancellation, accounting, and capability semantics and belongs behind a coarser execution boundary above E0.

## Capability engines

A capability engine owns independently stateful reusable behavior whose lifecycle or reason to change is distinct from the application façade. `corrective-workflow` is the first concrete example: it owns workflow artifacts, attempts, retries, validation state, and events without owning the application or local inference lifecycle.

Capability engines are created only from evidence. Memory orchestration, peer routing, or another subsystem should not become an engine until state, lifecycle, reuse, replacement, or testing pressure gives it a coherent boundary. Capability engines do not depend on one another by default; E1 coordinates separate capabilities unless a lower dependency is explicitly justified.

## E1: application semantics

E1 `application-runtime` is the frontend-neutral application façade and current local composition root. It owns application-level model lifecycle policy, prompt/text orchestration, normalized state/events, persisted preferences, and frontend-shared use cases. Conversation semantics belong here because Slint, a TUI, a headless host, and a transported client should observe the same conversation behavior.

The first composition still wires Candle, Hugging Face, redb, host workers, and E0 directly. That is an implementation constraint, not the definition of E1. Application-domain types must not acquire Candle source values, provider request DTOs, socket connections, or UI objects. Concrete native composition should move behind a coarse boundary when a second backend or deployment makes the seam real rather than guessed.

Do not solve replacement by turning the public façade into `ApplicationRuntime<A, B, C, ...>`. Cold replacement points may use narrow service boundaries, wrappers, or closed enums when substitution is proven. Token-sensitive local inference remains statically dispatched below E1 where measurement and ownership justify it.

## Model execution boundary

The application should eventually be able to fulfill one model request through different targets:

```text
application intent / workflow task
              ↓
       execution selection
       ┌──────┼───────────┐
       ↓      ↓           ↓
   local E0  peer node  hosted provider
```

The common boundary is coarse: target identity and capabilities, complete request admission, cancellation intent, bounded streamed output, usage, and terminal state. Local execution adapts that boundary to E0. Peer and hosted implementations translate it to their transports. Provider SDK and wire types remain in adapters/composition code.

Uniformity must not hide real differences. Context limits, token accounting, prompt/message formats, sampling controls, tool support, privacy boundary, cancellation guarantees, and usage reporting are target capabilities. Unsupported behavior fails explicitly instead of being guessed or silently emulated. This direction is recorded in [ADR-0008](../agent/decisions/0008-capability-and-execution-boundaries.md).

## Platform

`host-runtime` is the current process-host platform implementation. It quarantines bounded channels, named threads, monotonic time, and the synchronization/storage used by pull-oriented output accumulators. It is infrastructure below runtime orchestration; it does not own model, workflow, conversation, or application state.

`platform` is a physical category rather than a new dependency tier today. The validator maps the registered `host-runtime` package to the same lower infrastructure layer as adapters, while keeping the directory distinction explicit. A second platform crate requires architecture review instead of inheriting authority from its folder.

## Adapters

Adapters own vendor, model/backend, persistence, network, filesystem, and external-service integration details. They do not depend on runtimes or applications, and production adapters do not depend on one another.

Candle is the currently composed local inference backend. GGUF/llama.cpp exists at the adapter/E0 compatibility boundary but is not yet composed through E1 or the UI. Future hosted-model clients and peer transports also belong at adapter boundaries; being an adapter does not make them E0 model backends. Product support belongs in [implementation status](implementation-status.md).

## Frontend, node, and deployment boundary

`desktop-slint` owns the native event loop, presentation, platform path selection, and UI command mapping. It does not own model tensors, token scheduling, persistence implementation, Hub implementation, or inference lifecycle policy.

A native Slint, Tauri, TUI/CLI, headless node, or similar process can host or call E1 directly. A browser frontend requires an explicit transport to a native or remote host. A frontend may attach to a node without defining the node's lifetime; closing a window or terminal must not terminate a service intended to continue serving work.

The frontend presents state and pulls bounded output. It does not issue one inference command per generated token. Local generation scheduling lives beside model execution as recorded in [ADR-0003](../agent/decisions/0003-generation-scheduling-ownership.md).

## Lifecycle and resource policy

Model and sequence values are exclusively owned by E0 rather than shared through public `Arc<Model>`-style ownership. Public handles carry identity and generation safety, not ownership of native model state.

Admission validates capacities and accounting before state becomes visible. Cleanup failure does not imply release: unresolved local resources remain quarantined and accounted until explicit cleanup succeeds or its retry policy is exhausted. Detailed behavior belongs in [inference runtime](inference-runtime.md) and [model lifecycle](lifecycle.md).

Explicit bounded shutdown is required for normal operation; blocking `Drop` is not the primary shutdown protocol. See [ADR-0006](../agent/decisions/0006-explicit-bounded-shutdown.md). Remote targets will need their own honest terminal/cancellation semantics rather than inheriting local cleanup claims.

## Current product constraints

The current product boundary is intentionally narrower than the architecture:

- CPU is the supported device class;
- Candle Llama/Safetensors is the composed E1 inference path;
- GGUF is not yet an E1/UI selection;
- E1 exposes one resident local model at a time;
- direct completion is implemented before general chat/history rendering;
- hosted providers, peer execution, GPU execution, and browser transport are not supported product paths.

These are current product constraints, not reusable architecture rules. The authoritative integration matrix and active execution position are in [implementation status](implementation-status.md).

## Enforcement

The architecture validator loads typed Cargo metadata, fails closed on unknown workspace locations and unresolved local path targets, distinguishes dependency kinds, and enforces the logical direction F0/F1 → platform/adapters → E0/capabilities → E1 → applications.

Runtime and platform roles are explicit rather than inferred from folder membership. `inference-runtime`, `corrective-workflow`, and `application-runtime` are the recognized E0, capability, and E1 packages, while `host-runtime` is the only recognized platform crate. The completed `domain`/`runtime` migration is now the accepted layout, and nested adapter categories are not pre-authorized.

Runtime production dependencies on platform/adapters or other runtimes require exact reviewed source/target/kind entries in addition to satisfying the layer matrix. [Dependency policy](dependency-policy.md) owns those review rules and their current justifications.

Project-authored source denies unsafe code. Generated-code or FFI exceptions are narrow and contained; [workspace boundaries](workspace.md) records the current generated-code lint boundary.
