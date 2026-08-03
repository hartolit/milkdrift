# Milkdrift project architecture

This project selects **[Model B: Layered Workspace](../architecture.md#model-b-layered-workspace)** from the reusable architecture blueprint. This document specializes that model for Milkdrift. Accepted [ADRs](../agent/decisions/README.md) record decision rationale; [workspace boundaries](workspace.md) owns the exact crate inventory and dependency edges.

## Physical layout and logical roles

The root `Cargo.toml` is a virtual workspace manifest, not a package. `.cargo/config.toml` provides the workspace-local `cargo xtask` alias, and `tools/xtask` is the sole registered custom tooling member. Product code remains in five responsibility-based categories. The root-workspace member `benchmarks/runtime`, whose package name is `runtime-benchmarks`, is separately classified as a non-production measurement observer:

```text
.cargo/             workspace-local Cargo configuration
tools/xtask/        architecture, hygiene, and composite verification tooling
benchmarks/runtime/ cross-crate E0/E1 baseline and component measurement observer
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

`runtime-benchmarks` sits outside the product graph and depends inward on exact reviewed public production APIs. Its workspace-local normal edges are exactly `application-runtime`, `candle-backend`, `domain-contracts`, `host-runtime`, and `inference-runtime`; its external normal edges are exactly `serde`, `serde_json`, and `sha2` 0.11; and its sole development edge is external `criterion`. These remain observer edges rather than production-composition edges despite Cargo's `normal` classification. No production, tooling, test, or application package may depend on `runtime-benchmarks` through any dependency kind.

The package is an exact root member, uses the committed root `Cargo.lock` and shared root `target`, declares `publish = false`, and has no build script, Cargo custom-build target, or build dependencies. Directory placement alone does not authorize another benchmark package, and benchmark helpers do not become public product APIs merely to ease measurement.

## Domain tiers

`domain-contracts` is the F0 shared foundation. F0 inclusion requires either a backend/runtime crossing or at least two stable, distinct domain consumers. This keeps single-feature vocabulary with its owning algorithm; for example, `TaskId` belongs to `task-graph`, not `domain-contracts`.

`tokenization`, `context-planner`, `sampling`, and `task-graph` are F1 algorithm crates. The validator registers every domain-to-domain production edge exactly, requires a nonempty review rationale, and verifies that the complete reviewed graph is acyclic. The current graph contains only `tokenization → domain-contracts`, `context-planner → domain-contracts`, `sampling → domain-contracts`, and `task-graph → domain-contracts`. There is no current F1 peer edge, but F1 → F1 is not universally forbidden: a future peer edge requires an exact review and must preserve the DAG. Every unreviewed domain peer fails closed.

Portable domain code does not import runtimes, applications, platform implementations, vendor libraries, frontend toolkits, or filesystem/network/database/OS transport implementations. Portability claims are scoped in [portability](portability.md).

## E0: local inference ownership

E0 `inference-runtime` exclusively owns loaded model generations, backend sequences, request admission, generation workspaces, sampling execution, cancellation boundaries, draining, cleanup quarantine, accounting, unload, and shutdown. Its contracts describe direct ownership of model resources and token-step scheduling. E0 verifies complete loaded descriptors, the actual execution-device identity and accepted resident footprint, advertised limits and capabilities, sequence identity/state, position transitions, and exact vocabulary logits rather than trusting adapter claims after trait conformance. [ADR-0010](../agent/decisions/0010-verify-backend-contracts-at-e0.md) records the general substitution rule; [ADR-0019](../agent/decisions/0019-explicit-cuda-execution-foundation.md) applies it to devices and CUDA accounting.

Production E0 is instantiated once with `CandleLlamaSource`. Token-sensitive execution stays statically dispatched; E0 remains generic and backend-neutral at its project-owned contracts so deterministic test loaders can exercise lifecycle and failure semantics without adding another production engine.

A hosted model API or another machine is not an E0 backend merely because it can produce text. Remote execution has different ownership, cancellation, accounting, and capability semantics and belongs behind a coarser execution boundary above E0.

## Capability engines

A capability engine owns independently stateful reusable behavior whose lifecycle or reason to change is distinct from the application façade. `corrective-workflow` is the current example: it owns workflow artifacts, attempts, retries, validation state, bounded output production, explicit artifact release, and events without owning the application or local inference lifecycle. Its model and validator ports write into engine-owned bounded sinks; see [ADR-0011](../agent/decisions/0011-bound-workflow-output-at-the-port.md).

Capability engines are created only from evidence. Memory orchestration, peer routing, or another subsystem should not become an engine until state, lifecycle, reuse, replacement, or testing pressure gives it a coherent boundary. Capability engines do not depend on one another by default; E1 coordinates separate capabilities unless a lower dependency is explicitly justified.

## E1: application semantics and concrete local composition

E1 `application-runtime` is the frontend-neutral application façade and current local composition root. It owns application-level model and execution-device selection, bounded device discovery, lifecycle, immutable resolution, generation, conversation, prompt/text, normalized state/events, persisted preferences/catalogue state, accelerator-memory policy, and explicit shutdown semantics shared by every frontend.

`ModelSelection` contains only a normalized Hugging Face repository and requested revision. Resolution is device-independent: `ResolvedModel` reports artifacts, source, Safetensors format, validated scalar, tokenizer, immutable identity, and Llama/Candle compatibility evidence, but no selected or actual device. E1 stores `ApplicationDevice::{Cpu, Cuda { ordinal: u32 }}` separately; `LoadedModel` reports only the actual device verified from E0's receipt.

CPU always exists and is the fresh-install default. Initial bounded discovery probes CUDA 0 and, when different, the persisted selected CUDA ordinal. Application-owned summaries retain structured unavailability; persisted unavailable CUDA remains selected, visible, and load-blocking rather than falling back. Selection changes only when E1's lifecycle reports `can_select_device`. No Candle or `cudarc` type crosses the public E1 boundary. Concrete local wiring stays behind private `local.rs`, with one `HostedRuntime<CandleLlamaSource>` and one inference thread. The Hub worker, `HfTokenizer`, request-local `HfOwnedStreamingDecoder`, and redb storage remain private composition details. Startup is transactional across worker creation: if Hub-worker startup fails after inference startup, E1 requests bounded inference shutdown and joins the started worker before returning the Hub failure.

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
        Safetensors + selected CPU / feature-gated CUDA execution
```

Execution engine, model format, artifact source, and device are separate concepts rather than interchangeable caller-assembled axes. The reviewed E1 composition is Candle plus immutable Hugging Face Safetensors on explicit CPU or feature-gated Linux CUDA. CPU remains mandatory and default. E1 passes the exact selected domain `ExecutionDevice`; E0 reports actual device and footprint, and E1 verifies both before publishing loaded state. The complete opt-in feature graph is `desktop-slint/cuda -> application-runtime/cuda -> candle-backend/cuda`; the separate `inference-runtime/cuda` forwarding edge is development-only. No default graph reaches CUDA. GGUF is unsupported; if pursued later, Candle-native GGUF or another quantized format belongs under the Candle execution path and requires separate compatibility, tokenizer-provenance, artifact-identity, quantization, and device evidence.

## Model execution boundary

The implemented E1 selection covers local CPU and feature-gated CUDA execution through E0. Unavailable explicit CUDA fails without CPU fallback. Hosted providers, peer nodes, and remote transport are not product paths. If a remote target is implemented later, the common boundary is coarse: target identity and capabilities, complete request admission, cancellation intent, bounded streamed output, usage, and terminal state. Local execution adapts that boundary to E0; peer and hosted implementations translate it to their transports.

Uniformity must not hide real differences. Context limits, token accounting, prompt/message formats, sampling controls, tool support, privacy boundary, cancellation guarantees, and usage reporting are target capabilities. Unsupported behavior fails explicitly. This direction is recorded in [ADR-0008](../agent/decisions/0008-capability-and-execution-boundaries.md).

## Platform and adapters

`host-runtime` quarantines bounded channels, named threads, monotonic time, and synchronization/storage for pull-oriented output accumulators. It is infrastructure below runtime orchestration and owns no model, workflow, conversation, or application state.

Adapters own vendor, model, persistence, network, filesystem, and external-service integration details. They do not depend on runtimes or applications, and production adapters do not depend on one another. The current local path composes `candle-backend`, `hf-hub-adapter`, `hf-tokenizer`, and `redb-storage` in E1.

## Frontend and deployment boundary

`desktop-slint` owns the native event loop, presentation, platform path selection, and UI command mapping. Its compact device selector uses a Rust-owned `ApplicationDevice` identity/index model, never parses labels, and derives selection/load enabled state from E1. It distinguishes selected-device, artifact-only resolved-model, and receipt-verified actual loaded-device summaries. It does not construct backend sources, choose fallback policy, or own model tensors, token scheduling, persistence, Hub integration, or inference lifecycle policy.

A native Slint, Tauri, TUI/CLI, headless node, or similar process can host or call E1 directly. A browser frontend requires an explicit transport to a native or remote host. The frontend presents state and pulls bounded output; it does not issue one inference command per generated token. Local scheduling lives beside model execution as recorded in [ADR-0003](../agent/decisions/0003-generation-scheduling-ownership.md).

## Lifecycle and resource policy

Model and sequence values are exclusively owned by E0 rather than shared through public `Arc<Model>`-style ownership. Public handles carry identity and generation safety, not ownership of model state.

Admission validates capacities and accounting before state becomes visible. E1 re-probes the selected device, passes its exact `ExecutionDevice`, and validates the receipt ticket, logical model identity/handle, immutable resolution/artifacts, scalar, Llama/Candle/Safetensors evidence, tokenizer vocabulary, selected versus actual device, and bounded footprint. Cleanup failure does not imply release: a mismatch publishes no `LoadedModel`, and unresolved E0 resources remain in the existing private incompatible-load quarantine, owned and accounted through retry and exhaustion. A failed load that reports retained E0 cleanup keeps E1 unloading and device selection locked; a bounded private snapshot returns it to idle only after zero aggregate ownership is proven. Successful unload clears actual loaded-device state but preserves application selection.

Accelerator policy is explicit `Automatic` or a nonzero limit. E0's aggregate device budget is fixed at startup, so `Automatic` uses the least reported physical total across every CUDA row in the bounded startup catalogue; unavailable or capacity-unknown rows contribute zero and fail closed, while a limit applies a lower user cap. Load re-probes require the fixed nonzero budget to remain within the selected device's latest physical total, otherwise loading is blocked without fallback until restart. CPU host budgeting is unchanged, and selected-device Candle planning checks current available VRAM before partial residency. Host RAM is not used to infer CUDA capacity, and no undocumented `u64::MAX` shortcut is used. `LAS1` settings version 2 persists the selected device and policy while exact version 1 remains readable as CPU plus its legacy zero/limit mapping; `LAM1` model records remain version 1. Detailed behavior belongs in [inference runtime](inference-runtime.md), [model lifecycle](lifecycle.md), and [application runtime](application-runtime.md).

Explicit bounded shutdown is required for normal operation; blocking `Drop` is not the primary protocol. E1 distinguishes running, stopping, cleanly stopped, retryable failure, and terminal failure. A timeout leaves unfinished worker handles owned by E1 so a later shutdown call can retry and may complete cleanly. In contrast, E0 cleanup exhaustion is terminal: E0 publishes the structured failure, deliberately retains the runtime allocation until process exit rather than invoking unverified backend destruction, and terminates. E1 retains that failure independently from the join handle and never infers clean success from handle absence. See [ADR-0006](../agent/decisions/0006-explicit-bounded-shutdown.md).

## Current product constraints

- Candle is the sole local execution engine.
- Immutable Hugging Face Hub Safetensors execute on explicit CPU or opt-in Linux CUDA; CPU is mandatory and the fresh-install/default-build selection.
- E1 exposes one selected/resident local model.
- Accelerator policy is explicit `Automatic` or a nonzero limit resolved against the least startup-catalogue CUDA capacity, with unknown capacity and later incompatible capacity changes failing closed; CPU host-budget behavior is unchanged, and selected-device planning checks current available VRAM.
- Direct completion is available for every loaded model.
- Chat/history rendering is enabled only for the exact verified TinyLlama Chat v1 profile.
- GGUF, Metal, generic GPU aliases, automatic CPU fallback, hosted-provider, peer, browser-transport, and `application-api` paths are not implemented.
- External/product-model CUDA evidence and measurements remain outstanding; Phase 11 is active and incomplete.

The authoritative integration and validation matrix is in [implementation status](implementation-status.md).

## Enforcement

`cargo xtask architecture` loads typed locked Cargo metadata, fails closed on unknown workspace locations and unresolved local path targets, distinguishes dependency kinds, and enforces the logical direction F0/F1 → platform/adapters → E0/capabilities → E1 → applications. `tools/xtask` is the sole tooling package, and `benchmarks/runtime` is the sole recognized benchmark role outside those product layers.

`inference-runtime`, `corrective-workflow`, and `application-runtime` are the recognized E0, capability, and E1 packages; `host-runtime` is the only recognized platform package. Domain production dependencies are exact entries in the reviewed acyclic domain graph. Runtime production dependencies on adapters/platform or other runtimes likewise require exact reviewed source/target/kind entries. `cargo xtask hygiene` is the independent repository-hygiene check; `cargo xtask verify` runs both policies before the ordinary Cargo gates. [Dependency policy](dependency-policy.md) owns the review rules and hygiene boundary.

Project-authored source denies unsafe code. Generated-code exceptions are narrow and contained; [workspace boundaries](workspace.md) records the current Slint generated-code lint boundary.
