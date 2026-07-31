# llm-app project architecture

This project selects **[Model B: Layered Workspace](../architecture.md#model-b-layered-workspace)** from the reusable architecture blueprint. This document specializes that model for llm-app. Accepted [ADRs](../agent/decisions/README.md) record decision rationale; [workspace boundaries](workspace.md) owns the exact crate inventory and dependency edges.

## Physical layout and logical roles

The repository keeps five physical categories:

```text
crates/domain/      portable contracts and algorithms
crates/platform/    process-host execution primitives
crates/adapters/    external, vendor, model, and persistence integrations
crates/runtime/     stateful orchestration and resource ownership
crates/apps/        process, event-loop, and presentation boundaries
```

Runtime crates have distinct logical roles:

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

E1 may coordinate capability engines and E0. Its current private local composition owns one monomorphized Candle E0 worker/thread, one bounded Hub resolver worker, one concrete Hugging Face tokenizer path, and request-local Hugging Face streaming decoders. A capability engine may use E0, platform services, adapters, and domain code when its own lifecycle requires them. Neither capability engines nor E0 depend on E1. Applications depend on E1 rather than reconstructing application state machines.

## Domain tiers

`domain-contracts` is the F0 shared foundation. It owns vocabulary that genuinely crosses backend/runtime or multiple-domain boundaries: typed identities, capacities, model/sequence contracts, lifecycle transitions, and output records.

`tokenization`, `context-planner`, `sampling`, and `task-graph` are F1 algorithm crates. The currently enforced production policy permits F1 → F0 and rejects F1 → F1. This is a project constraint rather than a universal Rust rule; do not push unrelated vocabulary into F0 merely to evade the graph. Phase 9 may replace that restriction only with an explicitly reviewed acyclic graph.

Portable domain code does not import runtimes, applications, platform implementations, vendor libraries, frontend toolkits, or filesystem/network/database/OS transport implementations. Portability claims are scoped in [portability](portability.md).

## E0: local inference ownership

E0 `inference-runtime` exclusively owns loaded model generations, backend sequences, request admission, generation workspaces, sampling execution, cancellation boundaries, draining, cleanup quarantine, accounting, unload, and shutdown. Its contracts describe direct ownership of model resources and token-step scheduling. E0 verifies complete loaded descriptors, advertised limits and capabilities, sequence identity/state, position transitions, and exact vocabulary logits rather than trusting adapter claims after trait conformance. [ADR-0010](../agent/decisions/0010-verify-backend-contracts-at-e0.md) records this substitution rule.

Production E0 is instantiated once with `CandleLlamaSource`. Token-sensitive execution stays statically dispatched; E0 remains generic and backend-neutral at its project-owned contracts so deterministic test loaders can exercise lifecycle and failure semantics without adding another production engine.

A hosted model API or another machine is not an E0 backend merely because it can produce text. Remote execution has different ownership, cancellation, accounting, and capability semantics and belongs behind a coarser execution boundary above E0.

## Capability engines

A capability engine owns independently stateful reusable behavior whose lifecycle or reason to change is distinct from the application façade. `corrective-workflow` is the current example: it owns workflow artifacts, attempts, retries, validation state, bounded output production, explicit artifact release, and events without owning the application or local inference lifecycle. Its model and validator ports write into engine-owned bounded sinks; see [ADR-0011](../agent/decisions/0011-bound-workflow-output-at-the-port.md).

Capability engines are created only from evidence. Memory orchestration, peer routing, or another subsystem should not become an engine until state, lifecycle, reuse, replacement, or testing pressure gives it a coherent boundary. Capability engines do not depend on one another by default; E1 coordinates separate capabilities unless a lower dependency is explicitly justified.

## E1: application semantics and concrete local composition

E1 `application-runtime` is the frontend-neutral application façade and current local composition root. It owns application-level model selection and lifecycle, immutable resolution, generation, conversation, prompt/text, normalized state/events, persisted preferences/catalogue state, and explicit shutdown semantics shared by every frontend.

`ModelSelection` contains a normalized Hugging Face repository and requested revision. Resolution produces immutable Hub artifacts and derives the current execution facts:

- engine: Candle;
- artifact source: Hugging Face Hub;
- device: CPU;
- format: Safetensors;
- supported scalar: F32, F16, or BF16 as validated;
- immutable identity: repository plus resolved Hub commit.

Callers cannot construct arbitrary engine/source/device/format combinations, and Candle types do not cross the public E1 boundary. Concrete local wiring stays behind private `local.rs`, with one `HostedRuntime<CandleLlamaSource>` and one inference thread. The Hub worker, `HfTokenizer`, request-local `HfOwnedStreamingDecoder`, and redb storage remain private composition details.

[ADR-0013](../agent/decisions/0013-candle-only-local-execution.md) supersedes the former two-worker composition while retaining a non-generic E1 façade, private concrete composition, and static token-sensitive execution. There is no `application-api`; a transport contract requires a real separate-process or browser consumer.

## Current local execution composition

```text
apps/desktop-slint
        ↓
application-runtime (E1)
        ├── Hub worker → hf-hub-adapter → immutable Hub artifacts
        ├── hf-tokenizer / request-local streaming decoder
        ├── redb-storage
        └── one hosted inference worker
                    ↓
             inference-runtime (E0)
                    ↓
             candle-backend
                    ↓
        Safetensors + CPU execution
```

Execution engine, model format, artifact source, and device are separate concepts. Current support is one reviewed combination, not a claim that the dimensions are interchangeable today. GGUF is unsupported. If pursued later, Candle-native GGUF or another quantized format belongs under the Candle execution path and requires separate compatibility, tokenizer-provenance, artifact-identity, quantization, and device evidence. GPU support is also deferred.

## Model execution boundary

The implemented selection covers only local CPU execution through E0. Hosted providers, peer nodes, remote transport, and GPU execution are not product paths. If a remote target is implemented later, the common boundary is coarse: target identity and capabilities, complete request admission, cancellation intent, bounded streamed output, usage, and terminal state. Local execution adapts that boundary to E0; peer and hosted implementations translate it to their transports.

Uniformity must not hide real differences. Context limits, token accounting, prompt/message formats, sampling controls, tool support, privacy boundary, cancellation guarantees, and usage reporting are target capabilities. Unsupported behavior fails explicitly. This direction is recorded in [ADR-0008](../agent/decisions/0008-capability-and-execution-boundaries.md).

## Platform and adapters

`host-runtime` quarantines bounded channels, named threads, monotonic time, and synchronization/storage for pull-oriented output accumulators. It is infrastructure below runtime orchestration and owns no model, workflow, conversation, or application state.

Adapters own vendor, model, persistence, network, filesystem, and external-service integration details. They do not depend on runtimes or applications, and production adapters do not depend on one another. The current local path composes `candle-backend`, `hf-hub-adapter`, `hf-tokenizer`, and `redb-storage` in E1.

## Frontend and deployment boundary

`desktop-slint` owns the native event loop, presentation, platform path selection, and UI command mapping. It maps only E1 selection, state, event, and metadata types; it does not construct backend sources or own model tensors, token scheduling, persistence, Hub integration, or inference lifecycle policy.

A native Slint, Tauri, TUI/CLI, headless node, or similar process can host or call E1 directly. A browser frontend requires an explicit transport to a native or remote host. The frontend presents state and pulls bounded output; it does not issue one inference command per generated token. Local scheduling lives beside model execution as recorded in [ADR-0003](../agent/decisions/0003-generation-scheduling-ownership.md).

## Lifecycle and resource policy

Model and sequence values are exclusively owned by E0 rather than shared through public `Arc<Model>`-style ownership. Public handles carry identity and generation safety, not ownership of model state.

Admission validates capacities and accounting before state becomes visible. Cleanup failure does not imply release: unresolved resources remain quarantined and accounted until explicit cleanup succeeds or its bounded retry policy is exhausted. Detailed behavior belongs in [inference runtime](inference-runtime.md) and [model lifecycle](lifecycle.md).

Explicit bounded shutdown is required for normal operation; blocking `Drop` is not the primary protocol. E1 requests Hub shutdown, sends one ticketed E0 shutdown command, and attempts bounded joins for the sole inference worker and the Hub worker. See [ADR-0006](../agent/decisions/0006-explicit-bounded-shutdown.md).

## Current product constraints

- Candle is the sole local execution engine.
- Immutable Hugging Face Hub Safetensors on CPU is the only supported source/format/device composition.
- E1 exposes one selected/resident local model.
- Direct completion is available for every loaded model.
- Chat/history rendering is enabled only for the exact verified TinyLlama Chat v1 profile.
- GGUF, GPU, hosted-provider, peer, browser-transport, and `application-api` paths are not implemented.

The authoritative integration and validation matrix is in [implementation status](implementation-status.md).

## Enforcement

The architecture validator loads typed locked Cargo metadata, fails closed on unknown workspace locations and unresolved local path targets, distinguishes dependency kinds, and enforces the logical direction F0/F1 → platform/adapters → E0/capabilities → E1 → applications.

`inference-runtime`, `corrective-workflow`, and `application-runtime` are the recognized E0, capability, and E1 packages; `host-runtime` is the only recognized platform package. Runtime production dependencies on adapters/platform or other runtimes require exact reviewed source/target/kind entries. [Dependency policy](dependency-policy.md) owns those review rules and the Rust-owned repository hygiene gate.

Project-authored source denies unsafe code. Generated-code exceptions are narrow and contained; [workspace boundaries](workspace.md) records the current Slint generated-code lint boundary.
